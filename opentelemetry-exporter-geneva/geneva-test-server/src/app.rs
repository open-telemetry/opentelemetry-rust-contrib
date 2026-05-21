use crate::gcs::{handle_gcs_request, healthz};
use crate::ingest::{get_records, get_request_detail, get_requests, ingest, wait_request_detail};
use crate::models::AppState;
use axum::extract::DefaultBodyLimit;
use axum::{routing::get, routing::post, Router};
use std::sync::Arc;

pub(crate) fn build_router(state: Arc<AppState>) -> Router {
    let max_body_size = state.max_body_size;
    Router::new()
        .route("/healthz", get(healthz))
        .route(
            "/api/agent/v3/{environment}/{account}/MonitoringStorageKeys",
            get(handle_gcs_request),
        )
        .route(
            "/api/agent/v3/{environment}/{account}/MonitoringStorageKeys/",
            get(handle_gcs_request),
        )
        .route(
            "/userapi/agent/v3/{environment}/{account}/MonitoringStorageKeys",
            get(handle_gcs_request),
        )
        .route(
            "/userapi/agent/v3/{environment}/{account}/MonitoringStorageKeys/",
            get(handle_gcs_request),
        )
        .route("/api/v1/ingestion/ingest", post(ingest))
        .route("/api/v1/debug/requests", get(get_requests))
        .route(
            "/api/v1/debug/requests/{request_id}",
            get(get_request_detail),
        )
        .route(
            "/api/v1/debug/requests/{request_id}/wait",
            get(wait_request_detail),
        )
        .route("/api/v1/debug/records", get(get_records))
        .layer(DefaultBodyLimit::max(max_body_size))
        .with_state(state)
}
