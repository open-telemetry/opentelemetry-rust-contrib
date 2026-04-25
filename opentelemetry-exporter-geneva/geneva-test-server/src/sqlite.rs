use crate::config::ServerConfig;
use crate::decode::central_blob::{decode_central_blob, schema_fields_json};
use crate::decode::lz4::decompress_chunked_lz4;
use crate::models::{AcceptedRequest, AppState, WorkerHandle};
use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use std::path::Path;
use std::sync::{mpsc, Mutex};
use tracing::{error, info};

pub(crate) fn spawn_worker(config: ServerConfig) -> Result<(AppState, WorkerHandle)> {
    if let Some(parent) = config.db_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    let (tx, rx) = mpsc::channel::<AcceptedRequest>();
    let db_path = config.db_path.clone();

    let join = std::thread::Builder::new()
        .name("geneva-test-server-worker".to_string())
        .spawn(move || {
            if let Err(err) = worker_loop(&db_path, rx) {
                error!(error = %err, "geneva-test-server worker exited with error");
            }
        })?;

    let state = AppState {
        public_base_url: config.public_base_url,
        monitoring_endpoint: config.monitoring_endpoint,
        primary_moniker: config.primary_moniker,
        account_group: config.account_group,
        token_ttl_secs: config.token_ttl_secs,
        tokens: Mutex::new(std::collections::HashMap::new()),
        work_tx: tx,
        db_path: config.db_path,
    };

    Ok((state, WorkerHandle { _join: join }))
}

fn worker_loop(db_path: &Path, rx: mpsc::Receiver<AcceptedRequest>) -> Result<()> {
    let mut conn = Connection::open(db_path)?;
    init_db(&mut conn)?;
    info!(db_path = %db_path.display(), "geneva-test-server worker ready");

    for request in rx {
        if let Err(err) = persist_request(&mut conn, request) {
            error!(error = %err, "failed to persist accepted upload");
        }
    }

    Ok(())
}

fn init_db(conn: &mut Connection) -> Result<()> {
    conn.execute_batch(
        "
        PRAGMA journal_mode = WAL;
        PRAGMA foreign_keys = ON;

        CREATE TABLE IF NOT EXISTS ingest_requests (
            request_id TEXT PRIMARY KEY,
            received_at TEXT NOT NULL,
            event_name TEXT NOT NULL,
            namespace TEXT NOT NULL,
            source_identity TEXT NOT NULL,
            start_time TEXT NOT NULL,
            end_time TEXT NOT NULL,
            row_count INTEGER NOT NULL,
            metadata TEXT,
            decode_status TEXT NOT NULL,
            decode_error TEXT,
            query_json TEXT NOT NULL,
            compressed_body BLOB NOT NULL,
            central_blob BLOB
        );

        CREATE TABLE IF NOT EXISTS decoded_schemas (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            request_id TEXT NOT NULL,
            schema_id INTEGER NOT NULL,
            event_name TEXT NOT NULL,
            struct_name TEXT NOT NULL,
            qualified_name TEXT NOT NULL,
            md5 TEXT NOT NULL,
            fields_json TEXT NOT NULL,
            FOREIGN KEY(request_id) REFERENCES ingest_requests(request_id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS decoded_records (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            request_id TEXT NOT NULL,
            event_name TEXT NOT NULL,
            schema_id INTEGER NOT NULL,
            level INTEGER NOT NULL,
            row_index INTEGER NOT NULL,
            payload_json TEXT NOT NULL,
            FOREIGN KEY(request_id) REFERENCES ingest_requests(request_id) ON DELETE CASCADE
        );

        CREATE INDEX IF NOT EXISTS idx_ingest_requests_received_at
            ON ingest_requests(received_at DESC);
        CREATE INDEX IF NOT EXISTS idx_decoded_records_event_name
            ON decoded_records(event_name, id DESC);
        ",
    )?;
    Ok(())
}

fn persist_request(conn: &mut Connection, request: AcceptedRequest) -> Result<()> {
    let request_id = request.request_id.to_string();
    let query_json = serde_json::to_string(&request.query)?;

    let decode_result = (|| -> Result<(Vec<u8>, String, Vec<DecodedSchemaRow>, Vec<DecodedRecordRow>)> {
        let central_blob = decompress_chunked_lz4(&request.body)?;
        let decoded = decode_central_blob(&central_blob)?;
        let schemas = decoded
            .schemas
            .iter()
            .map(|schema| -> Result<DecodedSchemaRow> {
                Ok(DecodedSchemaRow {
                    schema_id: schema.schema_id as i64,
                    event_name: request.query.event.clone(),
                    struct_name: schema.schema.struct_name.clone(),
                    qualified_name: schema.schema.qualified_name.clone(),
                    md5: schema.md5.clone(),
                    fields_json: serde_json::to_string(&schema_fields_json(&schema.schema.fields))?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let records = decoded
            .events
            .iter()
            .enumerate()
            .map(|(index, event)| {
                Ok(DecodedRecordRow {
                    event_name: event.event_name.clone(),
                    schema_id: event.schema_id as i64,
                    level: i64::from(event.level),
                    row_index: index as i64,
                    payload_json: serde_json::to_string(&event.payload)?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok((central_blob, decoded.metadata, schemas, records))
    })();

    let (decode_status, decode_error, metadata, central_blob, schemas, records) =
        match decode_result {
            Ok((central_blob, metadata, schemas, records)) => (
                "decoded".to_string(),
                None,
                Some(metadata),
                Some(central_blob),
                schemas,
                records,
            ),
            Err(err) => (
                "decode_failed".to_string(),
                Some(err.to_string()),
                None,
                None,
                Vec::new(),
                Vec::new(),
            ),
        };

    let tx = conn.transaction()?;
    tx.execute(
        "INSERT OR REPLACE INTO ingest_requests (
            request_id, received_at, event_name, namespace, source_identity, start_time, end_time,
            row_count, metadata, decode_status, decode_error, query_json, compressed_body, central_blob
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        params![
            request_id,
            request.received_at.to_rfc3339(),
            request.query.event,
            request.query.namespace,
            request.query.source_identity,
            request.query.start_time,
            request.query.end_time,
            request.query.row_count as i64,
            metadata,
            decode_status,
            decode_error,
            query_json,
            request.body,
            central_blob,
        ],
    )?;

    for schema in schemas {
        tx.execute(
            "INSERT INTO decoded_schemas (
                request_id, schema_id, event_name, struct_name, qualified_name, md5, fields_json
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                request_id,
                schema.schema_id,
                schema.event_name,
                schema.struct_name,
                schema.qualified_name,
                schema.md5,
                schema.fields_json,
            ],
        )?;
    }

    for record in records {
        tx.execute(
            "INSERT INTO decoded_records (
                request_id, event_name, schema_id, level, row_index, payload_json
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                request_id,
                record.event_name,
                record.schema_id,
                record.level,
                record.row_index,
                record.payload_json,
            ],
        )?;
    }

    tx.commit()?;
    Ok(())
}

struct DecodedSchemaRow {
    schema_id: i64,
    event_name: String,
    struct_name: String,
    qualified_name: String,
    md5: String,
    fields_json: String,
}

struct DecodedRecordRow {
    event_name: String,
    schema_id: i64,
    level: i64,
    row_index: i64,
    payload_json: String,
}
