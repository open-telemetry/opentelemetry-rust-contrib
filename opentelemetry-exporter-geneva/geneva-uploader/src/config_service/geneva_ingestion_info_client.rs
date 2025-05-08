//! Geneva Ingestion Info Client (MonitoringStorageKeys API)

use crate::config_service::common::{
    build_static_headers, get_os_type, initialize_http_client, GenevaConfigClientConfig,
    AGENT_IDENTITY, AGENT_VERSION,
};

use crate::config_service::error::{GenevaConfigClientError, GenevaConfigClientResult};
use base64::{engine::general_purpose, Engine as _};
use chrono::{DateTime, Utc};
use reqwest::{
    header::{HeaderMap, HeaderValue, ACCEPT, USER_AGENT},
    Client,
};
use serde::Deserialize;
use std::fmt;
use std::fmt::Write;
use std::sync::RwLock;
use uuid::Uuid;

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
    #[serde(rename = "StorageAccountKeys", default)]
    storage_account_keys: Vec<StorageAccountKey>,
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
pub(crate) struct GenevaIngestionClient {
    config: GenevaConfigClientConfig,
    http_client: Client,
    cached_data: RwLock<Option<CachedAuthData>>,
    precomputed_url_prefix: String,
    agent_identity: String,
    agent_version: String,
    static_headers: HeaderMap,
}

impl fmt::Debug for GenevaIngestionClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GenevaIngestionClient")
            .field("config", &self.config)
            .field("precomputed_url_prefix", &self.precomputed_url_prefix)
            .field("agent_identity", &self.agent_identity)
            .field("agent_version", &self.agent_version)
            .field("static_headers", &self.static_headers)
            .finish()
    }
}

impl GenevaIngestionClient {
    #[allow(dead_code)]
    pub(crate) fn new(config: GenevaConfigClientConfig) -> GenevaConfigClientResult<Self> {
        let http_client = initialize_http_client(&config.auth_method)?;
        let static_headers = Self::build_static_headers(AGENT_IDENTITY, AGENT_VERSION);

        let identity = format!(
            "Tenant=Default/Role=GcsClient/RoleInstance={}",
            AGENT_IDENTITY
        );
        let encoded_identity = general_purpose::STANDARD.encode(&identity);
        let version_str = format!("Ver{}v0", config.config_major_version);

        let mut pre_url = String::with_capacity(config.endpoint.len() + 200);
        write!(
            &mut pre_url,
            "{}/api/agent/v3/{}/{}/MonitoringStorageKeys/?Namespace={}&Region={}&Identity={}&OSType={}&ConfigMajorVersion={}",
            config.endpoint.trim_end_matches('/'),
            config.environment,
            config.account,
            config.namespace,
            config.region,
            encoded_identity,
            get_os_type(),
            version_str
        ).map_err(|e| GenevaConfigClientError::InternalError(format!("Failed to write URL: {e}")))?;

        Ok(Self {
            config,
            http_client,
            cached_data: RwLock::new(None),
            precomputed_url_prefix: pre_url,
            agent_identity: AGENT_IDENTITY.to_string(),
            agent_version: AGENT_VERSION.to_string(),
            static_headers,
        })
    }

    fn parse_token_expiry(expiry_str: &str) -> Option<DateTime<Utc>> {
        DateTime::parse_from_rfc3339(expiry_str)
            .ok()
            .map(|dt| dt.with_timezone(&Utc))
    }

    fn build_static_headers(agent_identity: &str, agent_version: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        let user_agent = format!("{}-{}", agent_identity, agent_version);
        headers.insert(USER_AGENT, HeaderValue::from_str(&user_agent).unwrap());
        headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
        headers
    }

    #[allow(dead_code)]
    pub(crate) async fn get_ingestion_info(
        &self,
    ) -> GenevaConfigClientResult<(IngestionGatewayInfo, MonikerInfo, String)> {
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

        let (fresh_ingestion_gateway_info, fresh_moniker_info) =
            self.fetch_ingestion_info().await?;

        let token_expiry =
            Self::parse_token_expiry(&fresh_ingestion_gateway_info.auth_token_expiry_time)
                .ok_or_else(|| {
                    GenevaConfigClientError::InternalError("Failed to parse token expiry".into())
                })?;

        let token_endpoint = extract_endpoint_from_token(&fresh_ingestion_gateway_info.auth_token)?;

        let mut guard = self
            .cached_data
            .write()
            .map_err(|_| GenevaConfigClientError::InternalError("RwLock poisoned".to_string()))?;

        if let Some(existing) = guard.as_ref() {
            if existing.token_expiry >= token_expiry {
                return Ok((
                    existing.auth_info.0.clone(),
                    existing.auth_info.1.clone(),
                    existing.token_endpoint.clone(),
                ));
            }
        }
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

    async fn fetch_ingestion_info(
        &self,
    ) -> GenevaConfigClientResult<(IngestionGatewayInfo, MonikerInfo)> {
        let tag_id = Uuid::new_v4().to_string();
        let mut url = String::with_capacity(self.precomputed_url_prefix.len() + 50);
        write!(&mut url, "{}&TagId={}", self.precomputed_url_prefix, tag_id).map_err(|e| {
            GenevaConfigClientError::InternalError(format!("Failed to write URL: {e}"))
        })?;

        let req_id = Uuid::new_v4().to_string();

        let response = self
            .http_client
            .get(&url)
            .headers(self.static_headers.clone())
            .header("x-ms-client-request-id", req_id)
            .send()
            .await?;

        let status = response.status();
        let body = response.text().await?;

        if !status.is_success() {
            return Err(GenevaConfigClientError::RequestFailed {
                status: status.as_u16(),
                message: body,
            });
        }

        let parsed = match serde_json::from_str::<GenevaResponse>(&body) {
            Ok(response) => response,
            Err(e) => {
                return Err(GenevaConfigClientError::AuthInfoNotFound(format!(
                    "Failed to parse response: {}",
                    e
                )));
            }
        };

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
    }
}

fn extract_endpoint_from_token(token: &str) -> GenevaConfigClientResult<String> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err(GenevaConfigClientError::JwtTokenError(
            "Invalid JWT token format".into(),
        ));
    }
    let payload = parts[1];
    let payload = match payload.len() % 4 {
        0 => payload.to_string(),
        2 => format!("{}==", payload),
        3 => format!("{}=", payload),
        _ => payload.to_string(),
    };

    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|e| {
            GenevaConfigClientError::JwtTokenError(format!("Failed to decode JWT: {}", e))
        })?;
    let decoded_str = String::from_utf8(decoded).map_err(|e| {
        GenevaConfigClientError::JwtTokenError(format!("Invalid UTF-8 in JWT: {}", e))
    })?;
    let payload_json: serde_json::Value =
        serde_json::from_str(&decoded_str).map_err(GenevaConfigClientError::SerdeJson)?;
    let endpoint = payload_json["Endpoint"]
        .as_str()
        .ok_or_else(|| {
            GenevaConfigClientError::JwtTokenError("No Endpoint claim in JWT token".to_string())
        })?
        .to_string();
    Ok(endpoint)
}
