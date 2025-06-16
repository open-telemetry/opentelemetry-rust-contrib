//! Run this stress test using `$ cargo run --bin geneva_exporter --release -- <num-of-threads>`.
//!
//! IMPORTANT:
//!     To test with real endpoints, set GENEVA_ENDPOINT environment variable:
//!         export GENEVA_ENDPOINT=https://your-geneva-endpoint.com
//!     To test with mocked endpoints (default), no configuration needed.
//!
//! Hardware: MacBook Pro (Apple M4 Pro, 14 cores, 24 GB RAM)
//! Stress Test Results (wiremock, MockAuth):
//! Threads: 1  - Avg Throughput: ~9,100 iterations/sec
//! Threads: 5  - Avg Throughput: ~31,400 iterations/sec
//! Threads: 10 - Avg Throughput: ~50,500 iterations/sec
//! Threads: 14 - Avg Throughput: ~58,000 iterations/sec

use opentelemetry_proto::tonic::common::v1::any_value::Value;
use opentelemetry_proto::tonic::common::v1::{AnyValue, KeyValue};
use opentelemetry_proto::tonic::logs::v1::{LogRecord, ResourceLogs, ScopeLogs};
use std::sync::Arc;
use tokio::runtime::Runtime;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use geneva_uploader::{AuthMethod, GenevaClient, GenevaClientConfig};

mod throughput;

// Pre-generated test data to avoid allocation overhead
fn create_test_logs() -> Vec<ResourceLogs> {
    let mut log_records = Vec::new();

    // Create 10 simple log records
    for i in 0..10 {
        let log = LogRecord {
            observed_time_unix_nano: 1700000000000000000 + i,
            event_name: "StressTestEvent".to_string(),
            severity_number: 9,
            severity_text: "INFO".to_string(),
            body: Some(AnyValue {
                value: Some(Value::StringValue("Stress test log message".to_string())),
            }),
            attributes: vec![
                KeyValue {
                    key: "test_id".to_string(),
                    value: Some(AnyValue {
                        value: Some(Value::StringValue("stress".to_string())),
                    }),
                },
                KeyValue {
                    key: "index".to_string(),
                    value: Some(AnyValue {
                        value: Some(Value::IntValue(i as i64)),
                    }),
                },
            ],
            ..Default::default()
        };
        log_records.push(log);
    }

    vec![ResourceLogs {
        scope_logs: vec![ScopeLogs {
            log_records,
            ..Default::default()
        }],
        ..Default::default()
    }]
}

// Function to initialize the Geneva client with mocked endpoints
fn init_client(runtime: &Runtime) -> (GenevaClient, Option<String>) {
    // Check if we should use real endpoints
    if let Ok(endpoint) = std::env::var("GENEVA_ENDPOINT") {
        println!("Using real Geneva endpoint: {}", endpoint);

        let config = GenevaClientConfig {
            endpoint,
            environment: std::env::var("GENEVA_ENV").unwrap_or_else(|_| "test".to_string()),
            account: std::env::var("GENEVA_ACCOUNT").unwrap_or_else(|_| "test".to_string()),
            namespace: std::env::var("GENEVA_NAMESPACE").unwrap_or_else(|_| "test".to_string()),
            region: std::env::var("GENEVA_REGION").unwrap_or_else(|_| "test".to_string()),
            config_major_version: 1,
            auth_method: AuthMethod::Certificate {
                path: std::path::PathBuf::from(
                    std::env::var("GENEVA_CERT_PATH").unwrap_or_else(|_| "test.p12".to_string()),
                ),
                password: std::env::var("GENEVA_CERT_PASSWORD")
                    .unwrap_or_else(|_| "test".to_string()),
            },
            tenant: std::env::var("GENEVA_TENANT").unwrap_or_else(|_| "test".to_string()),
            role_name: std::env::var("GENEVA_ROLE").unwrap_or_else(|_| "test".to_string()),
            role_instance: std::env::var("GENEVA_INSTANCE").unwrap_or_else(|_| "test".to_string()),
        };

        let client = runtime.block_on(async {
            GenevaClient::new(config)
                .await
                .expect("Failed to create client")
        });

        (client, None)
    } else {
        println!("Using mocked Geneva endpoints");

        // Setup mock server
        let mock_server = runtime.block_on(async { MockServer::start().await });

        // Mock config service (GET)
        runtime.block_on(async {
            Mock::given(method("GET"))
                .respond_with(ResponseTemplate::new(200).set_body_string(format!(
                    r#"{{
                                "IngestionGatewayInfo": {{
                                    "Endpoint": "{}/ingestion",
                                    "AuthToken": "test-token",
                                    "AuthTokenExpiryTime": "2099-12-31T23:59:59Z"
                                }},
                                "StorageAccountKeys": [{{
                                    "AccountMonikerName": "testdiagaccount",
                                    "AccountGroupName": "testgroup",
                                    "IsPrimaryMoniker": true
                                }}],
                                "TagId": "test"
                            }}"#,
                    mock_server.uri()
                )))
                .mount(&mock_server)
                .await;
        });

        // Mock ingestion service (POST)
        runtime.block_on(async {
            Mock::given(method("POST"))
                .and(path("/ingestion"))
                .respond_with(
                    ResponseTemplate::new(202).set_body_string(r#"{"ticket": "accepted"}"#),
                )
                .mount(&mock_server)
                .await;
        });

        let config = GenevaClientConfig {
            endpoint: mock_server.uri(),
            environment: "test".to_string(),
            account: "test".to_string(),
            namespace: "test".to_string(),
            region: "test".to_string(),
            config_major_version: 1,
            auth_method: AuthMethod::MockAuth,
            tenant: "test".to_string(),
            role_name: "test".to_string(),
            role_instance: "test".to_string(),
        };

        let client = runtime.block_on(async {
            GenevaClient::new(config)
                .await
                .expect("Failed to create client")
        });

        (client, Some(mock_server.uri()))
    }
}

fn main() {
    // Initialize runtime
    let runtime = Arc::new(Runtime::new().expect("Failed to create runtime"));

    // Initialize the client and optionally the mock server
    let (client, _server_uri) = init_client(&runtime);
    let client = Arc::new(client);

    // Pre-generate test data
    let test_logs = Arc::new(create_test_logs());

    // Use the provided stress test framework
    println!("Starting stress test for Geneva Uploader...");
    throughput::test_throughput(move || {
        let client = client.clone();
        let logs = test_logs.clone();
        let runtime = runtime.clone();

        // Execute one upload operation
        runtime.block_on(async {
            // Ignore errors for throughput testing
            let _ = client.upload_logs(logs.to_vec()).await;
        });
    });
    println!("Stress test completed.");
}
