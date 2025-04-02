// Geneva Config Client with TLS (P12) and TODO: Managed Identity support

use base64::{engine::general_purpose, Engine as _};
use reqwest::Client;
use serde::Deserialize;
use std::time::Duration;
use thiserror::Error;
use uuid::Uuid;

use chrono::{DateTime, Utc};
use native_tls::{Identity, Protocol};
use std::fs;
use std::sync::RwLock;

/// Authentication methods for the Geneva Config Client.
///
/// The client supports two authentication methods:
/// - Certificate-based authentication using PKCS#12 (.p12) files
/// - Managed Identity (Azure) - planned for future implementation
///
/// # Certificate Format
/// Certificates should be in PKCS#12 (.p12) format for client TLS authentication.
///
/// ## Converting from PEM to PKCS#12
///
/// If you have PEM format cert and key, you can convert them using OpenSSL:
///
/// ### Linux/macOS:
/// ```bash
/// openssl pkcs12 -export \
///   -in cert.pem \
///   -inkey key.pem \
///   -out client.p12 \
///   -name "alias"
/// ```
///
/// ### Windows (PowerShell):
/// ```powershell
/// openssl pkcs12 -export -in cert.pem -inkey key.pem -out client.p12 -name "alias"
/// ```
#[allow(dead_code)]
#[derive(Clone)]
pub enum AuthMethod {
    /// Certificate-based authentication
    ///
    /// # Arguments
    /// * `path` - Path to the PKCS#12 (.p12) certificate file
    /// * `password` - Password to decrypt the PKCS#12 file
    Certificate { path: String, password: String },
    /// Azure Managed Identity authentication
    ///
    /// Note(TODO): This is not yet implemented.
    ManagedIdentity,
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
    #[error("Moniker not found: {0}")]
    MonikerNotFound(String),
    #[error("Internal error: {0}")]
    InternalError(String),
    //#[error("Mismatched TagId. Sent: {sent}, Received: {received}")]
    //MismatchedTagId { sent: String, received: String },
}

#[allow(dead_code)]
pub type Result<T> = std::result::Result<T, GenevaConfigClientError>;

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
    #[serde(rename = "Endpoint")]
    pub(crate) endpoint: String,
    #[serde(rename = "AuthToken")]
    pub(crate) auth_token: String,
    #[serde(rename = "AuthTokenExpiryTime")]
    pub(crate) auth_token_expiry_time: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub(crate) struct MonikerInfo {
    pub name: String,
    pub account_group: String,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct StorageAccountKey {
    #[serde(rename = "AccountMonikerName")]
    account_moniker_name: String,
    #[serde(rename = "AccountGroupName")]
    account_group_name: String,
    #[serde(rename = "IsPrimaryMoniker")]
    is_primary_moniker: bool,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
struct GenevaResponse {
    #[serde(rename = "IngestionGatewayInfo")]
    ingestion_gateway_info: IngestionGatewayInfo,
    #[serde(rename = "TagId")]
    tag_id: String,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct ExtendedGenevaResponse {
    #[serde(rename = "IngestionGatewayInfo")]
    ingestion_gateway_info: IngestionGatewayInfo,
    #[serde(rename = "StorageAccountKeys")]
    storage_account_keys: Vec<StorageAccountKey>,
}

#[allow(dead_code)]
struct CachedAuthData {
    token_info: Option<IngestionGatewayInfo>,
    moniker_info: Option<MonikerInfo>,
    token_expiry: Option<DateTime<Utc>>,
}

#[allow(dead_code)]
pub(crate) struct GenevaConfigClient {
    config: GenevaConfigClientConfig,
    http_client: Client,
    cached_data: RwLock<CachedAuthData>,
}

/// Client for interacting with the Geneva Configuration Service.
///
/// This client handles authentication and communication with the Geneva Config
/// API to retrieve configuration information like ingestion endpoints.
impl GenevaConfigClient {
    /// Creates a new Geneva Config Client with the provided configuration.
    ///
    /// # Arguments
    /// * `config` - The client configuration
    ///
    /// # Returns
    /// * `Result<Self>` - A new client instance or an error
    ///
    /// # Errors
    /// * `GenevaConfigClientError::Certificate` - If certificate authentication fails
    /// * `GenevaConfigClientError::Io` - If reading certificate files fails
    /// * `GenevaConfigClientError::Tls` - If TLS configuration fails
    #[allow(dead_code)]
    pub(crate) async fn new(config: GenevaConfigClientConfig) -> Result<Self> {
        let mut client_builder = Client::builder()
            .http1_only()
            .timeout(Duration::from_secs(30))
            .pool_idle_timeout(Duration::from_secs(90))
            .pool_max_idle_per_host(16);

        match &config.auth_method {
            // TODO: Certificate auth would be removed in favor of managed identity.,
            // This is for testing, so we can use self-signed certs, and password in plain text.
            AuthMethod::Certificate { path, password } => {
                // Read the PKCS#12 file
                let p12_bytes = fs::read(path)?;
                let identity = Identity::from_pkcs12(&p12_bytes, password)?;
                let tls_connector =
                    configure_tls_connector(native_tls::TlsConnector::builder(), identity)
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
            cached_data: RwLock::new(CachedAuthData {
                token_info: None,
                moniker_info: None,
                token_expiry: None,
            }),
        })
    }

    fn parse_token_expiry(&self, expiry_str: &str) -> Option<DateTime<Utc>> {
        // Attempt to parse the ISO 8601 datetime string
        DateTime::parse_from_rfc3339(expiry_str)
            .ok()
            .map(|dt| dt.with_timezone(&Utc))
    }

    fn is_token_expired(&self, expiry: DateTime<Utc>) -> bool {
        // Get current time
        let now = Utc::now();

        // Consider token expired if it's within 5 minutes of expiry
        now + chrono::Duration::minutes(5) >= expiry
    }

    #[allow(dead_code)]
    pub(crate) fn get_token_expiry(&self) -> Option<DateTime<Utc>> {
        self.cached_data
            .read()
            .ok()
            .and_then(|data| data.token_expiry)
    }

    /// Retrieves ingestion gateway information from the Geneva Config Service.
    ///
    /// # HTTP API Details
    ///
    /// ## Request
    /// - **Method**: GET
    /// - **Endpoint**: `{base_endpoint}/api/agent/v3/{environment}/{account}/MonitoringStorageKeys/`
    /// - **Query Parameters**:
    ///   - `Namespace`: Service namespace
    ///   - `Region`: Azure region
    ///   - `Identity`: Base64-encoded identity string (format: "Tenant=Default/Role=GcsClient/RoleInstance={agent_identity}")
    ///   - `OSType`: Operating system type (Darwin/Windows/Linux)
    ///   - `ConfigMajorVersion`: Version string (format: "Ver{major_version}v0")
    ///   - `TagId`: UUID for request tracking
    /// - **Headers**:
    ///   - `User-Agent`: "{agent_identity}-{agent_version}"
    ///   - `x-ms-client-request-id`: UUID for request tracking
    ///   - `Accept`: "application/json"
    ///
    /// ## Response
    /// - **Status**: 200 OK on success
    /// - **Content-Type**: application/json
    /// - **Body**:
    ///   ```json
    ///   {
    ///     "IngestionGatewayInfo": {
    ///       "endpoint": "https://ingestion.endpoint.example",
    ///       "AuthToken": "auth-token-value"
    ///     }
    ///   }
    ///   ```
    ///
    /// ## Authentication
    /// Uses mutual TLS (mTLS) with client certificate authentication
    ///
    /// # Returns
    /// * `Result<IngestionGatewayInfo, MonikerInfo>` - Ingestion gateway information, with storage monikers or an error
    ///
    /// # Errors
    /// * `GenevaConfigClientError::Http` - If the HTTP request fails
    /// * `GenevaConfigClientError::RequestFailed` - If the server returns a non-success status
    /// * `GenevaConfigClientError::AuthInfoNotFound` - If the response doesn't contain ingestion info
    /// * `GenevaConfigClientError::SerdeJson` - If JSON parsing fails
    #[allow(dead_code)]
    pub(crate) async fn get_ingestion_info(&self) -> Result<(IngestionGatewayInfo, MonikerInfo)> {
        // First, try to read from cache (shared read access)
        {
            let cached_data = self.cached_data.read().map_err(|_| {
                GenevaConfigClientError::InternalError("RwLock poisoned".to_string())
            })?;

            if let (Some(token_info), Some(moniker_info), Some(expiry)) = (
                &cached_data.token_info,
                &cached_data.moniker_info,
                cached_data.token_expiry,
            ) {
                if !self.is_token_expired(expiry) {
                    return Ok((token_info.clone(), moniker_info.clone()));
                }
            }
        }

        // Cache miss or expired token, fetch fresh data
        // Perform actual fetch before acquiring write lock to minimize lock contention
        let (fresh_token_info, fresh_moniker_info) = self.fetch_ingestion_info().await?;
        let token_expiry = self.parse_token_expiry(&fresh_token_info.auth_token_expiry_time);

        // Now update the cache with exclusive write access
        {
            let mut cached_data = self
                .cached_data
                .write()
                .map_err(|_| GenevaConfigClientError::Certificate("RwLock poisoned".to_string()))?;

            // Double-check in case another thread updated while we were fetching
            if let (Some(existing_expiry), Some(new_expiry)) =
                (cached_data.token_expiry, token_expiry)
            {
                if new_expiry <= existing_expiry {
                    // Another thread already got a newer token, use cached data
                    if let (Some(token), Some(moniker)) =
                        (&cached_data.token_info, &cached_data.moniker_info)
                    {
                        return Ok((token.clone(), moniker.clone()));
                    }
                }
            }

            // Update cache with fresh data
            cached_data.token_info = Some(fresh_token_info.clone());
            cached_data.moniker_info = Some(fresh_moniker_info.clone());
            cached_data.token_expiry = token_expiry;
        }

        Ok((fresh_token_info, fresh_moniker_info))
    }

    /// Internal method that actually fetches data from Geneva Config Service
    async fn fetch_ingestion_info(&self) -> Result<(IngestionGatewayInfo, MonikerInfo)> {
        let agent_identity = "GenevaUploader"; // TODO make this configurable
        let agent_version = "0.1"; // TODO make this configurable
        let identity = format!(
            "Tenant=Default/Role=GcsClient/RoleInstance={}",
            agent_identity
        );

        let encoded_identity = general_purpose::STANDARD.encode(&identity);
        let version_str = format!("Ver{}v0", self.config.config_major_version);

        // Construct the following URL:
        // https://<endpoint>/api/agent/v3/<environment>/<account>/MonitoringStorageKeys/
        //   ?Namespace=<namespace>
        //   &Region=<region>
        //   &Identity=<base64-encoded identity>
        //   &OSType=<os_type>
        //   &ConfigMajorVersion=Ver<major_version>v0
        //   &TagId=<uuid>
        let endpoint = self.config.endpoint.trim_end_matches('/');
        let tag_id = Uuid::new_v4().to_string(); //TODO - uuid is costly, check if counter is enough?
        let mut url = String::with_capacity(endpoint.len() + 200); // Pre-allocate with reasonable capacity
        url.push_str(endpoint);
        url.push_str("/api/agent/v3/");
        url.push_str(&self.config.environment);
        url.push('/');
        url.push_str(&self.config.account);
        url.push_str("/MonitoringStorageKeys/?Namespace=");
        url.push_str(&self.config.namespace);
        url.push_str("&Region=");
        url.push_str(&self.config.region);
        url.push_str("&Identity=");
        url.push_str(&encoded_identity);
        url.push_str("&OSType=");
        url.push_str(get_os_type());
        url.push_str("&ConfigMajorVersion=");
        url.push_str(&version_str);
        url.push_str("&TagId=");
        url.push_str(&tag_id);

        let req_id = Uuid::new_v4().to_string();

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
            let parsed = match serde_json::from_str::<ExtendedGenevaResponse>(&body) {
                Ok(response) => response,
                Err(_) => {
                    return Err(GenevaConfigClientError::AuthInfoNotFound(
                        "No primary diag moniker found in storage accounts".to_string(),
                    ));
                }
            };

            for account in &parsed.storage_account_keys {
                if account.is_primary_moniker && account.account_moniker_name.contains("diag") {
                    let moniker_info = MonikerInfo {
                        name: account.account_moniker_name.clone(),
                        account_group: account.account_group_name.clone(),
                    };

                    return Ok((parsed.ingestion_gateway_info, moniker_info));
                }
            }

            Err(GenevaConfigClientError::MonikerNotFound(
                "No primary diag moniker found in storage accounts".to_string(),
            ))
        } else {
            Err(GenevaConfigClientError::RequestFailed {
                status: status.as_u16(),
                message: body,
            })
        }
    }
}

#[inline]
fn get_os_type() -> &'static str {
    match std::env::consts::OS {
        "macos" => "Darwin",
        "windows" => "Windows",
        "linux" => "Linux",
        other => other,
    }
}

#[cfg(feature = "self_signed_certs")]
fn configure_tls_connector(
    mut builder: native_tls::TlsConnectorBuilder,
    identity: native_tls::Identity,
) -> native_tls::TlsConnectorBuilder {
    eprintln!("WARNING: Self-signed certificates will be accepted. This should only be used in development!");
    builder
        .identity(identity)
        .min_protocol_version(Some(Protocol::Tlsv12))
        .max_protocol_version(Some(Protocol::Tlsv12))
        .danger_accept_invalid_certs(true);
    builder
}

#[cfg(not(feature = "self_signed_certs"))]
fn configure_tls_connector(
    mut builder: native_tls::TlsConnectorBuilder,
    identity: native_tls::Identity,
) -> native_tls::TlsConnectorBuilder {
    builder
        .identity(identity)
        .min_protocol_version(Some(Protocol::Tlsv12))
        .max_protocol_version(Some(Protocol::Tlsv12));
    builder
}
