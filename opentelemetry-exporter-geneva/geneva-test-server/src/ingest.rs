use crate::models::{
    AcceptedRequest, AppState, DecodedRecordRow, ListQuery, RecordsResponse, RequestDetail,
    RequestSummary, RequestsResponse,
};
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{header::AUTHORIZATION, HeaderMap, StatusCode};
use axum::Json;
use chrono::Utc;
use rusqlite::{params, Connection};
use std::sync::Arc;
use uuid::Uuid;

use crate::models::{IngestQuery, IngestResponse};

pub(crate) async fn ingest(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<IngestQuery>,
    body: Bytes,
) -> Result<(StatusCode, Json<IngestResponse>), (StatusCode, String)> {
    validate_bearer_token(&state, &headers)?;

    if query.format != "centralbond/lz4hc" {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("unsupported format '{}'", query.format),
        ));
    }
    if query.endpoint != state.monitoring_endpoint {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "unexpected monitoring endpoint '{}', expected '{}'",
                query.endpoint, state.monitoring_endpoint
            ),
        ));
    }
    if query.moniker != state.primary_moniker {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "unexpected moniker '{}', expected '{}'",
                query.moniker, state.primary_moniker
            ),
        ));
    }
    if query.data_size != body.len() {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "dataSize {} does not match body length {}",
                query.data_size,
                body.len()
            ),
        ));
    }
    if body.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "empty request body".to_string()));
    }

    let request_id = Uuid::new_v4();
    let accepted = AcceptedRequest {
        request_id,
        received_at: Utc::now(),
        query,
        body: body.to_vec(),
    };

    state.work_tx.send(accepted).map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "worker queue closed".to_string(),
        )
    })?;

    Ok((
        StatusCode::ACCEPTED,
        Json(IngestResponse {
            ticket: request_id.to_string(),
        }),
    ))
}

pub(crate) async fn get_requests(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ListQuery>,
) -> Result<Json<RequestsResponse>, (StatusCode, String)> {
    let limit = query.limit.unwrap_or(50).min(500) as i64;
    let conn = open_db(&state)?;

    let sql = if query.event.is_some() {
        "SELECT request_id, received_at, event_name, namespace, row_count, decode_status, decode_error
         FROM ingest_requests
         WHERE event_name = ?1
         ORDER BY received_at DESC
         LIMIT ?2"
    } else {
        "SELECT request_id, received_at, event_name, namespace, row_count, decode_status, decode_error
         FROM ingest_requests
         ORDER BY received_at DESC
         LIMIT ?1"
    };

    let items = if let Some(event_name) = query.event {
        let mut stmt = conn.prepare(sql).map_err(sqlite_error)?;
        let rows = stmt
            .query_map(params![event_name, limit], |row| {
                Ok(RequestSummary {
                    request_id: row.get(0)?,
                    received_at: row.get(1)?,
                    event_name: row.get(2)?,
                    namespace: row.get(3)?,
                    row_count: row.get(4)?,
                    decode_status: row.get(5)?,
                    decode_error: row.get(6)?,
                })
            })
            .map_err(sqlite_error)?;
        collect_rows(rows).map_err(sqlite_error)?
    } else {
        let mut stmt = conn.prepare(sql).map_err(sqlite_error)?;
        let rows = stmt
            .query_map(params![limit], |row| {
                Ok(RequestSummary {
                    request_id: row.get(0)?,
                    received_at: row.get(1)?,
                    event_name: row.get(2)?,
                    namespace: row.get(3)?,
                    row_count: row.get(4)?,
                    decode_status: row.get(5)?,
                    decode_error: row.get(6)?,
                })
            })
            .map_err(sqlite_error)?;
        collect_rows(rows).map_err(sqlite_error)?
    };

    Ok(Json(RequestsResponse { items }))
}

pub(crate) async fn get_request_detail(
    State(state): State<Arc<AppState>>,
    Path(request_id): Path<String>,
) -> Result<Json<RequestDetail>, (StatusCode, String)> {
    let conn = open_db(&state)?;

    let mut stmt = conn
        .prepare(
            "SELECT request_id, received_at, event_name, namespace, source_identity, start_time, end_time,
                    row_count, decode_status, decode_error, metadata, query_json
             FROM ingest_requests
             WHERE request_id = ?1",
        )
        .map_err(sqlite_error)?;

    let detail = stmt
        .query_row(params![request_id], |row| {
            let query_json: String = row.get(11)?;
            let query = serde_json::from_str(&query_json).unwrap_or(serde_json::Value::Null);
            Ok(RequestDetail {
                request_id: row.get(0)?,
                received_at: row.get(1)?,
                event_name: row.get(2)?,
                namespace: row.get(3)?,
                source_identity: row.get(4)?,
                start_time: row.get(5)?,
                end_time: row.get(6)?,
                row_count: row.get(7)?,
                decode_status: row.get(8)?,
                decode_error: row.get(9)?,
                metadata: row.get(10)?,
                query,
                schemas: Vec::new(),
                records: Vec::new(),
            })
        })
        .map_err(|err| match err {
            rusqlite::Error::QueryReturnedNoRows => {
                (StatusCode::NOT_FOUND, "request not found".to_string())
            }
            _ => sqlite_error(err),
        })?;

    let schemas = {
        let mut stmt = conn
            .prepare(
                "SELECT schema_id, event_name, struct_name, qualified_name, md5, fields_json
                 FROM decoded_schemas
                 WHERE request_id = ?1
                 ORDER BY schema_id ASC",
            )
            .map_err(sqlite_error)?;
        let rows = stmt
            .query_map(params![detail.request_id.clone()], |row| {
                let fields_json: String = row.get(5)?;
                Ok(crate::models::DecodedSchemaRow {
                    schema_id: row.get(0)?,
                    event_name: row.get(1)?,
                    struct_name: row.get(2)?,
                    qualified_name: row.get(3)?,
                    md5: row.get(4)?,
                    fields: serde_json::from_str(&fields_json).unwrap_or(serde_json::Value::Null),
                })
            })
            .map_err(sqlite_error)?;
        collect_rows(rows).map_err(sqlite_error)?
    };

    let records = {
        let mut stmt = conn
            .prepare(
                "SELECT request_id, event_name, schema_id, level, row_index, payload_json
                 FROM decoded_records
                 WHERE request_id = ?1
                 ORDER BY row_index ASC",
            )
            .map_err(sqlite_error)?;
        let rows = stmt
            .query_map(params![detail.request_id.clone()], |row| {
                let payload_json: String = row.get(5)?;
                Ok(DecodedRecordRow {
                    request_id: row.get(0)?,
                    event_name: row.get(1)?,
                    schema_id: row.get(2)?,
                    level: row.get(3)?,
                    row_index: row.get(4)?,
                    payload: serde_json::from_str(&payload_json).unwrap_or(serde_json::Value::Null),
                })
            })
            .map_err(sqlite_error)?;
        collect_rows(rows).map_err(sqlite_error)?
    };

    Ok(Json(RequestDetail {
        schemas,
        records,
        ..detail
    }))
}

pub(crate) async fn get_records(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ListQuery>,
) -> Result<Json<RecordsResponse>, (StatusCode, String)> {
    let limit = query.limit.unwrap_or(100).min(1000) as i64;
    let conn = open_db(&state)?;

    let sql = if query.event.is_some() {
        "SELECT request_id, event_name, schema_id, level, row_index, payload_json
         FROM decoded_records
         WHERE event_name = ?1
         ORDER BY id DESC
         LIMIT ?2"
    } else {
        "SELECT request_id, event_name, schema_id, level, row_index, payload_json
         FROM decoded_records
         ORDER BY id DESC
         LIMIT ?1"
    };

    let items = if let Some(event_name) = query.event {
        let mut stmt = conn.prepare(sql).map_err(sqlite_error)?;
        let rows = stmt
            .query_map(params![event_name, limit], |row| {
                let payload_json: String = row.get(5)?;
                Ok(DecodedRecordRow {
                    request_id: row.get(0)?,
                    event_name: row.get(1)?,
                    schema_id: row.get(2)?,
                    level: row.get(3)?,
                    row_index: row.get(4)?,
                    payload: serde_json::from_str(&payload_json).unwrap_or(serde_json::Value::Null),
                })
            })
            .map_err(sqlite_error)?;
        collect_rows(rows).map_err(sqlite_error)?
    } else {
        let mut stmt = conn.prepare(sql).map_err(sqlite_error)?;
        let rows = stmt
            .query_map(params![limit], |row| {
                let payload_json: String = row.get(5)?;
                Ok(DecodedRecordRow {
                    request_id: row.get(0)?,
                    event_name: row.get(1)?,
                    schema_id: row.get(2)?,
                    level: row.get(3)?,
                    row_index: row.get(4)?,
                    payload: serde_json::from_str(&payload_json).unwrap_or(serde_json::Value::Null),
                })
            })
            .map_err(sqlite_error)?;
        collect_rows(rows).map_err(sqlite_error)?
    };

    Ok(Json(RecordsResponse { items }))
}

fn validate_bearer_token(
    state: &Arc<AppState>,
    headers: &HeaderMap,
) -> Result<(), (StatusCode, String)> {
    let auth_token = extract_bearer_token(headers)?;
    let now = Utc::now();

    let mut tokens = state.tokens.lock().map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "token store poisoned".to_string(),
        )
    })?;
    tokens.retain(|_, value| value.expires_at > now);

    match tokens.get(&auth_token) {
        Some(record) if record.expires_at > now => Ok(()),
        Some(_) => Err((StatusCode::UNAUTHORIZED, "token expired".to_string())),
        None => Err((StatusCode::UNAUTHORIZED, "unknown token".to_string())),
    }
}

fn extract_bearer_token(headers: &HeaderMap) -> Result<String, (StatusCode, String)> {
    let auth = headers.get(AUTHORIZATION).ok_or((
        StatusCode::UNAUTHORIZED,
        "missing Authorization header".to_string(),
    ))?;
    let auth = auth.to_str().map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "invalid Authorization header".to_string(),
        )
    })?;
    auth.strip_prefix("Bearer ").map(str::to_string).ok_or((
        StatusCode::UNAUTHORIZED,
        "expected Bearer token".to_string(),
    ))
}

fn open_db(state: &Arc<AppState>) -> Result<Connection, (StatusCode, String)> {
    Connection::open(&state.db_path).map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to open db: {err}"),
        )
    })
}

fn sqlite_error(err: rusqlite::Error) -> (StatusCode, String) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("sqlite error: {err}"),
    )
}

fn collect_rows<T>(
    rows: rusqlite::MappedRows<'_, impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>>,
) -> rusqlite::Result<Vec<T>> {
    rows.collect()
}
