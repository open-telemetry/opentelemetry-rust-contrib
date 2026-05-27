use crate::models::{
    AppState, GcsQuery, GcsResponse, IngestionGatewayInfoResponse, StorageAccountKeyResponse,
    TokenRecord,
};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::{SecondsFormat, Utc};
use std::sync::Arc;
use uuid::Uuid;

pub(crate) async fn healthz() -> &'static str {
    "ok"
}

pub(crate) async fn handle_gcs_request(
    State(state): State<Arc<AppState>>,
    Path((environment, account)): Path<(String, String)>,
    Query(query): Query<GcsQuery>,
) -> Result<Json<GcsResponse>, (StatusCode, String)> {
    let now = Utc::now();
    let expires_at = now + chrono::TimeDelta::seconds(state.token_ttl_secs);
    let token = Uuid::new_v4().to_string();
    let tag_id = query.tag_id.unwrap_or_else(|| Uuid::new_v4().to_string());

    let record = TokenRecord {
        token: token.clone(),
        issued_at: now,
        expires_at,
        environment,
        account,
        namespace: query.namespace,
        region: query.region,
        tag_id: tag_id.clone(),
    };

    let mut tokens = state.tokens.lock().map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "token store poisoned".to_string(),
        )
    })?;
    tokens.insert(token.clone(), record);
    tokens.retain(|_, value| value.expires_at > now);

    let response = GcsResponse {
        ingestion_gateway_info: IngestionGatewayInfoResponse {
            endpoint: state.public_base_url.clone(),
            auth_token: token,
            auth_token_expiry_time: expires_at.to_rfc3339_opts(SecondsFormat::Secs, true),
        },
        storage_account_keys: vec![StorageAccountKeyResponse {
            account_moniker_name: state.primary_moniker.clone(),
            account_group_name: state.account_group.clone(),
            is_primary_moniker: true,
        }],
        tag_id,
    };

    Ok(Json(response))
}
