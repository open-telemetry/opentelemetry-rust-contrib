use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{mpsc, Mutex};
use uuid::Uuid;

#[derive(Debug)]
pub(crate) struct AppState {
    pub(crate) public_base_url: String,
    pub(crate) monitoring_endpoint: String,
    pub(crate) primary_moniker: String,
    pub(crate) account_group: String,
    pub(crate) token_ttl_secs: i64,
    pub(crate) tokens: Mutex<HashMap<String, TokenRecord>>,
    pub(crate) work_tx: mpsc::Sender<AcceptedRequest>,
    pub(crate) db_path: PathBuf,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct TokenRecord {
    pub(crate) token: String,
    pub(crate) issued_at: DateTime<Utc>,
    pub(crate) expires_at: DateTime<Utc>,
    pub(crate) environment: String,
    pub(crate) account: String,
    pub(crate) namespace: String,
    pub(crate) region: String,
    pub(crate) tag_id: String,
}

#[derive(Debug)]
pub(crate) struct AcceptedRequest {
    pub(crate) request_id: Uuid,
    pub(crate) received_at: DateTime<Utc>,
    pub(crate) query: IngestQuery,
    pub(crate) body: Vec<u8>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GcsQuery {
    #[serde(rename = "Namespace")]
    pub(crate) namespace: String,
    #[serde(rename = "Region")]
    pub(crate) region: String,
    #[serde(rename = "Identity")]
    pub(crate) _identity: String,
    #[serde(rename = "OSType")]
    pub(crate) _os_type: String,
    #[serde(rename = "ConfigMajorVersion")]
    pub(crate) _config_major_version: String,
    #[serde(rename = "TagId")]
    pub(crate) tag_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct IngestQuery {
    pub(crate) endpoint: String,
    pub(crate) moniker: String,
    pub(crate) namespace: String,
    pub(crate) event: String,
    pub(crate) version: String,
    #[serde(rename = "sourceUniqueId")]
    pub(crate) source_unique_id: String,
    #[serde(rename = "sourceIdentity")]
    pub(crate) source_identity: String,
    #[serde(rename = "startTime")]
    pub(crate) start_time: String,
    #[serde(rename = "endTime")]
    pub(crate) end_time: String,
    pub(crate) format: String,
    #[serde(rename = "dataSize")]
    pub(crate) data_size: usize,
    #[serde(rename = "minLevel")]
    pub(crate) min_level: u32,
    #[serde(rename = "schemaIds")]
    pub(crate) schema_ids: String,
    #[serde(rename = "rowCount")]
    pub(crate) row_count: usize,
}

#[derive(Debug, Serialize)]
pub(crate) struct IngestionGatewayInfoResponse {
    #[serde(rename = "Endpoint")]
    pub(crate) endpoint: String,
    #[serde(rename = "AuthToken")]
    pub(crate) auth_token: String,
    #[serde(rename = "AuthTokenExpiryTime")]
    pub(crate) auth_token_expiry_time: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct StorageAccountKeyResponse {
    #[serde(rename = "AccountMonikerName")]
    pub(crate) account_moniker_name: String,
    #[serde(rename = "AccountGroupName")]
    pub(crate) account_group_name: String,
    #[serde(rename = "IsPrimaryMoniker")]
    pub(crate) is_primary_moniker: bool,
}

#[derive(Debug, Serialize)]
pub(crate) struct GcsResponse {
    #[serde(rename = "IngestionGatewayInfo")]
    pub(crate) ingestion_gateway_info: IngestionGatewayInfoResponse,
    #[serde(rename = "StorageAccountKeys")]
    pub(crate) storage_account_keys: Vec<StorageAccountKeyResponse>,
    #[serde(rename = "TagId")]
    pub(crate) tag_id: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct IngestResponse {
    pub(crate) ticket: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct RequestsResponse {
    pub(crate) items: Vec<RequestSummary>,
}

#[derive(Debug, Serialize)]
pub(crate) struct RequestSummary {
    pub(crate) request_id: String,
    pub(crate) received_at: String,
    pub(crate) event_name: String,
    pub(crate) namespace: String,
    pub(crate) row_count: i64,
    pub(crate) decode_status: String,
    pub(crate) decode_error: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct RequestDetail {
    pub(crate) request_id: String,
    pub(crate) received_at: String,
    pub(crate) event_name: String,
    pub(crate) namespace: String,
    pub(crate) source_identity: String,
    pub(crate) start_time: String,
    pub(crate) end_time: String,
    pub(crate) row_count: i64,
    pub(crate) decode_status: String,
    pub(crate) decode_error: Option<String>,
    pub(crate) metadata: Option<String>,
    pub(crate) query: Value,
    pub(crate) schemas: Vec<DecodedSchemaRow>,
    pub(crate) records: Vec<DecodedRecordRow>,
}

#[derive(Debug, Serialize)]
pub(crate) struct DecodedSchemaRow {
    pub(crate) schema_id: i64,
    pub(crate) event_name: String,
    pub(crate) struct_name: String,
    pub(crate) qualified_name: String,
    pub(crate) md5: String,
    pub(crate) fields: Value,
}

#[derive(Debug, Serialize)]
pub(crate) struct RecordsResponse {
    pub(crate) items: Vec<DecodedRecordRow>,
}

#[derive(Debug, Serialize)]
pub(crate) struct DecodedRecordRow {
    pub(crate) request_id: String,
    pub(crate) event_name: String,
    pub(crate) schema_id: i64,
    pub(crate) level: i64,
    pub(crate) row_index: i64,
    pub(crate) payload: Value,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ListQuery {
    pub(crate) limit: Option<usize>,
    pub(crate) event: Option<String>,
}

#[derive(Debug)]
pub(crate) struct WorkerHandle {
    pub(crate) _join: std::thread::JoinHandle<()>,
}
