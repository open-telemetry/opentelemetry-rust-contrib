// Geneva Config Client with TLS (P12) and TODO: Managed Identity support

use base64::{engine::general_purpose, Engine as _};
use reqwest::Client;
use serde::Deserialize;
use serde_json::Value;
use std::time::Duration;
use thiserror::Error;
use uuid::Uuid;

use native_tls::{Identity, Protocol, TlsConnector};
use std::fs;

/// Certificate should be in PKCS#12 (.p12) format for client TLS authentication.
///
/// If you have PEM format cert and key, you can convert it like this:
///
/// Linux/macOS:
/// openssl pkcs12 -export \
///   -in cert.pem \
///   -inkey key.pem \
///   -out client.p12 \
///   -name "alias"
///
/// Windows (PowerShell):
/// openssl pkcs12 -export -in cert.pem -inkey key.pem -out client.p12 -name "alias"
#[allow(dead_code)]
#[derive(Clone)]
pub enum AuthMethod {
    Certificate { path: String, password: String },
    ManagedIdentity, // TODO: Add support for managed identity auth
}

#[derive(Debug, Error)]
pub enum GenevaConfigClientError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("Certificate error: {0}")]
    Certificate(String),
    #[error("Missing Auth Info: {0}")]
    AuthInfoNotFound(String),
    #[error("Request failed with status {status}: {message}")]
    RequestFailed { status: u16, message: String },
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("TLS error: {0}")]
    Tls(#[from] native_tls::Error),
    #[error("JSON error: {0}")]
    SerdeJson(#[from] serde_json::Error),
}

#[allow(dead_code)]
pub type Result<T> = std::result::Result<T, GenevaConfigClientError>;

#[allow(dead_code)]
#[derive(Clone)]
pub struct GenevaConfigClientConfig {
    pub endpoint: String,
    pub environment: String,
    pub account: String,
    pub namespace: String,
    pub region: String,
    pub config_major_version: u32,
    pub auth_method: AuthMethod, // agent_identity and agent_version are hardcoded for now
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct IngestionGatewayInfo {
    pub(crate) endpoint: String,
    #[serde(rename = "AuthToken")]
    pub(crate) auth_token: String,
}

#[allow(dead_code)]
pub(crate) struct GenevaConfigClient {
    config: GenevaConfigClientConfig,
    http_client: Client,
}

impl GenevaConfigClient {
    #[allow(dead_code)]
    pub(crate) async fn new(config: GenevaConfigClientConfig) -> Result<Self> {
        let mut client_builder = Client::builder()
            // TODO: Remove this before stable. Used for testing with self-signed certs
            .danger_accept_invalid_certs(true)
            .http1_only()
            .timeout(Duration::from_secs(30));

        match &config.auth_method {
            // TODO: Certificate auth would be removed in favor of managed identity.,
            // This is for testing, so we can use self-signed certs, and password in plain text.
            AuthMethod::Certificate { path, password } => {
                let p12_bytes = fs::read(path)?;
                let identity = Identity::from_pkcs12(&p12_bytes, password)?;
                let tls_connector = TlsConnector::builder()
                    .identity(identity)
                    .min_protocol_version(Some(Protocol::Tlsv12))
                    .max_protocol_version(Some(Protocol::Tlsv12)) // TODO: Add support for TLS 1.3
                    .danger_accept_invalid_certs(true)
                    .build()?;

                client_builder = client_builder.use_preconfigured_tls(tls_connector);
            }
            AuthMethod::ManagedIdentity => {
                return Err(GenevaConfigClientError::Certificate(
                    "Managed Identity authentication is not implemented yet".into(),
                ));
            }
        }

        let http_client = client_builder.build()?;

        Ok(Self {
            config,
            http_client,
        })
    }

    #[allow(dead_code)]
    pub(crate) async fn get_ingestion_info(&self) -> Result<IngestionGatewayInfo> {
        let agent_identity = "GenevaUploader"; // TODO make this configurable
        let agent_version = "0.1"; // TODO make this configurable
        let identity = format!(
            "Tenant=Default/Role=GcsClient/RoleInstance={}",
            agent_identity
        );

        let encoded_identity = general_purpose::STANDARD.encode(&identity);
        let version_str = format!("Ver{}v0", self.config.config_major_version);
        // TODO - fetch it during startup once.
        let os_type = std::env::consts::OS
            .replace("macos", "Darwin")
            .replace("windows", "Windows")
            .replace("linux", "Linux");

        let mut url = format!(
            "{}/api/agent/v3/{}/{}/MonitoringStorageKeys/?Namespace={}&Region={}&Identity={}&OSType={}&ConfigMajorVersion={}",
            self.config.endpoint.trim_end_matches('/'),
            self.config.environment,
            self.config.account,
            self.config.namespace,
            self.config.region,
            encoded_identity,
            os_type,
            version_str
        );

        let tag_id = Uuid::new_v4().to_string();
        url.push_str(&format!("&TagId={}", tag_id));

        let req_id = Uuid::new_v4().to_string();
        // TODO: Make tag_id, agent_identity, and agent_version configurable instead of hardcoded/default

        let response = self
            .http_client
            .get(&url)
            .header(
                "User-Agent",
                format!("{}-{}", agent_identity, agent_version),
            )
            .header("x-ms-client-request-id", req_id)
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(GenevaConfigClientError::Http)?;

        let status = response.status();
        let body = response.text().await?;

        if status.is_success() {
            let response_json: Value = serde_json::from_str(&body)?;
            if let Some(info) = response_json.get("IngestionGatewayInfo") {
                let parsed: IngestionGatewayInfo = serde_json::from_value(info.clone())?;
                Ok(parsed)
            } else {
                Err(GenevaConfigClientError::AuthInfoNotFound(
                    "IngestionGatewayInfo not found in response".into(),
                ))
            }
        } else {
            Err(GenevaConfigClientError::RequestFailed {
                status: status.as_u16(),
                message: body,
            })
        }
    }
}
