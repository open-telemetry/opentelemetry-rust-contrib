//! Run this stress test using `$ cargo run --bin geneva_exporter --release -- <num-of-threads>`.
//!
//! IMPORTANT:
//!     To test with real endpoints, set GENEVA_ENDPOINT environment variable:
//!         export GENEVA_ENDPOINT=https://your-geneva-endpoint.com
//!     To test with mocked endpoints (default), no configuration needed.
//!
//! Hardware: MacBook Pro (Apple M4 Pro, 14 cores, 24 GB RAM)
//! Stress Test Results (wiremock, MockAuth):
//! Threads: 1  - Avg Throughput: ~13,100 iterations/sec
//! Threads: 5  - Avg Throughput: ~50,400 iterations/sec
//! Threads: 10 - Avg Throughput: ~60,500 iterations/sec
//! Threads: 14 - Avg Throughput: ~61,000 iterations/sec

use base64::{engine::general_purpose, Engine as _};
use chrono::{Duration, Utc};
use opentelemetry_proto::tonic::common::v1::any_value::Value;
use opentelemetry_proto::tonic::common::v1::{AnyValue, KeyValue};
use opentelemetry_proto::tonic::logs::v1::{LogRecord, ResourceLogs, ScopeLogs};
use std::sync::Arc;
use wiremock::matchers::{method, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

use geneva_uploader::{AuthMethod, GenevaClient, GenevaClientConfig};

mod async_throughput;
use async_throughput::AsyncThroughputTest;

// Pre-generated test data to avoid allocation overhead
fn create_test_logs() -> Vec<ResourceLogs> {
    let mut log_records = Vec::new();

    // Create 100 simple log records
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

async fn init_client() -> Result<(GenevaClient, Option<String>), Box<dyn std::error::Error>> {
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

        let client = GenevaClient::new(config)
            .await
            .expect("Failed to create client");

        Ok((client, None))
    } else {
        println!("Using mocked Geneva endpoints");

        // Setup mock server
        let mock_server = MockServer::start().await;
        let ingestion_endpoint = format!("{}/ingestion", mock_server.uri());
        println!(
            "Embedding ingestion_endpoint in JWT: {}",
            ingestion_endpoint
        );
        let (auth_token, auth_token_expiry) =
            generate_mock_jwt_and_expiry(&ingestion_endpoint, 24 * 3600);

        println!("MOCK JWT: {}", auth_token);

        // Mock config service (GET)
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string(format!(
                r#"{{
                    "IngestionGatewayInfo": {{
                        "Endpoint": "{}/ingestion",
                        "AuthToken": "{auth_token}",
                        "AuthTokenExpiryTime": "{auth_token_expiry}"
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

        // Mock ingestion service (POST)
        Mock::given(method("POST"))
            .and(path_regex(r"^/ingestion.*"))
            .respond_with({
                ResponseTemplate::new(202).set_body_string(r#"{"ticket": "accepted"}"#)
            })
            .mount(&mock_server)
            .await;

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

        let client = GenevaClient::new(config)
            .await
            .expect("Failed to create client");

        Ok((client, Some(mock_server.uri())))
    }
}

pub fn generate_mock_jwt_and_expiry(endpoint: &str, ttl_secs: i64) -> (String, String) {
    let header = r#"{"alg":"none","typ":"JWT"}"#;
    let expiry = Utc::now() + Duration::seconds(ttl_secs);

    let payload = format!(
        r#"{{"Endpoint":"{}","exp":{}}}"#,
        endpoint,
        expiry.timestamp()
    );

    // Encode without padding
    fn encode_no_pad(s: &str) -> String {
        let mut out = general_purpose::URL_SAFE_NO_PAD.encode(s);
        while out.ends_with('=') {
            out.pop();
        }
        out
    }

    let token = format!(
        "{}.{}.dummy",
        encode_no_pad(header),
        encode_no_pad(&payload)
    );

    (token, expiry.to_rfc3339())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();

    let workers = args
        .get(1)
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or_else(num_cpus::get);

    let concurrency = args
        .get(2)
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(100);

    // Initialize your resources
    let (client, _mock_uri) = init_client().await?;
    let client = Arc::new(client);
    let logs = Arc::new(create_test_logs());

    // Run the generic test
    let test = AsyncThroughputTest::new();
    use std::io;

    test.run::<_, _, io::Error>("Geneva Uploader", workers, concurrency, move || {
        let client = client.clone();
        let logs = logs.clone();
        async move {
            match client.upload_logs(&logs).await {
                Ok(val) => Ok(val),
                Err(e) => {
                    //eprintln!("upload_logs failed: {e}");
                    Err(io::Error::other(e))
                }
            }
        }
    })
    .await?;

    Ok(())
}
