//! Geneva exporter stress test using the generic stream throughput tester
//!
//! Run with: cargo run --bin geneva_stream_stress --release -- [concurrency]

use geneva_uploader::{AuthMethod, GenevaClient, GenevaClientConfig};
use opentelemetry_proto::tonic::common::v1::any_value::Value;
use opentelemetry_proto::tonic::common::v1::{AnyValue, KeyValue};
use opentelemetry_proto::tonic::logs::v1::{LogRecord, ResourceLogs, ScopeLogs};
use std::sync::Arc;

// Import the generic stream throughput test module
mod async_throughput;
use async_throughput::{StreamThroughputConfig, StreamThroughputTest};

// Import mock server setup if needed
use base64::{engine::general_purpose, Engine as _};
use chrono::{Duration, Utc};
use wiremock::matchers::{method, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

// Helper functions
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

fn generate_mock_jwt_and_expiry(endpoint: &str, ttl_secs: i64) -> (String, String) {
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

        let client = GenevaClient::new(config).await?;
        Ok((client, None))
    } else if let Ok(mock_endpoint) = std::env::var("MOCK_SERVER_URL") {
        println!("Using standalone mock server at: {}", mock_endpoint);

        let config = GenevaClientConfig {
            endpoint: mock_endpoint,
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

        let client = GenevaClient::new(config).await?;
        Ok((client, None))
    } else {
        println!("Using wiremock Geneva endpoints");

        // Setup mock server
        let mock_server = MockServer::start().await;
        let ingestion_endpoint = format!("{}/ingestion", mock_server.uri());
        let (auth_token, auth_token_expiry) =
            generate_mock_jwt_and_expiry(&ingestion_endpoint, 24 * 3600);

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

        let client = GenevaClient::new(config).await?;
        Ok((client, Some(mock_server.uri())))
    }
}

//#[tokio::main(flavor = "current_thread")]
#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let concurrency = args
        .get(1)
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(100);

    // Initialize client and test data
    let (client, _mock_uri) = init_client().await?;
    let client = Arc::new(client);
    let logs = Arc::new(create_test_logs());

    // Warm up the ingestion token cache
    println!("Warming up token cache...");
    client.upload_logs(&logs).await?;

    println!("\nStarting Geneva exporter stress test using stream-based approach");
    println!("Press Ctrl+C to stop continuous tests\n");

    // Test mode based on second argument
    let mode = args.get(2).map(|s| s.as_str()).unwrap_or("comparison");

    match mode {
        "continuous" => {
            // Run continuous test
            let config = StreamThroughputConfig {
                concurrency,
                report_interval: std::time::Duration::from_secs(5),
                target_ops: None,
            };

            let client_clone = client.clone();
            let logs_clone = logs.clone();

            let stats = StreamThroughputTest::run_continuous("Geneva Upload", config, move || {
                let client = client_clone.clone();
                let logs = logs_clone.clone();
                async move {
                    client
                        .upload_logs(&logs)
                        .await
                        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
                }
            })
            .await;

            stats.print("Final Results");
        }
        "fixed" => {
            // Run fixed test
            let target = args
                .get(3)
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(10_000);

            let config = StreamThroughputConfig {
                concurrency,
                target_ops: Some(target),
                ..Default::default()
            };

            let client_clone = client.clone();
            let logs_clone = logs.clone();

            let stats = StreamThroughputTest::run_fixed("Geneva Upload", config, move || {
                let client = client_clone.clone();
                let logs = logs_clone.clone();
                async move {
                    client
                        .upload_logs(&logs)
                        .await
                        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
                }
            })
            .await;

            stats.print("Final Results");
        }
        _ => {
            // Default: Run comparison test
            let concurrency_levels = vec![5, 10, 15, 30, 50, 100, 200, 500, 750, 1000];
            let target_ops = 10_000;

            let results = StreamThroughputTest::run_comparison(
                "Geneva Upload",
                &concurrency_levels,
                target_ops,
                move || {
                    let client = client.clone();
                    let logs = logs.clone();
                    async move {
                        client
                            .upload_logs(&logs)
                            .await
                            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
                    }
                },
            )
            .await;

            // Print summary
            println!("\nSummary:");
            println!("Concurrency | Throughput (ops/sec)");
            println!("----------- | -------------------");
            for (concurrency, stats) in results {
                println!("{:11} | {:19.2}", concurrency, stats.throughput);
            }
        }
    }

    Ok(())
}
