//! Shared utilities for Geneva Config clients (XML and Ingestion Info).

use crate::config_service::auth::{configure_tls_connector, load_identity, AuthMethod};
use crate::config_service::error::{GenevaConfigClientError, GenevaConfigClientResult};
use reqwest::{
    header::{HeaderMap, HeaderValue, ACCEPT, USER_AGENT},
    Client,
};

pub(crate) const AGENT_IDENTITY: &str = "GenevaUploader";
pub(crate) const AGENT_VERSION: &str = "0.1";

/// Configuration for the Geneva Config Client.
///
/// # Fields
/// * `endpoint` - The Geneva Config Service endpoint URL
/// * `environment` - Environment name (e.g., "prod", "dev")
/// * `account` - Account name in Geneva
/// * `namespace` - Namespace for the configuration
/// * `region` - Azure region (e.g., "westus2")
/// * `config_major_version` - Major version of the configuration schema
/// * `auth_method` - Authentication method to use (Certificate or ManagedIdentity)
///
/// # Example
/// ```ignore
/// let config = GenevaConfigClientConfig {
///     endpoint: "https://example.geneva.com".to_string(),
///     environment: "prod".to_string(),
///     account: "myaccount".to_string(),
///     namespace: "myservice".to_string(),
///     region: "westus2".to_string(),
///     config_major_version: 1,
///     auth_method: AuthMethod::Certificate {
///         path: "/path/to/cert.p12".to_string(),
///         password: "password".to_string(),
///     },
/// };
/// ```
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub(crate) struct GenevaConfigClientConfig {
    pub(crate) endpoint: String,
    pub(crate) environment: String,
    pub(crate) account: String,
    pub(crate) namespace: String,
    pub(crate) region: String,
    pub(crate) config_major_version: u32,
    pub(crate) auth_method: AuthMethod, // agent_identity and agent_version are hardcoded for now
}

/// Builds static headers for Geneva Config clients.
///
/// # Arguments
/// * `agent_identity` - The identity of the agent (e.g., "GenevaUploader").
/// * `agent_version` - The version of the agent (e.g., "0.1").
///
/// # Returns
/// * `HeaderMap` - A map of static headers including `User-Agent` and `Accept`.
pub(crate) fn build_static_headers(agent_identity: &str, agent_version: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    let user_agent = format!("{}-{}", agent_identity, agent_version);
    headers.insert(USER_AGENT, HeaderValue::from_str(&user_agent).unwrap());
    headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
    headers
}

/// Initializes an HTTP client with the provided configuration.
///
/// # Arguments
/// * `auth_method` - The authentication method (`AuthMethod::Certificate` or `AuthMethod::ManagedIdentity`).
///
/// # Returns
/// * `GenevaConfigClientResult<Client>` - A configured `reqwest::Client` instance.
pub(crate) fn initialize_http_client(auth_method: &AuthMethod) -> GenevaConfigClientResult<Client> {
    let mut client_builder = Client::builder()
        .http1_only()
        .timeout(std::time::Duration::from_secs(30));

    match auth_method {
        AuthMethod::Certificate { path, password } => {
            let identity = load_identity(path, password)?;
            let tls_connector =
                configure_tls_connector(native_tls::TlsConnector::builder(), identity)
                    .build()
                    .map_err(|e| GenevaConfigClientError::Certificate(e.to_string()))?;
            client_builder = client_builder.use_preconfigured_tls(tls_connector);
        }
        AuthMethod::ManagedIdentity => {
            return Err(GenevaConfigClientError::AuthMethodNotImplemented(
                "Managed Identity authentication is not implemented yet".into(),
            ));
        }
    }

    client_builder
        .build()
        .map_err(GenevaConfigClientError::Http)
}

/// Gets the operating system type as a string.
///
/// # Returns
/// * `&'static str` - The operating system type ("Darwin", "Windows", "Linux", etc.).
#[inline]
pub(crate) fn get_os_type() -> &'static str {
    match std::env::consts::OS {
        "macos" => "Darwin",
        "windows" => "Windows",
        "linux" => "Linux",
        other => other,
    }
}
