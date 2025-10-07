// Geneva Config Client with TLS (PKCS#12) and Azure Workload Identity support

use base64::{engine::general_purpose, Engine as _};
use reqwest::{
    header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION, USER_AGENT},
    Client,
};
use serde::Deserialize;
use std::time::Duration;
use thiserror::Error;
use uuid::Uuid;

use chrono::{DateTime, Utc};
use native_tls::{Identity, Protocol};
use std::fmt;
use std::fmt::Write;
use std::fs;
use std::path::PathBuf;
use std::sync::RwLock;

// Azure Identity imports for MSI and Workload Identity authentication
use azure_core::credentials::TokenCredential;
use azure_identity::{
    ManagedIdentityCredential, ManagedIdentityCredentialOptions, UserAssignedId,
    WorkloadIdentityCredential, WorkloadIdentityCredentialOptions,
};

/// Authentication methods for the Geneva Config Client.
///
/// The client supports three authentication methods:
/// - Certificate-based authentication using PKCS#12 (.p12) files
/// - Azure Workload Identity (Federated Identity) for Kubernetes workloads
/// - Mock authentication for testing
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
#[derive(Clone, Debug)]
pub enum AuthMethod {
    /// Certificate-based authentication
    ///
    /// # Arguments
    /// * `path` - Path to the PKCS#12 (.p12) certificate file
    /// * `password` - Password to decrypt the PKCS#12 file
    Certificate { path: PathBuf, password: String },
    /// System-assigned managed identity (auto-detected)
    SystemManagedIdentity,
    /// User-assigned managed identity by client ID
    UserManagedIdentity { client_id: String },
    /// User-assigned managed identity by object ID
    UserManagedIdentityByObjectId { object_id: String },
    /// User-assigned managed identity by resource ID
    UserManagedIdentityByResourceId { resource_id: String },
    /// Azure Workload Identity authentication (Federated Identity for Kubernetes)
    ///
    /// # Arguments
    /// * `client_id` - Azure AD Application (client) ID
    /// * `tenant_id` - Azure AD Tenant ID
    /// * `token_file` - Optional path to the service account token file.
    ///   If None, defaults to AZURE_FEDERATED_TOKEN_FILE env var
    /// * `resource` - Azure AD resource URI for token acquisition
    ///   (e.g., <https://monitor.azure.com> for Azure Public Cloud)
    WorkloadIdentity {
        client_id: String,
        tenant_id: String,
        token_file: Option<PathBuf>,
        resource: String,
    },
    #[cfg(feature = "mock_auth")]
    MockAuth, // No authentication, used for testing purposes
}

#[derive(Debug, Error)]
pub(crate) enum GenevaConfigClientError {
    // Authentication-related errors
    #[error("Missing Auth Info: {0}")]
    AuthInfoNotFound(String),
    #[error("Invalid or malformed JWT token: {0}")]
    JwtTokenError(String),
    #[error("Certificate error: {0}")]
    Certificate(String),
    #[error("Workload Identity authentication error: {0}")]
    WorkloadIdentityAuth(String),
    #[error("MSI authentication error: {0}")]
    MsiAuth(String),

    // Networking / HTTP / TLS
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("Request failed with status {status}: {message}")]
    RequestFailed { status: u16, message: String },

    // Data / parsing
    #[error("JSON error: {0}")]
    SerdeJson(#[from] serde_json::Error),

    // Misc
    #[error("Moniker not found: {0}")]
    MonikerNotFound(String),
    #[error("Internal error: {0}")]
    InternalError(String),
}

#[allow(dead_code)]
pub(crate) type Result<T> = std::result::Result<T, GenevaConfigClientError>;

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
    pub(crate) msi_resource: Option<String>, // Required when using any Managed Identity variant
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
#[derive(Debug, Deserialize)]
struct GenevaResponse {
    #[serde(rename = "IngestionGatewayInfo")]
    ingestion_gateway_info: IngestionGatewayInfo,
    // TODO: Make storage_account_keys optional since it might not be present in all responses
    #[serde(rename = "StorageAccountKeys", default)]
    storage_account_keys: Vec<StorageAccountKey>,
    // Keep tag_id as it might be used for validation
    #[serde(rename = "TagId")]
    tag_id: String,
}

#[allow(dead_code)]
struct CachedAuthData {
    // Store the complete token and moniker info
    auth_info: (IngestionGatewayInfo, MonikerInfo),
    // Store the endpoint from token for quick access
    token_endpoint: String,
    // Store expiry separately for quick access
    token_expiry: DateTime<Utc>,
}

#[allow(dead_code)]
pub(crate) struct GenevaConfigClient {
    config: GenevaConfigClientConfig,
    http_client: Client,
    // TODO: revisit if the lock can be removed
    cached_data: RwLock<Option<CachedAuthData>>,
    precomputed_url_prefix: String,
    agent_identity: String,
    agent_version: String,
}

impl fmt::Debug for GenevaConfigClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GenevaConfigClient")
            .field("config", &self.config)
            .field("precomputed_url_prefix", &self.precomputed_url_prefix)
            .field("agent_identity", &self.agent_identity)
            .field("agent_version", &self.agent_version)
            .finish()
    }
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
    /// * `GenevaConfigClientError::Certificate` - If reading the certificate file, parsing it, or constructing the TLS connector fails
    /// * `GenevaConfigClientError::AuthMethodNotImplemented` - If the specified authentication method is not yet supported
    #[allow(dead_code)]
    pub(crate) fn new(config: GenevaConfigClientConfig) -> Result<Self> {
        let agent_identity = "GenevaUploader";
        let agent_version = "0.1";

        let mut client_builder = Client::builder()
            .http1_only()
            .timeout(Duration::from_secs(30)) //TODO - make this configurable
            .default_headers(Self::build_static_headers(agent_identity, agent_version));

        match &config.auth_method {
            // TODO: Certificate auth would be removed in favor of managed identity.,
            // This is for testing, so we can use self-signed certs, and password in plain text.
            AuthMethod::Certificate { path, password } => {
                // Read the PKCS#12 file
                let p12_bytes = fs::read(path)
                    .map_err(|e| GenevaConfigClientError::Certificate(e.to_string()))?;
                let identity = Identity::from_pkcs12(&p12_bytes, password)
                    .map_err(|e| GenevaConfigClientError::Certificate(e.to_string()))?;
                //TODO - use use_native_tls instead of preconfigured_tls once we no longer need self-signed certs
                // and TLS 1.2 as the exclusive protocol.
                let tls_connector =
                    configure_tls_connector(native_tls::TlsConnector::builder(), identity)
                        .build()
                        .map_err(|e| GenevaConfigClientError::Certificate(e.to_string()))?;
                client_builder = client_builder.use_preconfigured_tls(tls_connector);
            }
            AuthMethod::WorkloadIdentity { .. } => {
                // No special HTTP client configuration needed for Workload Identity
                // Authentication is done via Bearer token in request headers
            }
            AuthMethod::SystemManagedIdentity
            | AuthMethod::UserManagedIdentity { .. }
            | AuthMethod::UserManagedIdentityByObjectId { .. }
            | AuthMethod::UserManagedIdentityByResourceId { .. } => { /* no special HTTP client changes needed */
            }
            #[cfg(feature = "mock_auth")]
            AuthMethod::MockAuth => {
                // Mock authentication for testing purposes, no actual auth needed
                // Just use the default client builder
                eprintln!("WARNING: Using MockAuth for GenevaConfigClient. This should only be used in tests!");
            }
        }

        let identity = format!("Tenant=Default/Role=GcsClient/RoleInstance={agent_identity}");

        let encoded_identity = general_purpose::STANDARD.encode(&identity);
        let version_str = format!("Ver{0}v0", config.config_major_version);

        // Use different API endpoints based on authentication method
        // Certificate auth uses "api", MSI auth and Workload Identity use "userapi"
        let api_path = match &config.auth_method {
            AuthMethod::Certificate { .. } => "api",
            AuthMethod::SystemManagedIdentity
            | AuthMethod::UserManagedIdentity { .. }
            | AuthMethod::UserManagedIdentityByObjectId { .. }
            | AuthMethod::UserManagedIdentityByResourceId { .. }
            | AuthMethod::WorkloadIdentity { .. } => "userapi",
            #[cfg(feature = "mock_auth")]
            AuthMethod::MockAuth => "api", // treat mock like certificate path for URL shape
        };

        let mut pre_url = String::with_capacity(config.endpoint.len() + 200);
        write!(
            &mut pre_url,
            "{}/{}/agent/v3/{}/{}/MonitoringStorageKeys/?Namespace={}&Region={}&Identity={}&OSType={}&ConfigMajorVersion={}",
            config.endpoint.trim_end_matches('/'),
            api_path,
            config.environment,
            config.account,
            config.namespace,
            config.region,
            encoded_identity,
            get_os_type(),
            version_str
        ).map_err(|e| GenevaConfigClientError::InternalError(format!("Failed to write URL: {e}")))?;

        let http_client = client_builder.build()?;

        Ok(Self {
            config,
            http_client,
            cached_data: RwLock::new(None),
            precomputed_url_prefix: pre_url,
            agent_identity: agent_identity.to_string(), // TODO make this configurable
            agent_version: "1.0".to_string(),           // TODO make this configurable
        })
    }

    fn parse_token_expiry(expiry_str: &str) -> Option<DateTime<Utc>> {
        // Attempt to parse the ISO 8601 datetime string
        DateTime::parse_from_rfc3339(expiry_str)
            .ok()
            .map(|dt| dt.with_timezone(&Utc))
    }

    fn build_static_headers(agent_identity: &str, agent_version: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        let user_agent = format!("{agent_identity}-{agent_version}");
        headers.insert(USER_AGENT, HeaderValue::from_str(&user_agent).unwrap());
        headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
        headers
    }

    /// Get Azure AD token using Workload Identity (Federated Identity)
    async fn get_workload_identity_token(&self) -> Result<String> {
        let (client_id, tenant_id, token_file, resource) =
            match &self.config.auth_method {
                AuthMethod::WorkloadIdentity {
                    client_id,
                    tenant_id,
                    token_file,
                    resource,
                } => (client_id, tenant_id, token_file, resource),
                _ => return Err(GenevaConfigClientError::WorkloadIdentityAuth(
                    "get_workload_identity_token called but auth method is not WorkloadIdentity"
                        .to_string(),
                )),
            };

        // TODO: Extract scope generation logic into helper function shared with get_msi_token()
        let base = resource.trim_end_matches("/.default").trim_end_matches('/');
        let mut scope_candidates: Vec<String> = vec![format!("{base}/.default"), base.to_string()];
        if !base.ends_with('/') {
            scope_candidates.push(format!("{base}/"));
        }

        // TODO: Consider caching WorkloadIdentityCredential if profiling shows credential creation overhead
        let options = WorkloadIdentityCredentialOptions {
            client_id: Some(client_id.clone()),
            tenant_id: Some(tenant_id.clone()),
            token_file_path: token_file.clone(),
            ..Default::default()
        };

        let credential = WorkloadIdentityCredential::new(Some(options)).map_err(|e| {
            GenevaConfigClientError::WorkloadIdentityAuth(format!(
                "Failed to create WorkloadIdentityCredential: {e}"
            ))
        })?;

        let mut last_err: Option<String> = None;
        for scope in &scope_candidates {
            match credential.get_token(&[scope.as_str()], None).await {
                Ok(token) => return Ok(token.token.secret().to_string()),
                Err(e) => last_err = Some(e.to_string()),
            }
        }

        let detail = last_err.unwrap_or_else(|| "no error detail".into());
        Err(GenevaConfigClientError::WorkloadIdentityAuth(format!(
            "Workload Identity token acquisition failed. Scopes tried: {scopes}. Last error: {detail}",
            scopes = scope_candidates.join(", ")
        )))
    }

    /// Get MSI token for GCS authentication
    async fn get_msi_token(&self) -> Result<String> {
        let resource = self.config.msi_resource.as_ref().ok_or_else(|| {
            GenevaConfigClientError::MsiAuth(
                "msi_resource not set in config (required for Managed Identity auth)".to_string(),
            )
        })?;

        // TODO: Extract scope generation logic into helper function shared with get_workload_identity_token()
        let base = resource.trim_end_matches("/.default").trim_end_matches('/');
        let mut scope_candidates: Vec<String> = vec![format!("{base}/.default"), base.to_string()];
        if !base.ends_with('/') {
            scope_candidates.push(format!("{base}/"));
        }

        let user_assigned_id = match &self.config.auth_method {
            AuthMethod::SystemManagedIdentity => None,
            AuthMethod::UserManagedIdentity { client_id } => {
                Some(UserAssignedId::ClientId(client_id.clone()))
            }
            AuthMethod::UserManagedIdentityByObjectId { object_id } => {
                Some(UserAssignedId::ObjectId(object_id.clone()))
            }
            AuthMethod::UserManagedIdentityByResourceId { resource_id } => {
                Some(UserAssignedId::ResourceId(resource_id.clone()))
            }
            _ => {
                return Err(GenevaConfigClientError::MsiAuth(
                    "get_msi_token called but auth method is not a managed identity variant"
                        .to_string(),
                ))
            }
        };

        // TODO: Consider caching ManagedIdentityCredential if profiling shows credential creation overhead
        let options = ManagedIdentityCredentialOptions {
            user_assigned_id,
            ..Default::default()
        };
        let credential = ManagedIdentityCredential::new(Some(options)).map_err(|e| {
            GenevaConfigClientError::MsiAuth(format!("Failed to create MSI credential: {e}"))
        })?;

        let mut last_err: Option<String> = None;
        for scope in &scope_candidates {
            match credential.get_token(&[scope.as_str()], None).await {
                Ok(token) => return Ok(token.token.secret().to_string()),
                Err(e) => last_err = Some(e.to_string()),
            }
        }

        let detail = last_err.unwrap_or_else(|| "no error detail".into());
        Err(GenevaConfigClientError::MsiAuth(format!(
            "Managed Identity token acquisition failed. Scopes tried: {scopes}. Last error: {detail}. IMDS fallback intentionally disabled.",
            scopes = scope_candidates.join(", ")
        )))
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
    pub(crate) async fn get_ingestion_info(
        &self,
    ) -> Result<(IngestionGatewayInfo, MonikerInfo, String)> {
        // First, try to read from cache (shared read access)
        if let Ok(guard) = self.cached_data.read() {
            if let Some(cached_data) = guard.as_ref() {
                let expiry = cached_data.token_expiry;
                if expiry > Utc::now() + chrono::Duration::minutes(5) {
                    return Ok((
                        cached_data.auth_info.0.clone(),
                        cached_data.auth_info.1.clone(),
                        cached_data.token_endpoint.clone(),
                    ));
                }
            }
        }
        // Cache miss or expired token, fetch fresh data
        // Perform actual fetch before acquiring write lock to minimize lock contention
        let (fresh_ingestion_gateway_info, fresh_moniker_info) =
            self.fetch_ingestion_info().await?;

        let token_expiry =
            Self::parse_token_expiry(&fresh_ingestion_gateway_info.auth_token_expiry_time)
                .ok_or_else(|| {
                    GenevaConfigClientError::InternalError("Failed to parse token expiry".into())
                })?;

        let token_endpoint =
            match extract_endpoint_from_token(&fresh_ingestion_gateway_info.auth_token) {
                Ok(ep) => ep,
                Err(err) => {
                    // Fallback: some tokens legitimately omit the Endpoint claim; use server endpoint.
                    #[cfg(debug_assertions)]
                    eprintln!("[geneva][debug] token Endpoint claim missing or unparsable: {err}");
                    fresh_ingestion_gateway_info.endpoint.clone()
                }
            };

        // Now update the cache with exclusive write access
        let mut guard = self
            .cached_data
            .write()
            .map_err(|_| GenevaConfigClientError::InternalError("RwLock poisoned".to_string()))?;

        // Double-check in case another thread updated while we were fetching
        if let Some(existing) = guard.as_ref() {
            if existing.token_expiry >= token_expiry {
                return Ok((
                    existing.auth_info.0.clone(),
                    existing.auth_info.1.clone(),
                    existing.token_endpoint.clone(),
                ));
            }
        }
        // Update with fresh data
        *guard = Some(CachedAuthData {
            auth_info: (
                fresh_ingestion_gateway_info.clone(),
                fresh_moniker_info.clone(),
            ),
            token_endpoint: token_endpoint.clone(),
            token_expiry,
        });

        Ok((
            fresh_ingestion_gateway_info,
            fresh_moniker_info,
            token_endpoint,
        ))
    }

    /// Internal method that actually fetches data from Geneva Config Service
    async fn fetch_ingestion_info(&self) -> Result<(IngestionGatewayInfo, MonikerInfo)> {
        let tag_id = Uuid::new_v4().to_string(); // TODO: consider cheaper counter if perf-critical
        let mut url = String::with_capacity(self.precomputed_url_prefix.len() + 50);
        write!(&mut url, "{}&TagId={tag_id}", self.precomputed_url_prefix).map_err(|e| {
            GenevaConfigClientError::InternalError(format!("Failed to write URL: {e}"))
        })?;

        let req_id = Uuid::new_v4().to_string();

        let mut request = self.http_client.get(&url);

        request = request.header("x-ms-client-request-id", req_id);

        // Add appropriate authentication header
        match &self.config.auth_method {
            AuthMethod::WorkloadIdentity { .. } => {
                let token = self.get_workload_identity_token().await?;
                request = request.header(AUTHORIZATION, format!("Bearer {}", token));
            }
            AuthMethod::SystemManagedIdentity
            | AuthMethod::UserManagedIdentity { .. }
            | AuthMethod::UserManagedIdentityByObjectId { .. }
            | AuthMethod::UserManagedIdentityByResourceId { .. } => {
                let token = self.get_msi_token().await?;
                request = request.header(AUTHORIZATION, format!("Bearer {}", token));
            }
            AuthMethod::Certificate { .. } => { /* mTLS only */ }
            #[cfg(feature = "mock_auth")]
            AuthMethod::MockAuth => { /* no auth header */ }
        }

        // Send HTTP request
        let response = match request.send().await {
            Ok(resp) => resp,
            Err(e) => return Err(GenevaConfigClientError::Http(e)),
        };

        let status = response.status();
        let body = response.text().await?;

        if status.is_success() {
            let parsed = serde_json::from_str::<GenevaResponse>(&body).map_err(|e| {
                GenevaConfigClientError::AuthInfoNotFound(format!("Failed to parse response: {e}"))
            })?;

            for account in parsed.storage_account_keys {
                if account.is_primary_moniker && account.account_moniker_name.contains("diag") {
                    let moniker_info = MonikerInfo {
                        name: account.account_moniker_name,
                        account_group: account.account_group_name,
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

fn extract_endpoint_from_token(token: &str) -> Result<String> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err(GenevaConfigClientError::JwtTokenError(
            "Invalid JWT token format".into(),
        ));
    }

    // Base64-decode the JWT payload (2nd segment of the token).
    // Some JWTs may omit padding ('='), so we restore it manually to ensure valid Base64.
    // This is necessary because the decoder (URL_SAFE_NO_PAD) expects properly padded input.
    let payload = parts[1];
    let payload = match payload.len() % 4 {
        0 => payload.to_string(),
        2 => format!("{payload}=="),
        3 => format!("{payload}="),
        1 => {
            return Err(GenevaConfigClientError::JwtTokenError(
                "Invalid JWT payload length".into(),
            ))
        }
        _ => payload.to_string(),
    };

    // Decode the Base64-encoded payload into raw bytes.
    // Try URL-safe (with and without padding), then fall back to standard Base64.
    let decoded = match general_purpose::URL_SAFE_NO_PAD.decode(&payload) {
        Ok(b) => b,
        Err(e_url_no_pad) => match general_purpose::URL_SAFE.decode(&payload) {
            Ok(b) => b,
            Err(e_url_pad) => match general_purpose::STANDARD.decode(&payload) {
                Ok(b) => b,
                Err(e_std) => {
                    return Err(GenevaConfigClientError::JwtTokenError(format!(
                        "Failed to decode JWT (URL_SAFE_NO_PAD, URL_SAFE, and STANDARD): \
                         no_pad_err={e_url_no_pad}; pad_err={e_url_pad}; std_err={e_std}"
                    )))
                }
            },
        },
    };

    // Convert the raw bytes into a UTF-8 string
    let decoded_str = String::from_utf8(decoded).map_err(|e| {
        GenevaConfigClientError::JwtTokenError(format!("Invalid UTF-8 in JWT: {e}"))
    })?;

    // Parse as JSON and extract the Endpoint claim
    let payload_json: serde_json::Value =
        serde_json::from_str(&decoded_str).map_err(GenevaConfigClientError::SerdeJson)?;

    if let Some(ep) = payload_json["Endpoint"].as_str() {
        return Ok(ep.to_string());
    }
    Err(GenevaConfigClientError::JwtTokenError(
        "No Endpoint claim in JWT token".to_string(),
    ))
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
