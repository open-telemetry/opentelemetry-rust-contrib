mod app;
mod config;
mod decode;
mod gcs;
mod ingest;
mod models;
mod sqlite;

use crate::app::build_router;
use crate::config::ServerConfig;
use crate::sqlite::spawn_worker;
use std::sync::Arc;
use tracing::info;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();

    let config = ServerConfig::from_env()?;
    let (state, worker) = spawn_worker(config.clone())?;
    let app = build_router(Arc::new(state));
    let listener = tokio::net::TcpListener::bind(config.listen_addr).await?;

    info!(
        listen_addr = %config.listen_addr,
        public_base_url = %config.public_base_url,
        db_path = %config.db_path.display(),
        "geneva-test-server listening"
    );

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    drop(worker);
    Ok(())
}

fn init_tracing() {
    let filter = std::env::var("RUST_LOG")
        .unwrap_or_else(|_| "geneva_test_server=info,tower_http=info".to_string());
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .compact()
        .init();
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        use tokio::signal::unix::{signal, SignalKind};

        if let Ok(mut sigterm) = signal(SignalKind::terminate()) {
            sigterm.recv().await;
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use geneva_uploader::{AuthMethod, GenevaClient, GenevaClientConfig};
    use opentelemetry_proto::tonic::collector::logs::v1::ExportLogsServiceRequest;
    use opentelemetry_proto::tonic::common::v1::{any_value::Value, AnyValue, KeyValue};
    use opentelemetry_proto::tonic::logs::v1::{LogRecord, ResourceLogs, ScopeLogs};
    use serde_json::Value as JsonValue;
    use std::net::SocketAddr;

    #[tokio::test]
    async fn uploader_batch_is_accepted_and_decoded() {
        let server = TestServer::start().await;
        let client = GenevaClient::new(GenevaClientConfig {
            endpoint: server.base_url.clone(),
            environment: "testenv".to_string(),
            account: "testaccount".to_string(),
            namespace: "TestNamespace".to_string(),
            region: "testregion".to_string(),
            config_major_version: 1,
            auth_method: AuthMethod::MockAuth,
            tenant: "tenant-a".to_string(),
            role_name: "checkout".to_string(),
            role_instance: "instance-1".to_string(),
            msi_resource: None,
        })
        .expect("client should initialize");

        let request = ExportLogsServiceRequest {
            resource_logs: vec![ResourceLogs {
                scope_logs: vec![ScopeLogs {
                    log_records: vec![LogRecord {
                        time_unix_nano: 1_718_432_000_000_000_000,
                        event_name: "CheckoutEvent".to_string(),
                        severity_number: 17,
                        severity_text: "ERROR".to_string(),
                        attributes: vec![
                            string_attr("operation", "checkout"),
                            int_attr("result", 127),
                        ],
                        body: Some(AnyValue {
                            value: Some(Value::StringValue("checkout failed".to_string())),
                        }),
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                ..Default::default()
            }],
        };
        let batches = client
            .encode_and_compress_logs(&request.resource_logs)
            .expect("batch should encode");
        assert_eq!(batches.len(), 1);
        client
            .upload_batch(&batches[0])
            .await
            .expect("batch should upload");

        let detail = server.wait_for_request(&batches[0].event_name).await;
        assert_eq!(detail["decode_status"], "decoded");
        assert_eq!(detail["event_name"], "CheckoutEvent");
        assert_eq!(detail["row_count"], 1);

        let records = detail["records"].as_array().expect("records array");
        assert_eq!(records.len(), 1);
        let payload = &records[0]["payload"];
        assert_eq!(payload["Role"], "checkout");
        assert_eq!(payload["RoleInstance"], "instance-1");
        assert_eq!(payload["body"], "checkout failed");
        assert_eq!(payload["operation"], "checkout");
        assert_eq!(payload["result"], 127);
    }

    struct TestServer {
        base_url: String,
        _worker: models::WorkerHandle,
        task: tokio::task::JoinHandle<()>,
        http: reqwest::Client,
        _temp_dir: tempfile::TempDir,
    }

    impl TestServer {
        async fn start() -> Self {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind test server");
            let addr = listener.local_addr().expect("local addr");
            let base_url = format!("http://{addr}");
            let temp_dir = tempfile::tempdir().expect("temp dir");
            let db_path = temp_dir.path().join("geneva-test-server.sqlite3");
            let config = ServerConfig {
                listen_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
                public_base_url: base_url.clone(),
                db_path,
                token_ttl_secs: 900,
                max_body_size: 64 * 1024 * 1024,
                monitoring_endpoint: base_url.clone(),
                primary_moniker: "diag-test-moniker".to_string(),
                account_group: "diag-test-account-group".to_string(),
            };
            let (state, worker) = sqlite::spawn_worker(config).expect("spawn worker");
            let app = build_router(Arc::new(state));
            let task = tokio::spawn(async move {
                axum::serve(listener, app).await.expect("serve");
            });

            let server = Self {
                base_url,
                _worker: worker,
                task,
                http: reqwest::Client::new(),
                _temp_dir: temp_dir,
            };
            server.wait_until_ready().await;
            server
        }

        async fn wait_until_ready(&self) {
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
            loop {
                if self
                    .http
                    .get(format!("{}/healthz", self.base_url))
                    .send()
                    .await
                    .is_ok_and(|response| response.status().is_success())
                {
                    return;
                }
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "server did not become ready before timeout"
                );
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            }
        }

        async fn wait_for_request(&self, event_name: &str) -> JsonValue {
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
            loop {
                let response = self
                    .http
                    .get(format!(
                        "{}/api/v1/debug/requests?event={event_name}",
                        self.base_url
                    ))
                    .send()
                    .await
                    .expect("list requests");
                let body: JsonValue = response.json().await.expect("requests json");
                if let Some(request_id) = body["items"]
                    .as_array()
                    .and_then(|items| items.first())
                    .and_then(|item| item["request_id"].as_str())
                {
                    let detail = self
                        .http
                        .get(format!(
                            "{}/api/v1/debug/requests/{request_id}/wait",
                            self.base_url
                        ))
                        .send()
                        .await
                        .expect("wait request");
                    return detail.json().await.expect("detail json");
                }
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "request was not observed before timeout"
                );
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            }
        }
    }

    impl Drop for TestServer {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    fn string_attr(key: &str, value: &str) -> KeyValue {
        KeyValue {
            key: key.to_string(),
            value: Some(AnyValue {
                value: Some(Value::StringValue(value.to_string())),
            }),
        }
    }

    fn int_attr(key: &str, value: i64) -> KeyValue {
        KeyValue {
            key: key.to_string(),
            value: Some(AnyValue {
                value: Some(Value::IntValue(value)),
            }),
        }
    }
}
