use anyhow::{Context, Result};
use std::net::SocketAddr;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct ServerConfig {
    pub listen_addr: SocketAddr,
    pub public_base_url: String,
    pub db_path: PathBuf,
    pub token_ttl_secs: i64,
    pub max_body_size: usize,
    pub monitoring_endpoint: String,
    pub primary_moniker: String,
    pub account_group: String,
}

impl ServerConfig {
    pub fn from_env() -> Result<Self> {
        let listen_addr = std::env::var("GENEVA_TEST_SERVER_ADDR")
            .unwrap_or_else(|_| "127.0.0.1:18080".to_string())
            .parse::<SocketAddr>()
            .context("invalid GENEVA_TEST_SERVER_ADDR")?;

        let public_base_url = std::env::var("GENEVA_TEST_SERVER_BASE_URL")
            .unwrap_or_else(|_| format!("http://{listen_addr}"));
        let public_base_url = public_base_url.trim_end_matches('/').to_string();

        let db_path = std::env::var("GENEVA_TEST_SERVER_DB")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("target/geneva-test-server.sqlite3"));

        let token_ttl_secs = std::env::var("GENEVA_TEST_SERVER_TOKEN_TTL_SECS")
            .unwrap_or_else(|_| "900".to_string())
            .parse::<i64>()
            .context("invalid GENEVA_TEST_SERVER_TOKEN_TTL_SECS")?;
        let max_body_size = std::env::var("GENEVA_TEST_SERVER_MAX_BODY_BYTES")
            .unwrap_or_else(|_| (64 * 1024 * 1024).to_string())
            .parse::<usize>()
            .context("invalid GENEVA_TEST_SERVER_MAX_BODY_BYTES")?;

        let monitoring_endpoint = std::env::var("GENEVA_TEST_SERVER_MONITORING_ENDPOINT")
            .unwrap_or_else(|_| "https://monitoring.test.internal".to_string());
        let primary_moniker = std::env::var("GENEVA_TEST_SERVER_PRIMARY_MONIKER")
            .unwrap_or_else(|_| "diag-test-moniker".to_string());
        let account_group = std::env::var("GENEVA_TEST_SERVER_ACCOUNT_GROUP")
            .unwrap_or_else(|_| "diag-test-account-group".to_string());

        Ok(Self {
            listen_addr,
            public_base_url,
            db_path,
            token_ttl_secs,
            max_body_size,
            monitoring_endpoint,
            primary_moniker,
            account_group,
        })
    }
}
