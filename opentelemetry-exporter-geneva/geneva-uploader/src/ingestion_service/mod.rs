pub(crate) mod uploader;

#[cfg(test)]
mod tests {
    use std::time::Instant;

    // WireMock tests for retry logic
    #[cfg(all(test, feature = "mock_auth"))]
    mod wiremock_tests {
        use crate::config_service::client::{
            AuthMethod, GenevaConfigClient, GenevaConfigClientConfig,
        };
        use crate::ingestion_service::uploader::{GenevaUploader, GenevaUploaderConfig};
        use crate::payload_encoder::central_blob::BatchMetadata;
        use crate::retry::RetryConfig;
        use std::sync::Arc;
        use std::time::Duration;
        use wiremock::matchers::{method, path_regex};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        // Helper to create a mock config service response
        fn mock_config_response() -> serde_json::Value {
            // Create a valid JWT token with an Endpoint claim
            // Header: {"alg":"HS256","typ":"JWT"}
            // Payload: {"Endpoint":"https://test.endpoint"}
            // This is a mock JWT token that will be replaced with the actual ingestion server URL
            let jwt_token = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJFbmRwb2ludCI6Imh0dHBzOi8vdGVzdC5lbmRwb2ludCJ9.signature";

            serde_json::json!({
                "IngestionGatewayInfo": {
                    "Endpoint": "PLACEHOLDER", // Will be replaced with mock server URL
                    "AuthToken": jwt_token,
                    "AuthTokenExpiryTime": "2030-01-01T00:00:00Z"
                },
                "StorageAccountKeys": [
                    {
                        "AccountMonikerName": "mock-diag-moniker",
                        "AccountGroupName": "mock-group",
                        "IsPrimaryMoniker": true
                    }
                ],
                "TagId": "mock-tag-id"
            })
        }

        #[tokio::test]
        async fn test_retry_on_transient_failure() {
            let config_server = MockServer::start().await;
            let ingestion_server = MockServer::start().await;

            // Create config response with ingestion server URL
            let mut config_response = mock_config_response();
            config_response["IngestionGatewayInfo"]["Endpoint"] =
                serde_json::Value::String(ingestion_server.uri());

            // Mock config service endpoint
            Mock::given(method("GET"))
                .and(path_regex("/api/agent/v3/.*/MonitoringStorageKeys/"))
                .respond_with(ResponseTemplate::new(200).set_body_json(&config_response))
                .mount(&config_server)
                .await;

            // Mock ingestion endpoint - fail twice, then succeed
            Mock::given(method("POST"))
                .and(path_regex("/api/v1/ingestion/ingest.*"))
                .respond_with(ResponseTemplate::new(500).set_body_string("Server error"))
                .up_to_n_times(2)
                .mount(&ingestion_server)
                .await;

            Mock::given(method("POST"))
                .and(path_regex("/api/v1/ingestion/ingest.*"))
                .respond_with(ResponseTemplate::new(202).set_body_json(serde_json::json!({
                    "ticket": "success-ticket-123"
                })))
                .mount(&ingestion_server)
                .await;

            // Create uploader with retry config
            let config = GenevaConfigClientConfig {
                endpoint: config_server.uri(),
                environment: "test".into(),
                account: "test".into(),
                namespace: "test".into(),
                region: "test".into(),
                config_major_version: 1,
                auth_method: AuthMethod::MockAuth,
            };

            let config_client = Arc::new(GenevaConfigClient::new(config).unwrap());

            let uploader_config = GenevaUploaderConfig {
                namespace: "test".into(),
                source_identity: "test".into(),
                environment: "test".into(),
                config_version: "Ver1v0".into(),
                retry_config: RetryConfig::new()
                    .with_max_retries(3)
                    .with_delay(Duration::from_millis(10)), // Short delay for testing
            };

            let uploader = GenevaUploader::from_config_client(config_client, uploader_config)
                .await
                .unwrap();

            let metadata = BatchMetadata {
                start_time: 1_700_000_000_000_000_000,
                end_time: 1_700_000_300_000_000_000,
                schema_ids: "test-schema".to_string(),
            };

            // Should retry twice and succeed on third attempt
            let result = uploader.upload(vec![1, 2, 3], "TestEvent", &metadata).await;
            assert!(result.is_ok());
            assert_eq!(result.unwrap().ticket, "success-ticket-123");
        }

        #[tokio::test]
        async fn test_no_retry_on_client_error() {
            let config_server = MockServer::start().await;
            let ingestion_server = MockServer::start().await;

            // Create config response with ingestion server URL
            let mut config_response = mock_config_response();
            config_response["IngestionGatewayInfo"]["Endpoint"] =
                serde_json::Value::String(ingestion_server.uri());

            // Mock config service endpoint
            Mock::given(method("GET"))
                .and(path_regex("/api/agent/v3/.*/MonitoringStorageKeys/"))
                .respond_with(ResponseTemplate::new(200).set_body_json(&config_response))
                .mount(&config_server)
                .await;

            // Mock ingestion endpoint - return 400 Bad Request
            Mock::given(method("POST"))
                .and(path_regex("/api/v1/ingestion/ingest.*"))
                .respond_with(ResponseTemplate::new(400).set_body_string("Bad request"))
                .expect(1) // Should only be called once (no retries)
                .mount(&ingestion_server)
                .await;

            // Create uploader with retry config
            let config = GenevaConfigClientConfig {
                endpoint: config_server.uri(),
                environment: "test".into(),
                account: "test".into(),
                namespace: "test".into(),
                region: "test".into(),
                config_major_version: 1,
                auth_method: AuthMethod::MockAuth,
            };

            let config_client = Arc::new(GenevaConfigClient::new(config).unwrap());

            let uploader_config = GenevaUploaderConfig {
                namespace: "test".into(),
                source_identity: "test".into(),
                environment: "test".into(),
                config_version: "Ver1v0".into(),
                retry_config: RetryConfig::new()
                    .with_max_retries(3)
                    .with_delay(Duration::from_millis(10)),
            };

            let uploader = GenevaUploader::from_config_client(config_client, uploader_config)
                .await
                .unwrap();

            let metadata = BatchMetadata {
                start_time: 1_700_000_000_000_000_000,
                end_time: 1_700_000_300_000_000_000,
                schema_ids: "test-schema".to_string(),
            };

            // Should fail immediately without retries
            let result = uploader.upload(vec![1, 2, 3], "TestEvent", &metadata).await;
            assert!(result.is_err());
        }

        #[tokio::test]
        async fn test_retry_on_rate_limiting() {
            let config_server = MockServer::start().await;
            let ingestion_server = MockServer::start().await;

            // Create config response with ingestion server URL
            let mut config_response = mock_config_response();
            config_response["IngestionGatewayInfo"]["Endpoint"] =
                serde_json::Value::String(ingestion_server.uri());

            // Mock config service endpoint
            Mock::given(method("GET"))
                .and(path_regex("/api/agent/v3/.*/MonitoringStorageKeys/"))
                .respond_with(ResponseTemplate::new(200).set_body_json(&config_response))
                .mount(&config_server)
                .await;

            // Mock ingestion endpoint - rate limit once, then succeed
            Mock::given(method("POST"))
                .and(path_regex("/api/v1/ingestion/ingest.*"))
                .respond_with(ResponseTemplate::new(429).set_body_string("Too many requests"))
                .up_to_n_times(1)
                .mount(&ingestion_server)
                .await;

            Mock::given(method("POST"))
                .and(path_regex("/api/v1/ingestion/ingest.*"))
                .respond_with(ResponseTemplate::new(202).set_body_json(serde_json::json!({
                    "ticket": "success-after-rate-limit"
                })))
                .mount(&ingestion_server)
                .await;

            // Create uploader with retry config
            let config = GenevaConfigClientConfig {
                endpoint: config_server.uri(),
                environment: "test".into(),
                account: "test".into(),
                namespace: "test".into(),
                region: "test".into(),
                config_major_version: 1,
                auth_method: AuthMethod::MockAuth,
            };

            let config_client = Arc::new(GenevaConfigClient::new(config).unwrap());

            let uploader_config = GenevaUploaderConfig {
                namespace: "test".into(),
                source_identity: "test".into(),
                environment: "test".into(),
                config_version: "Ver1v0".into(),
                retry_config: RetryConfig::new()
                    .with_max_retries(2)
                    .with_delay(Duration::from_millis(10)),
            };

            let uploader = GenevaUploader::from_config_client(config_client, uploader_config)
                .await
                .unwrap();

            let metadata = BatchMetadata {
                start_time: 1_700_000_000_000_000_000,
                end_time: 1_700_000_300_000_000_000,
                schema_ids: "test-schema".to_string(),
            };

            // Should retry on 429 and succeed
            let result = uploader.upload(vec![1, 2, 3], "TestEvent", &metadata).await;
            assert!(result.is_ok());
            assert_eq!(result.unwrap().ticket, "success-after-rate-limit");
        }

        #[tokio::test]
        async fn test_max_retries_exhausted() {
            let config_server = MockServer::start().await;
            let ingestion_server = MockServer::start().await;

            // Create config response with ingestion server URL
            let mut config_response = mock_config_response();
            config_response["IngestionGatewayInfo"]["Endpoint"] =
                serde_json::Value::String(ingestion_server.uri());

            // Mock config service endpoint
            Mock::given(method("GET"))
                .and(path_regex("/api/agent/v3/.*/MonitoringStorageKeys/"))
                .respond_with(ResponseTemplate::new(200).set_body_json(&config_response))
                .mount(&config_server)
                .await;

            // Mock ingestion endpoint - always fail with 503
            Mock::given(method("POST"))
                .and(path_regex("/api/v1/ingestion/ingest.*"))
                .respond_with(ResponseTemplate::new(503).set_body_string("Service unavailable"))
                .expect(3) // Initial attempt + 2 retries
                .mount(&ingestion_server)
                .await;

            // Create uploader with retry config
            let config = GenevaConfigClientConfig {
                endpoint: config_server.uri(),
                environment: "test".into(),
                account: "test".into(),
                namespace: "test".into(),
                region: "test".into(),
                config_major_version: 1,
                auth_method: AuthMethod::MockAuth,
            };

            let config_client = Arc::new(GenevaConfigClient::new(config).unwrap());

            let uploader_config = GenevaUploaderConfig {
                namespace: "test".into(),
                source_identity: "test".into(),
                environment: "test".into(),
                config_version: "Ver1v0".into(),
                retry_config: RetryConfig::new()
                    .with_max_retries(2)
                    .with_delay(Duration::from_millis(10)),
            };

            let uploader = GenevaUploader::from_config_client(config_client, uploader_config)
                .await
                .unwrap();

            let metadata = BatchMetadata {
                start_time: 1_700_000_000_000_000_000,
                end_time: 1_700_000_300_000_000_000,
                schema_ids: "test-schema".to_string(),
            };

            // Should fail after exhausting retries
            let result = uploader.upload(vec![1, 2, 3], "TestEvent", &metadata).await;
            assert!(result.is_err());
            let err_msg = format!("{:?}", result.unwrap_err());
            assert!(err_msg.contains("503") || err_msg.contains("Service unavailable"));
        }

        #[tokio::test]
        async fn test_zero_retries_configuration() {
            let config_server = MockServer::start().await;
            let ingestion_server = MockServer::start().await;

            // Create config response with ingestion server URL
            let mut config_response = mock_config_response();
            config_response["IngestionGatewayInfo"]["Endpoint"] =
                serde_json::Value::String(ingestion_server.uri());

            // Mock config service endpoint
            Mock::given(method("GET"))
                .and(path_regex("/api/agent/v3/.*/MonitoringStorageKeys/"))
                .respond_with(ResponseTemplate::new(200).set_body_json(&config_response))
                .mount(&config_server)
                .await;

            // Mock ingestion endpoint - fail with 500
            Mock::given(method("POST"))
                .and(path_regex("/api/v1/ingestion/ingest.*"))
                .respond_with(ResponseTemplate::new(500).set_body_string("Server error"))
                .expect(1) // Should only be called once (no retries)
                .mount(&ingestion_server)
                .await;

            // Create uploader with zero retries
            let config = GenevaConfigClientConfig {
                endpoint: config_server.uri(),
                environment: "test".into(),
                account: "test".into(),
                namespace: "test".into(),
                region: "test".into(),
                config_major_version: 1,
                auth_method: AuthMethod::MockAuth,
            };

            let config_client = Arc::new(GenevaConfigClient::new(config).unwrap());

            let uploader_config = GenevaUploaderConfig {
                namespace: "test".into(),
                source_identity: "test".into(),
                environment: "test".into(),
                config_version: "Ver1v0".into(),
                retry_config: RetryConfig::new()
                    .with_max_retries(0) // No retries
                    .with_delay(Duration::from_millis(10)),
            };

            let uploader = GenevaUploader::from_config_client(config_client, uploader_config)
                .await
                .unwrap();

            let metadata = BatchMetadata {
                start_time: 1_700_000_000_000_000_000,
                end_time: 1_700_000_300_000_000_000,
                schema_ids: "test-schema".to_string(),
            };

            // Should fail immediately without retries
            let result = uploader.upload(vec![1, 2, 3], "TestEvent", &metadata).await;
            assert!(result.is_err());
        }
    }

    mod test_helpers {
        use crate::{
            AuthMethod, GenevaConfigClient, GenevaConfigClientConfig, GenevaUploader,
            GenevaUploaderConfig,
        };
        use std::env;
        use std::fs;
        use std::sync::Arc;

        pub struct TestUploadContext {
            pub data: Vec<u8>,
            pub uploader: GenevaUploader,
            pub event_name: String,
        }

        pub async fn build_test_upload_context() -> TestUploadContext {
            // Load binary blob
            let blob_path =
                env::var("GENEVA_BLOB_PATH").expect("GENEVA_BLOB_PATH env var is required");
            let data = fs::read(&blob_path).expect("Failed to read binary blob");

            // Read config from environment
            let endpoint = env::var("GENEVA_ENDPOINT").expect("GENEVA_ENDPOINT is required");
            let environment =
                env::var("GENEVA_ENVIRONMENT").expect("GENEVA_ENVIRONMENT is required");
            let account = env::var("GENEVA_ACCOUNT").expect("GENEVA_ACCOUNT is required");
            let namespace = env::var("GENEVA_NAMESPACE").expect("GENEVA_NAMESPACE is required");
            let region = env::var("GENEVA_REGION").expect("GENEVA_REGION is required");
            let cert_path = std::path::PathBuf::from(
                std::env::var("GENEVA_CERT_PATH").expect("GENEVA_CERT_PATH is required"),
            );
            let cert_password = env::var("GENEVA_CERT_PASSWORD").unwrap_or_default();
            let config_major_version = env::var("GENEVA_CONFIG_MAJOR_VERSION")
                .expect("GENEVA_CONFIG_MAJOR_VERSION is required")
                .parse::<u32>()
                .expect("GENEVA_CONFIG_MAJOR_VERSION must be a u32");
            let source_identity = env::var("GENEVA_SOURCE_IDENTITY").unwrap_or_else(|_| {
                "Tenant=Default/Role=Uploader/RoleInstance=devhost".to_string()
            });

            // Define uploader config
            let config_version = format!("Ver{config_major_version}v0");
            let uploader_config = GenevaUploaderConfig {
                namespace: namespace.clone(),
                source_identity,
                environment: environment.clone(),
                config_version,
                retry_config: crate::RetryConfig::default(),
            };

            let config = GenevaConfigClientConfig {
                endpoint,
                environment,
                account,
                namespace,
                region,
                config_major_version,
                auth_method: AuthMethod::Certificate {
                    path: cert_path,
                    password: cert_password,
                },
            };

            // Build client and uploader
            let config_client =
                GenevaConfigClient::new(config).expect("Failed to create config client");
            let uploader =
                GenevaUploader::from_config_client(Arc::new(config_client), uploader_config)
                    .await
                    .expect("Failed to create uploader");

            // Event name/version
            let event_name = "Log".to_string();

            TestUploadContext {
                data,
                uploader,
                event_name,
            }
        }
    }

    #[tokio::test]
    /// To run this test against a real Geneva Config Service and GIG, set the following environment variables:
    ///
    /// ```bash
    /// export GENEVA_ENDPOINT="xxxhttps://<gcs-endpoint>"
    /// export GENEVA_ENVIRONMENT="Test"
    /// export GENEVA_ACCOUNT="YourAccountName"
    /// export GENEVA_NAMESPACE="YourNamespace"
    /// export GENEVA_REGION="YourRegion"
    /// export GENEVA_CONFIG_MAJOR_VERSION="2"
    /// export GENEVA_CERT_PATH="/path/to/client.p12"
    /// export GENEVA_CERT_PASSWORD="your-cert-password"
    /// export GENEVA_SOURCE_IDENTITY="Tenant=YourTenant/Role=YourRole/RoleInstance=YourInstance"
    /// export GENEVA_BLOB_PATH="/path/to/blob.bin"
    ///
    /// cargo test test_upload_to_gig_real_server -- --ignored --nocapture
    /// ```
    #[ignore]
    async fn test_upload_to_gig_real_server() {
        use crate::payload_encoder::central_blob::BatchMetadata;
        let ctx = test_helpers::build_test_upload_context().await;
        let blob_size = ctx.data.len();
        println!("✅ Loaded blob ({blob_size} bytes)");
        // below call is only for logging purposes, to get endpoint and auth info.
        let (auth_info, _, _) = ctx
            .uploader
            .config_client
            .get_ingestion_info()
            .await
            .unwrap();
        println!("🚀 Uploading to: {}", auth_info.endpoint);

        let start = Instant::now();

        // Create test metadata
        let metadata = BatchMetadata {
            start_time: 1_700_000_000_000_000_000,
            end_time: 1_700_000_300_000_000_000,
            schema_ids: "075bcd15e5b2ed60f26e66085ac2b2e8".to_string(), // Example MD5 hash
        };

        let response = ctx
            .uploader
            .upload(ctx.data, &ctx.event_name, &metadata)
            .await
            .expect("Upload failed");

        let elapsed = start.elapsed();
        println!(
            "✅ Upload complete in {elapsed:.2?}. Ticket: {}",
            response.ticket
        );
    }

    /// To run this test with parallel uploads:
    ///
    /// ```bash
    /// export GENEVA_ENDPOINT="https://<gcs-endpoint>"
    /// export GENEVA_ENVIRONMENT="Test"
    /// export GENEVA_ACCOUNT="YourAccount"
    /// export GENEVA_NAMESPACE="YourNamespace"
    /// export GENEVA_REGION="YourRegion"
    /// export GENEVA_CONFIG_MAJOR_VERSION="2"
    /// export GENEVA_CERT_PATH="/path/to/client.p12"
    /// export GENEVA_CERT_PASSWORD="your-password"
    /// export GENEVA_SOURCE_IDENTITY="Tenant=YourTenant/Role=Role/RoleInstance=Instance"
    /// export GENEVA_BLOB_PATH="/path/to/blob.bin"
    /// export GENEVA_PARALLEL_UPLOADS="10"
    ///
    /// cargo test test_parallel_uploads -- --ignored --nocapture
    /// Output:
    // 🔥 Performing warm-up upload...
    // 🔥 Warm-up upload complete in 222.42ms
    // 🚀 Launching 5 parallel uploads...
    // ✅ Upload 2 complete in 120.43ms. Ticket: ...
    // ✅ Upload 4 complete in 120.35ms. Ticket: ...
    // ✅ Upload 3 complete in 120.50ms. Ticket: ...
    // ✅ Upload 1 complete in 154.62ms. Ticket: ...
    // ✅ Upload 0 complete in 154.65ms. Ticket: ...
    // 📊 Average upload duration: 133.60 ms
    // ⏱️ Total elapsed for 5 parallel uploads: 154.93ms

    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn test_parallel_uploads() {
        use crate::payload_encoder::central_blob::BatchMetadata;
        use std::env;
        use std::time::Instant;

        // Read parallelism level from env
        // Use env variable if provided, else saturate all tokio threads by default (num_cpus::get())
        let parallel_uploads: usize = env::var("GENEVA_PARALLEL_UPLOADS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or_else(num_cpus::get);
        let ctx = test_helpers::build_test_upload_context().await;

        // --- Warm-up: do the first upload to populate the token cache ---
        println!("🔥 Performing warm-up upload...");
        let start_warmup = Instant::now();

        // Create test metadata for warm-up
        let warmup_metadata = BatchMetadata {
            start_time: 1_700_000_000_000_000_000,
            end_time: 1_700_000_300_000_000_000,
            schema_ids: "075bcd15e5b2ed60f26e66085ac2b2e8".to_string(), // Example MD5 hash
        };

        let _ = ctx
            .uploader
            .upload(ctx.data.clone(), &ctx.event_name, &warmup_metadata)
            .await
            .expect("Warm-up upload failed");
        let warmup_elapsed = start_warmup.elapsed();
        println!("🔥 Warm-up upload complete in {warmup_elapsed:.2?}");

        println!("🚀 Launching {parallel_uploads} parallel uploads...");

        let start_all = Instant::now();

        let mut handles = vec![];
        for i in 0..parallel_uploads {
            let uploader = ctx.uploader.clone();
            let data = ctx.data.clone();
            let event_name = ctx.event_name.to_string();
            let handle = tokio::spawn(async move {
                let start = Instant::now();

                // Create test metadata for this upload
                let metadata = BatchMetadata {
                    start_time: 1_700_000_000_000_000_000,
                    end_time: 1_700_000_300_000_000_000,
                    schema_ids: "075bcd15e5b2ed60f26e66085ac2b2e8".to_string(), // Example MD5 hash
                };

                let resp = uploader
                    .upload(data, &event_name, &metadata)
                    .await
                    .unwrap_or_else(|_| panic!("Upload {i} failed"));
                let elapsed = start.elapsed();
                println!(
                    "✅ Upload {i} complete in {elapsed:.2?}. Ticket: {}",
                    resp.ticket
                );
                elapsed
            });

            handles.push(handle);
        }

        let durations: Vec<_> = futures::future::join_all(handles)
            .await
            .into_iter()
            .map(|res| res.expect("Join error in upload task"))
            .collect();

        let total_time = start_all.elapsed();

        let avg_ms =
            durations.iter().map(|d| d.as_millis()).sum::<u128>() as f64 / durations.len() as f64;
        println!("📊 Average upload duration: {avg_ms:.2} ms");

        println!("⏱️ Total elapsed for {parallel_uploads} parallel uploads: {total_time:.2?}");
    }
}
