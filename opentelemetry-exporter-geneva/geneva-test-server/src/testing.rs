use crate::app::build_router;
use crate::config::ServerConfig;
use crate::models::WorkerHandle;
use crate::sqlite::spawn_worker;
use serde_json::Value;
use std::net::SocketAddr;
use std::sync::Arc;

pub struct TestServer {
    base_url: String,
    _worker: WorkerHandle,
    task: tokio::task::JoinHandle<()>,
    http: reqwest::Client,
    _temp_dir: tempfile::TempDir,
}

impl TestServer {
    pub async fn start() -> Self {
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
        let (state, worker) = spawn_worker(config).expect("spawn worker");
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

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub async fn wait_for_request(&self, event_name: &str) -> Value {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let body = self
                .get_debug_json(
                    &format!("/api/v1/debug/requests?event={event_name}"),
                    deadline,
                    "list requests",
                )
                .await;
            if let Some(request_id) = body["items"]
                .as_array()
                .and_then(|items| items.first())
                .and_then(|item| item["request_id"].as_str())
            {
                return self
                    .get_debug_json(
                        &format!("/api/v1/debug/requests/{request_id}/wait"),
                        deadline,
                        "wait request",
                    )
                    .await;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "request was not observed before timeout"
            );
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    }

    async fn get_debug_json(
        &self,
        path: &str,
        deadline: tokio::time::Instant,
        label: &str,
    ) -> Value {
        loop {
            let response = self
                .http
                .get(format!("{}{}", self.base_url, path))
                .send()
                .await
                .unwrap_or_else(|err| panic!("{label}: request failed: {err}"));
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|err| panic!("{label}: failed to read response body: {err}"));

            if status.is_success() {
                return serde_json::from_str(&body).unwrap_or_else(|err| {
                    panic!("{label}: failed to decode JSON response: {err}; body={body:?}")
                });
            }

            assert!(
                tokio::time::Instant::now() < deadline,
                "{label}: endpoint returned {status}; body={body:?}"
            );
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
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
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}
