pub(crate) mod client;

#[cfg(test)]
mod tests {
    use crate::config_service::client::{AuthMethod, GenevaConfigClient, GenevaConfigClientConfig};
    use openssl::{
        asn1::Asn1Time,
        bn::{BigNum, MsbOption},
        hash::MessageDigest,
        pkcs12::Pkcs12,
        pkey::{PKey, Private},
        rsa::Rsa,
        ssl::{SslAcceptor, SslMethod, SslVersion},
        x509::{
            extension::{
                AuthorityKeyIdentifier, BasicConstraints, ExtendedKeyUsage, KeyUsage,
                SubjectAlternativeName, SubjectKeyIdentifier,
            },
            X509NameBuilder, X509,
        },
    };
    use rcgen::generate_simple_self_signed;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::path::PathBuf;
    use std::thread;
    use tempfile::NamedTempFile;
    use tokio::sync::Mutex;
    use uuid::Uuid;
    use wiremock::matchers::{header, method, path, query_param, query_param_is_missing};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    // Tests that mutate IDENTITY_ENDPOINT / IDENTITY_HEADER must hold this lock.
    static ENV_LOCK: Mutex<()> = Mutex::const_new(());

    struct EnvVarGuard {
        previous: Vec<(&'static str, Option<String>)>,
    }

    impl EnvVarGuard {
        fn set(vars: &[(&'static str, String)]) -> Self {
            let previous = vars
                .iter()
                .map(|(name, _)| (*name, std::env::var(name).ok()))
                .collect();
            for (name, value) in vars {
                // Tests serialize environment mutations with ENV_LOCK.
                unsafe { std::env::set_var(name, value) };
            }
            Self { previous }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            for (name, value) in self.previous.drain(..) {
                if let Some(value) = value {
                    // Tests serialize environment mutations with ENV_LOCK.
                    unsafe { std::env::set_var(name, value) };
                } else {
                    // Tests serialize environment mutations with ENV_LOCK.
                    unsafe { std::env::remove_var(name) };
                }
            }
        }
    }

    fn generate_test_password() -> String {
        Uuid::new_v4().to_string()
    }

    #[test]
    fn config_fields() {
        let config = GenevaConfigClientConfig {
            endpoint: "https://example.com".to_string(),
            environment: "env".to_string(),
            account: "acct".to_string(),
            namespace: "ns".to_string(),
            region: "region".to_string(),
            config_major_version: 1,
            auth_method: AuthMethod::WorkloadIdentity {
                resource: "https://monitor.azure.com".to_string(),
            },
            msi_resource: None,
            test_root_ca_pem: None,
        };

        assert_eq!(config.environment, "env");
        assert_eq!(config.account, "acct");

        match config.auth_method {
            AuthMethod::WorkloadIdentity { .. } => {}
            _ => panic!("expected WorkloadIdentity variant"),
        }
    }

    fn generate_self_signed_p12() -> (NamedTempFile, String) {
        let password = generate_test_password();

        // This returns a CertifiedKey, not a Certificate
        let cert = generate_simple_self_signed(vec!["localhost".into()]).unwrap();

        // The correct methods for rcgen 0.13:
        let cert_der = cert.cert.der().as_ref().to_vec();
        let key_der = cert.signing_key.serialize_der();

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

        let pkcs12_size = pkcs12.len();
        println!("PKCS#12 size: {pkcs12_size}");
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(&pkcs12).unwrap();
        println!("Temp file path: {:?}", file.path());
        println!("Temp file name: {:?}", file.path().file_name());

        (file, password)
    }

    struct GeneratedTlsMaterial {
        client_p12_file: NamedTempFile,
        client_password: String,
        root_ca_pem: Vec<u8>,
        server_cert: X509,
        server_key: PKey<Private>,
    }

    struct LocalTlsConfigService {
        endpoint: String,
        handle: thread::JoinHandle<()>,
    }

    fn build_pkcs12_der(cert: &X509, key: &PKey<Private>, password: &str) -> Vec<u8> {
        Pkcs12::builder()
            .name("alias")
            .pkey(key)
            .cert(cert)
            .build2(password)
            .unwrap()
            .to_der()
            .unwrap()
    }

    fn build_pkcs12(cert: &X509, key: &PKey<Private>, password: &str) -> NamedTempFile {
        let pkcs12 = build_pkcs12_der(cert, key, password);
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(&pkcs12).unwrap();
        file
    }

    fn build_serial_number() -> openssl::asn1::Asn1Integer {
        let mut serial = BigNum::new().unwrap();
        serial.rand(159, MsbOption::MAYBE_ZERO, false).unwrap();
        serial.to_asn1_integer().unwrap()
    }

    fn build_subject_name(common_name: &str) -> openssl::x509::X509Name {
        let mut name_builder = X509NameBuilder::new().unwrap();
        name_builder
            .append_entry_by_text("CN", common_name)
            .unwrap();
        name_builder.build()
    }

    fn generate_ca() -> (X509, PKey<Private>) {
        let key = PKey::from_rsa(Rsa::generate(2048).unwrap()).unwrap();
        let subject_name = build_subject_name("GenevaUploader Test CA");

        let mut builder = X509::builder().unwrap();
        builder.set_version(2).unwrap();
        let serial_number = build_serial_number();
        builder.set_serial_number(&serial_number).unwrap();
        builder.set_subject_name(&subject_name).unwrap();
        builder.set_issuer_name(&subject_name).unwrap();
        builder.set_pubkey(&key).unwrap();
        let not_before = Asn1Time::days_from_now(0).unwrap();
        builder.set_not_before(&not_before).unwrap();
        let not_after = Asn1Time::days_from_now(365).unwrap();
        builder.set_not_after(&not_after).unwrap();
        builder
            .append_extension(BasicConstraints::new().critical().ca().build().unwrap())
            .unwrap();
        builder
            .append_extension(
                KeyUsage::new()
                    .critical()
                    .key_cert_sign()
                    .crl_sign()
                    .build()
                    .unwrap(),
            )
            .unwrap();
        let subject_key_identifier = SubjectKeyIdentifier::new()
            .build(&builder.x509v3_context(None, None))
            .unwrap();
        builder.append_extension(subject_key_identifier).unwrap();
        builder.sign(&key, MessageDigest::sha256()).unwrap();

        (builder.build(), key)
    }

    fn generate_signed_leaf(
        ca_cert: &X509,
        ca_key: &PKey<Private>,
        common_name: &str,
        subject_alt_name: Option<&str>,
        for_server: bool,
    ) -> (X509, PKey<Private>) {
        let key = PKey::from_rsa(Rsa::generate(2048).unwrap()).unwrap();
        let subject_name = build_subject_name(common_name);

        let mut builder = X509::builder().unwrap();
        builder.set_version(2).unwrap();
        let serial_number = build_serial_number();
        builder.set_serial_number(&serial_number).unwrap();
        builder.set_subject_name(&subject_name).unwrap();
        builder.set_issuer_name(ca_cert.subject_name()).unwrap();
        builder.set_pubkey(&key).unwrap();
        let not_before = Asn1Time::days_from_now(0).unwrap();
        builder.set_not_before(&not_before).unwrap();
        let not_after = Asn1Time::days_from_now(365).unwrap();
        builder.set_not_after(&not_after).unwrap();
        builder
            .append_extension(BasicConstraints::new().critical().build().unwrap())
            .unwrap();
        builder
            .append_extension(
                KeyUsage::new()
                    .critical()
                    .digital_signature()
                    .key_encipherment()
                    .build()
                    .unwrap(),
            )
            .unwrap();
        let mut extended_key_usage = ExtendedKeyUsage::new();
        if for_server {
            extended_key_usage.server_auth();
        } else {
            extended_key_usage.client_auth();
        }
        builder
            .append_extension(extended_key_usage.build().unwrap())
            .unwrap();
        let subject_key_identifier = SubjectKeyIdentifier::new()
            .build(&builder.x509v3_context(Some(ca_cert), None))
            .unwrap();
        builder.append_extension(subject_key_identifier).unwrap();
        let authority_key_identifier = AuthorityKeyIdentifier::new()
            .keyid(true)
            .issuer(true)
            .build(&builder.x509v3_context(Some(ca_cert), None))
            .unwrap();
        builder.append_extension(authority_key_identifier).unwrap();
        if let Some(dns_name) = subject_alt_name {
            let subject_alt_name = SubjectAlternativeName::new()
                .dns(dns_name)
                .build(&builder.x509v3_context(Some(ca_cert), None))
                .unwrap();
            builder.append_extension(subject_alt_name).unwrap();
        }
        builder.sign(ca_key, MessageDigest::sha256()).unwrap();

        (builder.build(), key)
    }

    fn generate_ca_signed_tls_material() -> GeneratedTlsMaterial {
        let (ca_cert, ca_key) = generate_ca();
        let (server_cert, server_signing_key) =
            generate_signed_leaf(&ca_cert, &ca_key, "localhost", Some("localhost"), true);

        let (client_cert, client_signing_key) =
            generate_signed_leaf(&ca_cert, &ca_key, "GenevaUploader Test Client", None, false);
        let client_password = generate_test_password();
        let client_p12_file = build_pkcs12(&client_cert, &client_signing_key, &client_password);

        GeneratedTlsMaterial {
            client_p12_file,
            client_password,
            root_ca_pem: ca_cert.to_pem().unwrap(),
            server_cert,
            server_key: server_signing_key,
        }
    }

    fn spawn_tls_config_service(
        response_body: String,
        server_cert: X509,
        server_key: PKey<Private>,
    ) -> LocalTlsConfigService {
        let mut builder = SslAcceptor::mozilla_intermediate_v5(SslMethod::tls()).unwrap();
        builder.set_certificate(&server_cert).unwrap();
        builder.set_private_key(&server_key).unwrap();
        builder.check_private_key().unwrap();
        builder
            .set_min_proto_version(Some(SslVersion::TLS1_2))
            .unwrap();
        builder
            .set_max_proto_version(Some(SslVersion::TLS1_2))
            .unwrap();
        let acceptor = builder.build();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!(
            "https://localhost:{}",
            listener.local_addr().unwrap().port()
        );
        let handle = thread::spawn(move || {
            let (socket, _) = listener.accept().unwrap();
            let Ok(mut tls_stream) = acceptor.accept(socket) else {
                return;
            };

            let mut request = Vec::new();
            let mut buffer = [0_u8; 1024];
            loop {
                let bytes_read = tls_stream.read(&mut buffer).unwrap();
                if bytes_read == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..bytes_read]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }

            let request = String::from_utf8_lossy(&request);
            assert!(
                request.starts_with("GET /api/agent/v3/mockenv/mockacct/MonitoringStorageKeys/?"),
                "unexpected request: {request}",
            );

            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                response_body.len(),
                response_body,
            );
            tls_stream.write_all(response.as_bytes()).unwrap();
            tls_stream.flush().unwrap();
        });

        LocalTlsConfigService { endpoint, handle }
    }

    fn config_service_response() -> (String, &'static str, &'static str) {
        let jwt_endpoint = "https://test.endpoint";
        let valid_token =
            "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJFbmRwb2ludCI6Imh0dHBzOi8vdGVzdC5lbmRwb2ludCJ9.signature";
        let response = serde_json::json!({
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

        (response.to_string(), jwt_endpoint, valid_token)
    }

    #[cfg_attr(target_os = "macos", ignore)] // cert generated not compatible with macOS
    #[tokio::test]
    async fn get_ingestion_info_mocked() {
        let mock_server = MockServer::start().await;
        let (mock_response, jwt_endpoint, valid_token) = config_service_response();

        Mock::given(method("GET"))
            .and(path(
                "/api/agent/v3/mockenv/mockacct/MonitoringStorageKeys/",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_string(mock_response))
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
            msi_resource: None,
            test_root_ca_pem: None,
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

    #[tokio::test(flavor = "current_thread")]
    async fn user_managed_identity_by_resource_id_uses_local_msi_res_id_endpoint() {
        let _env_lock = ENV_LOCK.lock().await;
        let msi_server = MockServer::start().await;
        let config_server = MockServer::start().await;
        let full_resource_id = "/subscriptions/test-sub/resourceGroups/test-rg/providers/Microsoft.Kubernetes/connectedClusters/test-cluster/providers/Microsoft.KubernetesConfiguration/extensions/test-extension-a";
        let fake_msi_token = "fake-msi-token";
        let (mock_response, jwt_endpoint, valid_token) = config_service_response();
        let _env_guard = EnvVarGuard::set(&[
            (
                "IDENTITY_ENDPOINT",
                format!("{}/metadata/identity/oauth2/token", msi_server.uri()),
            ),
            ("IDENTITY_HEADER", "fake-identity-header".to_string()),
        ]);

        Mock::given(method("GET"))
            .and(path("/metadata/identity/oauth2/token"))
            .and(header("Metadata", "true"))
            .and(header("X-IDENTITY-HEADER", "fake-identity-header"))
            .and(query_param("api-version", "2018-02-01"))
            .and(query_param("resource", "https://monitor.core.windows.net"))
            .and(query_param("msi_res_id", full_resource_id))
            .and(query_param_is_missing("client_id"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": fake_msi_token,
                "expires_on": "1777334267",
                "resource": "https://monitor.core.windows.net/",
                "token_type": "Bearer"
            })))
            .expect(1)
            .mount(&msi_server)
            .await;

        Mock::given(method("GET"))
            .and(path(
                "/userapi/agent/v3/mockenv/mockacct/MonitoringStorageKeys/",
            ))
            .and(header("Authorization", "Bearer fake-msi-token"))
            .respond_with(ResponseTemplate::new(200).set_body_string(mock_response))
            .expect(1)
            .mount(&config_server)
            .await;

        let config = GenevaConfigClientConfig {
            endpoint: config_server.uri(),
            environment: "mockenv".into(),
            account: "mockacct".into(),
            namespace: "mockns".into(),
            region: "mockregion".into(),
            config_major_version: 1,
            auth_method: AuthMethod::UserManagedIdentityByResourceId {
                resource_id: full_resource_id.to_string(),
            },
            msi_resource: Some("https://monitor.core.windows.net/.default".into()),
            test_root_ca_pem: None,
        };

        let client = GenevaConfigClient::new(config).unwrap();
        let (ingestion_info, moniker_info, token_endpoint) =
            client.get_ingestion_info().await.unwrap();

        assert_eq!(ingestion_info.endpoint, "https://mock.ingestion.endpoint");
        assert_eq!(ingestion_info.auth_token, valid_token);
        assert_eq!(moniker_info.name, "mock-diag-moniker");
        assert_eq!(token_endpoint, jwt_endpoint);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn user_managed_identity_by_resource_id_returns_error_on_local_msi_failure() {
        let _env_lock = ENV_LOCK.lock().await;
        let msi_server = MockServer::start().await;
        let config_server = MockServer::start().await;
        let full_resource_id = "/subscriptions/test-sub/resourceGroups/test-rg/providers/Microsoft.Kubernetes/connectedClusters/test-cluster/providers/Microsoft.KubernetesConfiguration/extensions/test-extension-a";
        let _env_guard = EnvVarGuard::set(&[
            (
                "IDENTITY_ENDPOINT",
                format!("{}/metadata/identity/oauth2/token", msi_server.uri()),
            ),
            ("IDENTITY_HEADER", "fake-identity-header".to_string()),
        ]);

        Mock::given(method("GET"))
            .and(path("/metadata/identity/oauth2/token"))
            .and(query_param("msi_res_id", full_resource_id))
            .respond_with(ResponseTemplate::new(503))
            .expect(1)
            .mount(&msi_server)
            .await;

        Mock::given(method("GET"))
            .and(path(
                "/userapi/agent/v3/mockenv/mockacct/MonitoringStorageKeys/",
            ))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&config_server)
            .await;

        let config = GenevaConfigClientConfig {
            endpoint: config_server.uri(),
            environment: "mockenv".into(),
            account: "mockacct".into(),
            namespace: "mockns".into(),
            region: "mockregion".into(),
            config_major_version: 1,
            auth_method: AuthMethod::UserManagedIdentityByResourceId {
                resource_id: full_resource_id.to_string(),
            },
            msi_resource: Some("https://monitor.core.windows.net/.default".into()),
            test_root_ca_pem: None,
        };

        let client = GenevaConfigClient::new(config).unwrap();
        let result = client.get_ingestion_info().await;

        match result {
            Err(crate::config_service::client::GenevaConfigClientError::MsiAuth(message)) => {
                assert!(message.contains("503"));
            }
            other => panic!("Expected local MSI auth error, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn tls_rejects_untrusted_runtime_generated_ca() {
        let tls_material = generate_ca_signed_tls_material();
        let (response_body, _, _) = config_service_response();
        let tls_server = spawn_tls_config_service(
            response_body,
            tls_material.server_cert,
            tls_material.server_key,
        );

        let config = GenevaConfigClientConfig {
            endpoint: tls_server.endpoint,
            environment: "mockenv".into(),
            account: "mockacct".into(),
            namespace: "mockns".into(),
            region: "mockregion".into(),
            config_major_version: 1,
            auth_method: AuthMethod::Certificate {
                path: PathBuf::from(
                    tls_material
                        .client_p12_file
                        .path()
                        .to_string_lossy()
                        .to_string(),
                ),
                password: tls_material.client_password,
            },
            msi_resource: None,
            test_root_ca_pem: None,
        };

        let client = GenevaConfigClient::new(config).unwrap();
        let result = client.get_ingestion_info().await;

        assert!(matches!(
            result,
            Err(crate::config_service::client::GenevaConfigClientError::Http(_))
        ));
        tls_server.handle.join().unwrap();
    }

    #[tokio::test]
    async fn tls_accepts_runtime_generated_ca() {
        let tls_material = generate_ca_signed_tls_material();
        let (response_body, jwt_endpoint, valid_token) = config_service_response();
        let tls_server = spawn_tls_config_service(
            response_body,
            tls_material.server_cert,
            tls_material.server_key,
        );

        let config = GenevaConfigClientConfig {
            endpoint: tls_server.endpoint,
            environment: "mockenv".into(),
            account: "mockacct".into(),
            namespace: "mockns".into(),
            region: "mockregion".into(),
            config_major_version: 1,
            auth_method: AuthMethod::Certificate {
                path: PathBuf::from(
                    tls_material
                        .client_p12_file
                        .path()
                        .to_string_lossy()
                        .to_string(),
                ),
                password: tls_material.client_password,
            },
            msi_resource: None,
            test_root_ca_pem: Some(tls_material.root_ca_pem),
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
        assert_eq!(moniker_info.name, "mock-diag-moniker");
        assert_eq!(moniker_info.account_group, "mock-diag-group");
        assert_eq!(token_endpoint, jwt_endpoint);
        tls_server.handle.join().unwrap();
    }

    #[cfg_attr(target_os = "macos", ignore)] // cert generated not compatible with macOS
    #[tokio::test]
    async fn error_handling_with_non_success_status() {
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
            msi_resource: None,
            test_root_ca_pem: None,
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
            panic!("Expected RequestFailed with 403, got: {result:?}");
        }
    }

    #[cfg_attr(target_os = "macos", ignore)] // cert generated not compatible with macOS
    #[tokio::test]
    async fn missing_ingestion_gateway_info() {
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
            msi_resource: None,
            test_root_ca_pem: None,
        };

        let client = GenevaConfigClient::new(config).unwrap();
        let result = client.get_ingestion_info().await;

        assert!(result.is_err());
        match result {
            Err(err) => match err {
                crate::config_service::client::GenevaConfigClientError::AuthInfoNotFound(_) => {
                    // Test passed
                }
                _ => panic!("Expected AuthInfoNotFound error, got: {err:?}"),
            },
            _ => panic!("Expected error, got success"),
        }
    }

    #[cfg_attr(target_os = "macos", ignore)] // cert generated not compatible with macOS
    #[tokio::test]
    async fn invalid_certificate_path() {
        let config = GenevaConfigClientConfig {
            endpoint: "https://example.com".to_string(),
            environment: "env".to_string(),
            account: "acct".to_string(),
            namespace: "ns".to_string(),
            region: "region".to_string(),
            config_major_version: 1,
            auth_method: AuthMethod::Certificate {
                path: PathBuf::from("/nonexistent/path.p12".to_string()),
                password: generate_test_password(),
            },
            msi_resource: None,
            test_root_ca_pem: None,
        };

        let result = GenevaConfigClient::new(config);

        assert!(result.is_err());
        match result {
            Err(err) => match err {
                crate::config_service::client::GenevaConfigClientError::Certificate(_) => {
                    // Test passed
                }
                _ => panic!("Expected Io error, got: {err:?}"),
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
    // cargo test get_ingestion_info_real_server -- --ignored
    // ```
    use std::env;
    #[tokio::test]
    #[ignore] // This test is ignored by default to prevent running in CI pipelines
    async fn get_ingestion_info_real_server() {
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
            msi_resource: None,
            test_root_ca_pem: None,
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
        let token_len = ingestion_info.auth_token.len();
        println!("Auth token length: {token_len}");
        println!("Moniker name: {}", moniker.name);
    }
}
