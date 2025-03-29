mod client;

#[cfg(test)]
mod tests {
    use crate::config_service::client::{AuthMethod, GenevaConfigClient, GenevaConfigClientConfig};
    use openssl::{pkcs12::Pkcs12, pkey::PKey, x509::X509};
    use rcgen::{generate_simple_self_signed, Certificate};
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

    fn generate_self_signed_p12() -> (PathBuf, String) {
        let password = "test".to_string();

        // This returns a CertifiedKey, not a Certificate
        let cert = generate_simple_self_signed(vec!["localhost".into()]).unwrap();

        // The correct methods for rcgen 0.13:
        let cert_der = cert.cert.der().as_ref().to_vec();
        let key_der = cert.key_pair.serialize_der();

        // Convert to OpenSSL types
        let x509 = X509::from_der(&cert_der).unwrap();
        let pkey = PKey::private_key_from_der(&key_der).unwrap();

        // Build PKCS#12
        let pkcs12 = Pkcs12::builder()
            .build(&password, "alias", &pkey, &x509)
            .unwrap()
            .to_der()
            .unwrap();

        let mut file = NamedTempFile::new().unwrap();
        file.write_all(&pkcs12).unwrap();

        (file.into_temp_path().to_path_buf(), password)
    }

    #[tokio::test]
    async fn test_get_ingestion_info_mocked() {
        let mock_server = MockServer::start().await;

        let mock_response = serde_json::json!({
            "IngestionGatewayInfo": {
                "endpoint": "https://mock.ingestion.endpoint",
                "AuthToken": "mock-token"
            }
        });

        Mock::given(method("GET"))
            .and(path(
                "/api/agent/v3/mockenv/mockacct/MonitoringStorageKeys/",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(mock_response))
            .mount(&mock_server)
            .await;

        let (temp_p12_path, password) = generate_self_signed_p12();

        let config = GenevaConfigClientConfig {
            endpoint: mock_server.uri(),
            environment: "mockenv".into(),
            account: "mockacct".into(),
            namespace: "mockns".into(),
            region: "mockregion".into(),
            config_major_version: 1,
            auth_method: AuthMethod::Certificate {
                path: temp_p12_path.to_string_lossy().to_string(),
                password,
            },
        };

        let client = GenevaConfigClient::new(config).await.unwrap();
        let result = client.get_ingestion_info().await.unwrap();

        assert_eq!(result.endpoint, "https://mock.ingestion.endpoint");
        assert_eq!(result.auth_token, "mock-token");
    }
}
