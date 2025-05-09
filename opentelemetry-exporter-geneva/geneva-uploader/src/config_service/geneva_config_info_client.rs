//! Geneva Configuration XML Client (MonitoringConfiguration API)

use crate::config_service::error::{GenevaConfigClientError, GenevaConfigClientResult};

use crate::config_service::common::{
    build_static_headers, get_os_type, initialize_http_client, GenevaConfigClientConfig,
    AGENT_IDENTITY, AGENT_VERSION,
};
use base64::{engine::general_purpose, Engine as _};

use flate2::read::GzDecoder;
use reqwest::{header::HeaderMap, Client};
use serde::Deserialize;
use std::fmt;
use std::fmt::Write;
use std::io::Read;

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct GcsConfiguration {
    #[serde(rename = "Md5Hash")]
    pub md5_hash: String,
    #[serde(rename = "ConfigurationXml")]
    pub configuration_xml: String,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct GcsConfigurationNotFound {
    #[serde(rename = "LatestConfigVersionFound")]
    pub latest_config_version_found: String,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum GcsConfigurationAPIResponse {
    Config(GcsConfiguration),
    NotFound(GcsConfigurationNotFound),
}

#[allow(dead_code)]
pub(crate) struct GenevaConfigXmlClient {
    config: GenevaConfigClientConfig,
    http_client: Client,
    agent_identity: String,
    agent_version: String,
    static_headers: HeaderMap,
}

impl fmt::Debug for GenevaConfigXmlClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GenevaConfigXmlClient")
            .field("config", &self.config)
            .field("agent_identity", &self.agent_identity)
            .field("agent_version", &self.agent_version)
            .field("static_headers", &self.static_headers)
            .finish()
    }
}

impl GenevaConfigXmlClient {
    #[allow(dead_code)]
    pub(crate) fn new(config: GenevaConfigClientConfig) -> GenevaConfigClientResult<Self> {
        let http_client = initialize_http_client(&config.auth_method)?;

        let static_headers = build_static_headers(AGENT_IDENTITY, AGENT_VERSION);

        Ok(Self {
            config,
            http_client,
            agent_identity: AGENT_IDENTITY.to_string(),
            agent_version: AGENT_VERSION.to_string(),
            static_headers,
        })
    }

    #[allow(dead_code)]
    pub(crate) async fn fetch_gcs_config(
        &self,
        namespace: &str,
        config_major_version: u32,
        config_minor_version: u32,
    ) -> GenevaConfigClientResult<GcsConfiguration> {
        let mut url = String::with_capacity(self.config.endpoint.len() + 100);

        let version_str = format!("Ver{}v0.{}", config_major_version, config_minor_version);
        write!(
            &mut url,
            "{}/api/agent/v3/{}/{}/MonitoringConfiguration/?Namespace={}&Version={}&OSType={}",
            self.config.endpoint.trim_end_matches('/'),
            self.config.environment,
            self.config.account,
            namespace,
            version_str,
            get_os_type(),
        )
        .expect("Failed to write URL");

        let response = self
            .http_client
            .get(&url)
            .headers(self.static_headers.clone())
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

        let parsed: GcsConfigurationAPIResponse = serde_json::from_str(&body)?;

        match parsed {
            GcsConfigurationAPIResponse::Config(cfg) => Ok(cfg),
            GcsConfigurationAPIResponse::NotFound(nf) => {
                Err(GenevaConfigClientError::InternalError(format!(
                    "GCS config not found, latest version: {}",
                    nf.latest_config_version_found
                )))
            }
        }
    }

    #[allow(dead_code)]
    /// Decodes and decompresses the base64/gzip config XML string.
    pub(crate) fn decode_gcs_config_xml(encoded: &str) -> crate::Result<String> {
        let decoded = general_purpose::STANDARD.decode(encoded).map_err(|e| {
            GenevaConfigClientError::InternalError(format!("Base64 decode failed: {e}"))
        })?;
        let mut gz = GzDecoder::new(&decoded[..]);
        let mut out = String::new();
        gz.read_to_string(&mut out).map_err(|e| {
            GenevaConfigClientError::InternalError(format!("Gzip decode failed: {e}"))
        })?;
        Ok(out)
    }
}
