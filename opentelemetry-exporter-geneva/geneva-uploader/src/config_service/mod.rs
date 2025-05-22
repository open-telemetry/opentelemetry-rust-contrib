pub(crate) mod client;

#[cfg(test)]
mod tests {
    use crate::config_service::client::{AuthMethod, GenevaConfigClient, GenevaConfigClientConfig};
    use openssl::{pkcs12::Pkcs12, pkey::PKey, x509::X509};
    use rcgen::generate_simple_self_signed;
    use std::io::Write;
    use std::path::PathBuf;
    use tempfile::NamedTempFile;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn test_config_fields() {
        let config = GenevaConfigClientConfig {
            endpoint: "https://example.com".to_string(),
            environment: "env".to_string(),
            account: "acct".to_string(),
            namespace: "ns".to_string(),
            region: "region".to_string(),
            config_major_version: 1,
            auth_method: AuthMethod::ManagedIdentity,
        };

        assert_eq!(config.environment, "env");
        assert_eq!(config.account, "acct");
        assert!(matches!(config.auth_method, AuthMethod::ManagedIdentity));
    }

    fn generate_self_signed_p12() -> (NamedTempFile, String) {
        let password = "test".to_string();

        // This returns a CertifiedKey, not a Certificate
        let cert = generate_simple_self_signed(vec!["localhost".into()]).unwrap();

        // The correct methods for rcgen 0.13:
        let cert_der = cert.cert.der().as_ref().to_vec();
        let key_der = cert.key_pair.serialize_der();

        // Convert to OpenSSL types
        let x509 = X509::from_der(&cert_der).unwrap();
        let pkey = PKey::private_key_from_der(&key_der).unwrap();

        // Build PKCS#12 - fixed builder usage to match OpenSSL version
        // Remove deprecated .build() method
        let pkcs12 = Pkcs12::builder()
            .name("alias")
            .pkey(&pkey)
            .cert(&x509)
            .build2(&password)
            .unwrap()
            .to_der()
            .unwrap();

        println!("PKCS#12 size: {}", pkcs12.len());
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(&pkcs12).unwrap();
        println!("Temp file path: {:?}", file.path());
        println!("Temp file name: {:?}", file.path().file_name());

        (file, password)
    }

    #[cfg_attr(target_os = "macos", ignore)] // cert generated not compatible with macOS
    #[tokio::test]
    async fn test_get_ingestion_info_mocked() {
        let mock_server = MockServer::start().await;
        let jwt_endpoint = "https://test.endpoint";
        let valid_token = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJFbmRwb2ludCI6Imh0dHBzOi8vdGVzdC5lbmRwb2ludCJ9.signature";

        let mock_response = serde_json::json!({
            "IngestionGatewayInfo": {
                "Endpoint": "https://mock.ingestion.endpoint",
                "AuthToken": valid_token,
                "AuthTokenExpiryTime": "2030-01-01T00:00:00Z"
            },
            "StorageAccountKeys": [
                {
                    "AccountMonikerName": "mock-diag-moniker",
                    "AccountGroupName": "mock-diag-group",
                    "IsPrimaryMoniker": true
                }
            ],
            "TagId": "mock-tag-id"
        });

        Mock::given(method("GET"))
            .and(path(
                "/api/agent/v3/mockenv/mockacct/MonitoringStorageKeys/",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(mock_response))
            .mount(&mock_server)
            .await;

        let (temp_p12_file, password) = generate_self_signed_p12();

        let config = GenevaConfigClientConfig {
            endpoint: mock_server.uri(),
            environment: "mockenv".into(),
            account: "mockacct".into(),
            namespace: "mockns".into(),
            region: "mockregion".into(),
            config_major_version: 1,
            auth_method: AuthMethod::Certificate {
                path: PathBuf::from(temp_p12_file.path().to_string_lossy().to_string()),
                password,
            },
        };

        let client = GenevaConfigClient::new(config).unwrap();
        let (ingestion_info, moniker_info, token_endpoint) =
            client.get_ingestion_info().await.unwrap();

        assert_eq!(ingestion_info.endpoint, "https://mock.ingestion.endpoint");
        assert_eq!(ingestion_info.auth_token, valid_token);
        assert_eq!(
            ingestion_info.auth_token_expiry_time,
            "2030-01-01T00:00:00Z"
        );

        // Check moniker info
        assert_eq!(moniker_info.name, "mock-diag-moniker");
        assert_eq!(moniker_info.account_group, "mock-diag-group");
        assert_eq!(token_endpoint, jwt_endpoint);
    }

    #[cfg_attr(target_os = "macos", ignore)] // cert generated not compatible with macOS
    #[tokio::test]
    async fn test_error_handling_with_non_success_status() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path(
                "/api/agent/v3/mockenv/mockacct/MonitoringStorageKeys/",
            ))
            .respond_with(ResponseTemplate::new(403).set_body_string("Forbidden"))
            .mount(&mock_server)
            .await;

        let (temp_p12_file, password) = generate_self_signed_p12();

        let config = GenevaConfigClientConfig {
            endpoint: mock_server.uri(),
            environment: "mockenv".into(),
            account: "mockacct".into(),
            namespace: "mockns".into(),
            region: "mockregion".into(),
            config_major_version: 1,
            auth_method: AuthMethod::Certificate {
                path: PathBuf::from(temp_p12_file.path().to_string_lossy().to_string()),
                password,
            },
        };

        let client = GenevaConfigClient::new(config).unwrap();
        let result = client.get_ingestion_info().await;

        assert!(result.is_err());
        if let Err(crate::config_service::client::GenevaConfigClientError::RequestFailed {
            status,
            ..
        }) = result
        {
            assert_eq!(status, 403);
        } else {
            panic!("Expected RequestFailed with 403, got: {:?}", result);
        }
    }

    #[cfg_attr(target_os = "macos", ignore)] // cert generated not compatible with macOS
    #[tokio::test]
    async fn test_missing_ingestion_gateway_info() {
        let mock_server = MockServer::start().await;

        // Response without IngestionGatewayInfo
        let mock_response = serde_json::json!({
            "SomeOtherField": "value"
        });

        Mock::given(method("GET"))
            .and(path(
                "/api/agent/v3/mockenv/mockacct/MonitoringStorageKeys/",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(mock_response))
            .mount(&mock_server)
            .await;

        let (temp_p12_file, password) = generate_self_signed_p12();

        let config = GenevaConfigClientConfig {
            endpoint: mock_server.uri(),
            environment: "mockenv".into(),
            account: "mockacct".into(),
            namespace: "mockns".into(),
            region: "mockregion".into(),
            config_major_version: 1,
            auth_method: AuthMethod::Certificate {
                path: PathBuf::from(temp_p12_file.path().to_string_lossy().to_string()),
                password,
            },
        };

        let client = GenevaConfigClient::new(config).unwrap();
        let result = client.get_ingestion_info().await;

        assert!(result.is_err());
        match result {
            Err(err) => match err {
                crate::config_service::client::GenevaConfigClientError::AuthInfoNotFound(_) => {
                    // Test passed
                }
                _ => panic!("Expected AuthInfoNotFound error, got: {:?}", err),
            },
            _ => panic!("Expected error, got success"),
        }
    }

    #[cfg_attr(target_os = "macos", ignore)] // cert generated not compatible with macOS
    #[tokio::test]
    async fn test_invalid_certificate_path() {
        let config = GenevaConfigClientConfig {
            endpoint: "https://example.com".to_string(),
            environment: "env".to_string(),
            account: "acct".to_string(),
            namespace: "ns".to_string(),
            region: "region".to_string(),
            config_major_version: 1,
            auth_method: AuthMethod::Certificate {
                path: PathBuf::from("/nonexistent/path.p12".to_string()),
                password: "test".to_string(),
            },
        };

        let result = GenevaConfigClient::new(config);

        assert!(result.is_err());
        match result {
            Err(err) => match err {
                crate::config_service::client::GenevaConfigClientError::Certificate(_) => {
                    // Test passed
                }
                _ => panic!("Expected Io error, got: {:?}", err),
            },
            _ => panic!("Expected error, got success"),
        }
    }

    // To run this test, set the following environment variables:
    // ```bash
    // export GENEVA_ENDPOINT="https://your-geneva-endpoint.com"
    // export GENEVA_ENVIRONMENT="production"
    // export GENEVA_ACCOUNT="your-account"
    // export GENEVA_NAMESPACE="your-namespace"
    // export GENEVA_REGION="your-region"
    // export GENEVA_CONFIG_MAJOR_VERSION="config-version"
    // export GENEVA_CERT_PATH="/path/to/your/certificate.p12"
    // export GENEVA_CERT_PASSWORD="your-certificate-password" // Empty string if no password
    // cargo test test_get_ingestion_info_real_server -- --ignored
    // ```
    use std::env;
    #[tokio::test]
    #[ignore] // This test is ignored by default to prevent running in CI pipelines
    async fn test_get_ingestion_info_real_server() {
        // Read configuration from environment variables
        let endpoint =
            env::var("GENEVA_ENDPOINT").expect("GENEVA_ENDPOINT environment variable must be set");
        let environment = env::var("GENEVA_ENVIRONMENT")
            .expect("GENEVA_ENVIRONMENT environment variable must be set");
        let account =
            env::var("GENEVA_ACCOUNT").expect("GENEVA_ACCOUNT environment variable must be set");
        let namespace = env::var("GENEVA_NAMESPACE")
            .expect("GENEVA_NAMESPACE environment variable must be set");
        let region =
            env::var("GENEVA_REGION").expect("GENEVA_REGION environment variable must be set");
        let cert_path = env::var("GENEVA_CERT_PATH")
            .expect("GENEVA_CERT_PATH environment variable must be set");
        let cert_password = env::var("GENEVA_CERT_PASSWORD")
            .expect("GENEVA_CERT_PASSWORD environment variable must be set");
        let config_major_version = env::var("GENEVA_CONFIG_MAJOR_VERSION")
            .expect("GENEVA_CONFIG_MAJOR_VERSION environment variable must be set")
            .parse::<u32>() // Convert string to u32
            .expect("GENEVA_CONFIG_MAJOR_VERSION must be a valid unsigned integer");

        let config = GenevaConfigClientConfig {
            endpoint,
            environment,
            account,
            namespace,
            region,
            config_major_version,
            auth_method: AuthMethod::Certificate {
                path: PathBuf::from(cert_path),
                password: cert_password,
            },
        };

        println!("Connecting to real Geneva Config service...");
        let client = GenevaConfigClient::new(config).expect("Failed to create client");

        println!("Fetching ingestion info...");
        let (ingestion_info, moniker, _token_endpoint) = client
            .get_ingestion_info()
            .await
            .expect("Failed to get ingestion info");
        // Validate the response contains expected fields
        assert!(
            !ingestion_info.endpoint.is_empty(),
            "Endpoint should not be empty"
        );
        assert!(
            !ingestion_info.auth_token.is_empty(),
            "Auth token should not be empty"
        );
        assert!(!moniker.name.is_empty(), "Moniker name should not be empty");
        assert!(
            !moniker.account_group.is_empty(),
            "Moniker account group should not be empty"
        );

        println!("Successfully connected to real server");
        println!("Endpoint: {}", ingestion_info.endpoint);
        println!("Auth token length: {}", ingestion_info.auth_token.len());
        println!("Moniker name: {}", moniker.name);
    }
}
