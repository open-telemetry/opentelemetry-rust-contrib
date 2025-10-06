// Geneva exporter stress test using the generic stream throughput tester
//
// Run with: cargo run --bin geneva_stream_stress --release -- [multi|current] [concurrency] [fixed|continuous|comparison]

/*
   ## Hardware & OS:
    Hardware:
     - CPU: 16 logical processors (8 physical cores), AMD EPYC 7763 @ 2.45 GHz
     - L1/L2/L3 cache: 512 KB per core
     - RAM: 64 GB
     - OS: Ubuntu 6.6 (WSL2 on Windows 11), x86_64

     Using Tokio multi-thread runtime
    Using standalone mock server at: http://localhost:8080
    WARNING: Using MockAuth for GenevaConfigClient. This should only be used in tests!
    Warming up token cache...

    $ cargo run --bin geneva_exporter --release -- multi 500  continuous # 16 cores / workers
    Testing Geneva Upload with concurrency level: 500
        Progress: 288092 ops completed (288092 successful, 100.0%) in 5.00s = 57580.46 ops/sec
        Progress: 594529 ops completed (594529 successful, 100.0%) in 10.00s = 59427.39 ops/sec
        Progress: 905048 ops completed (905048 successful, 100.0%) in 15.01s = 60315.43 ops/sec
        Progress: 1215825 ops completed (1215825 successful, 100.0%) in 20.01s = 60772.70 ops/sec
        Progress: 1521028 ops completed (1521028 successful, 100.0%) in 25.01s = 60824.08 ops/sec

    $ cargo run --bin geneva_exporter --release -- current 500  continuous
    Testing Geneva Upload with concurrency level: 500
        Progress: 74247 ops completed (74247 successful, 100.0%) in 5.00s = 14845.95 ops/sec
        Progress: 151466 ops completed (151466 successful, 100.0%) in 10.00s = 15143.45 ops/sec
        Progress: 228674 ops completed (228674 successful, 100.0%) in 15.00s = 15241.91 ops/sec
        Progress: 299853 ops completed (299853 successful, 100.0%) in 20.00s = 14990.25 ops/sec
        Progress: 372788 ops completed (372788 successful, 100.0%) in 25.00s = 14909.05 ops/sec
        Progress: 449360 ops completed (449360 successful, 100.0%) in 30.01s = 14976.14 ops/sec

*/
use geneva_uploader::{AuthMethod, GenevaClient, GenevaClientConfig};
use opentelemetry_proto::tonic::common::v1::any_value::Value;
use opentelemetry_proto::tonic::common::v1::{AnyValue, KeyValue};
use opentelemetry_proto::tonic::logs::v1::{LogRecord, ResourceLogs, ScopeLogs};
use std::sync::Arc;

// Import the generic stream throughput test module
mod async_throughput;
use async_throughput::{ThroughputConfig, ThroughputTest};

// Import mock server setup if needed
use wiremock::matchers::{method, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

// Helper functions
fn create_test_logs(base_timestamp: u64) -> Vec<ResourceLogs> {
    let mut log_records = Vec::new();

    // Create 10 simple log records
    for i in 0..10 {
        let timestamp = base_timestamp + i * 1_000_000; // 1 ms apart
        let log = LogRecord {
            observed_time_unix_nano: timestamp,
            event_name: "Log".to_string(),
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

fn generate_mock_jwt_and_expiry(_endpoint: &str, _ttl_secs: i64) -> (String, String) {
    // This is the working token nginx config, updated with 1 year expiry
    // Expires: June 30, 2026 (timestamp: 1782936000)
    let token = "eyJhbGciOiJub25lIiwidHlwIjoiSldUIn0.eyJFbmRwb2ludCI6Imh0dHA6Ly9sb2NhbGhvc3Q6ODA4MC9pbmdlc3Rpb24iLCJleHAiOjE3ODI5MzYwMDB9.dummy";
    let expiry = "2026-06-30T12:00:00+00:00";

    (token.to_string(), expiry.to_string())
}

async fn init_client() -> Result<(GenevaClient, Option<String>), Box<dyn std::error::Error>> {
    // Check if we should use real endpoints
    if let Ok(endpoint) = std::env::var("GENEVA_ENDPOINT") {
        println!("Using real Geneva endpoint: {endpoint}");

        let config = GenevaClientConfig {
            endpoint,
            environment: std::env::var("GENEVA_ENV").unwrap_or_else(|_| "test".to_string()),
            account: std::env::var("GENEVA_ACCOUNT").unwrap_or_else(|_| "test".to_string()),
            namespace: std::env::var("GENEVA_NAMESPACE").unwrap_or_else(|_| "test".to_string()),
            region: std::env::var("GENEVA_REGION").unwrap_or_else(|_| "test".to_string()),
            config_major_version: std::env::var("GENEVA_CONFIG_MAJOR_VERSION")
                .ok()
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or(1),
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
            msi_resource: None,
        };

        let client = GenevaClient::new(config).map_err(std::io::Error::other)?;
        Ok((client, None))
    } else if let Ok(mock_endpoint) = std::env::var("MOCK_SERVER_URL") {
        println!("Using standalone mock server at: {mock_endpoint}");

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
            msi_resource: None,
        };

        let client = GenevaClient::new(config).map_err(std::io::Error::other)?;
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
            msi_resource: None,
        };

        let client = GenevaClient::new(config).map_err(std::io::Error::other)?;
        Ok((client, Some(mock_server.uri())))
    }
}

// Usage examples:
// # Use multi-thread runtime (default)
// cargo run --bin geneva_stream_stress --release -- 100
//
// # Use current-thread runtime
// cargo run --bin geneva_stream_stress --release -- current 100
//
// # Explicitly use multi-thread runtime
// cargo run --bin geneva_stream_stress --release -- multi 100
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();

    // Check if first argument is runtime type
    let (runtime_type, args_start_idx) = if args.len() > 1 {
        match args[1].as_str() {
            "current" => ("current", 2),
            "multi" => ("multi", 2),
            _ => ("multi", 1), // default to multi, first arg is not runtime type
        }
    } else {
        ("multi", 1)
    };

    // Build the appropriate runtime
    let runtime = match runtime_type {
        "current" => {
            println!("Using Tokio current-thread runtime");
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?
        }
        _ => {
            println!("Using Tokio multi-thread runtime");
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?
        }
    };

    // Run the async main function
    runtime.block_on(async_main(args, args_start_idx, runtime_type))
}

async fn async_main(
    args: Vec<String>,
    args_start_idx: usize,
    runtime_type: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    // Get timestamp for events
    let base_timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u64;

    // Get concurrency from the appropriate position
    let concurrency = args
        .get(args_start_idx)
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(100);

    // Get mode from the next position
    let mode = args
        .get(args_start_idx + 1)
        .map(|s| s.as_str())
        .unwrap_or("comparison");

    // Initialize client and test data
    let (client, _mock_uri) = init_client().await?;
    let client = Arc::new(client);
    let logs = Arc::new(create_test_logs(base_timestamp));

    // Warm up the ingestion token cache
    println!("Warming up token cache...");
    let warm_batches = client
        .encode_and_compress_logs(&logs)
        .map_err(|e| format!("Failed to encode logs: {e}"))?;
    for batch in &warm_batches {
        client
            .upload_batch(batch)
            .await
            .map_err(|e| format!("Failed to upload batch: {e}"))?;
    }

    println!("\nStarting Geneva exporter stress test using stream-based approach");
    println!("Press Ctrl+C to stop continuous tests\n");

    match mode {
        "continuous" => {
            // Run continuous test
            let config = ThroughputConfig {
                concurrency,
                report_interval: std::time::Duration::from_secs(5),
                target_ops: None,
                use_spawn: runtime_type != "current", // Use task spawning for multi-thread runtime
            };

            ThroughputTest::run_continuous("Geneva Upload", config, move || {
                let client = client.clone();
                let logs = logs.clone();
                async move {
                    let batches = client.encode_and_compress_logs(&logs)?;

                    // Upload batches sequentially TODO - use buffer_unordered for concurrency
                    for batch in &batches {
                        client
                            .upload_batch(batch)
                            .await
                            .map_err(|e| format!("Failed to upload batch: {e}"))?;
                    }
                    Ok::<(), String>(())
                }
            })
            .await;
        }
        "fixed" => {
            // Run fixed test
            let target = args
                .get(3)
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(10_000);

            let config = ThroughputConfig {
                concurrency,
                target_ops: Some(target),
                use_spawn: runtime_type != "current", // Use task spawning for multi-thread runtime
                ..Default::default()
            };

            let stats = ThroughputTest::run_fixed("Geneva Upload", config, move || {
                let client = client.clone();
                let logs = logs.clone();
                async move {
                    let batches = client
                        .encode_and_compress_logs(&logs)
                        .map_err(|e| format!("Failed to encode logs: {e}"))?;

                    // Upload batches sequentially - TODO - use buffer_unordered for concurrency
                    for batch in &batches {
                        client
                            .upload_batch(batch)
                            .await
                            .map_err(|e| format!("Failed to upload batch: {e}"))?;
                    }
                    Ok::<(), String>(())
                }
            })
            .await;

            stats.print("Final Results");
        }
        _ => {
            // Default: Run comparison test
            let concurrency_levels = vec![5, 10, 15, 30, 50, 100, 200, 500, 750, 1000];
            let target_ops = 10_000;

            let results = ThroughputTest::run_comparison(
                "Geneva Upload",
                &concurrency_levels,
                target_ops,
                move || {
                    let client = client.clone();
                    let logs = logs.clone();
                    async move {
                        let batches = match client.encode_and_compress_logs(&logs) {
                            Ok(batches) => batches,
                            Err(e) => return Err(format!("Failed to encode logs: {e}")),
                        };
                        for batch in &batches {
                            if let Err(e) = client.upload_batch(batch).await {
                                return Err(format!("Failed to upload batch: {e}"));
                            }
                        }
                        Ok(())
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
