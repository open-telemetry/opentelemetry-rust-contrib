//! Example: Custom Route Extraction with Match Tables
//!
//! This example demonstrates how to use `HTTPLayerBuilder::with_route_extractor_fn()`
//! to implement custom route normalization logic that reduces cardinality by replacing
//! known dynamic path segments with placeholders.
//!
//! Use case: Your API has user-specific endpoints like `/users/alice/profile` and
//! `/users/bob/profile`. Without normalization, each user creates a unique span name
//! and `http.route` attribute, causing cardinality explosion in your metrics backend.
//!
//! Solution: Use a match table to recognize known usernames and replace them with
//! `{username}`, producing consistent routes like `/users/{username}/profile`.

use axum::extract::Path;
use axum::routing::get;
use axum::Router;
use opentelemetry::global;
use opentelemetry_instrumentation_tower::HTTPLayerBuilder;
use opentelemetry_otlp::{MetricExporter, SpanExporter};
use opentelemetry_sdk::{
    metrics::{PeriodicReader, SdkMeterProvider},
    trace::SdkTracerProvider,
};
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

const SERVICE_NAME: &str = "example-custom-route-extractor";
const _OTEL_METRIC_EXPORT_INTERVAL: Duration = Duration::from_secs(10);

fn init_otel_resource() -> opentelemetry_sdk::Resource {
    opentelemetry_sdk::Resource::builder()
        .with_service_name(SERVICE_NAME)
        .build()
}

/// Handler for user profile requests.
async fn get_user_profile(Path(username): Path<String>) -> String {
    format!("Profile for user: {username}")
}

/// Handler for user posts.
async fn get_user_posts(Path(username): Path<String>) -> String {
    format!("Posts by user: {username}")
}

/// Handler for the index page.
async fn index() -> &'static str {
    "Welcome! Try /users/alice/profile or /users/bob/posts"
}

/// Custom route extractor that normalizes known usernames to `{username}`.
///
/// This function demonstrates a pattern-based approach to route normalization:
/// 1. Parse the path into segments
/// 2. Check if a segment matches a known pattern (username in this case)
/// 3. Replace matching segments with a placeholder
///
/// For production use, you might:
/// - Load usernames from a database or cache
/// - Use regex patterns for more complex matching
/// - Combine multiple normalization rules
fn normalize_route(known_usernames: Arc<HashSet<String>>, path: &str) -> Option<String> {
    let mut result = String::with_capacity(path.len());

    for (i, segment) in path.split('/').enumerate() {
        if i > 0 {
            result.push('/');
        }
        if known_usernames.contains(segment) {
            result.push_str("{username}");
        } else {
            result.push_str(segment);
        }
    }

    Some(result)
}

#[tokio::main]
async fn main() {
    let meter_provider = {
        let exporter = MetricExporter::builder().with_tonic().build().unwrap();

        let reader = PeriodicReader::builder(exporter)
            .with_interval(_OTEL_METRIC_EXPORT_INTERVAL)
            .build();

        let provider = SdkMeterProvider::builder()
            .with_reader(reader)
            .with_resource(init_otel_resource())
            .build();

        global::set_meter_provider(provider.clone());
        provider
    };

    let tracer_provider = {
        let exporter = SpanExporter::builder().with_tonic().build().unwrap();

        let provider = SdkTracerProvider::builder()
            .with_batch_exporter(exporter)
            .with_resource(init_otel_resource())
            .build();

        global::set_tracer_provider(provider.clone());
        provider
    };

    // Known usernames that should be normalized to {username}.
    // In production, this could be loaded from a database, config file,
    // or populated dynamically from authentication tokens.
    let known_usernames: Arc<HashSet<String>> = Arc::new(
        ["alice", "bob", "charlie", "dave"]
            .iter()
            .map(|s| s.to_string())
            .collect(),
    );

    // Create the HTTP layer with a custom route extractor.
    // The closure captures the known_usernames set and uses it to normalize routes.
    let otel_layer = HTTPLayerBuilder::builder()
        .with_route_extractor_fn({
            let usernames = known_usernames.clone();
            move |req: &http::Request<_>| normalize_route(usernames.clone(), req.uri().path())
        })
        .build()
        .unwrap();

    // With this configuration:
    // - GET /users/alice/profile -> span name: "GET /users/{username}/profile"
    // - GET /users/bob/posts    -> span name: "GET /users/{username}/posts"
    // - GET /users/unknown/profile -> span name: "GET /users/unknown/profile"
    //   (unknown users are not normalized - you might want to handle this differently)

    let app = Router::new()
        .route("/", get(index))
        .route("/users/{username}/profile", get(get_user_profile))
        .route("/users/{username}/posts", get(get_user_posts))
        .layer(otel_layer);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:5000").await.unwrap();
    println!("Server running on http://localhost:5000");
    println!("Try:");
    println!("  curl http://localhost:5000/users/alice/profile");
    println!("  curl http://localhost:5000/users/bob/posts");
    println!("  curl http://localhost:5000/users/unknown/profile");

    let server = axum::serve(listener, app);

    if let Err(err) = server.await {
        eprintln!("server error: {err}");
    }

    // Gracefully shutdown the providers to ensure all spans and metrics are flushed.
    if let Err(err) = tracer_provider.shutdown() {
        eprintln!("tracer provider shutdown error: {err}");
    }
    if let Err(err) = meter_provider.shutdown() {
        eprintln!("meter provider shutdown error: {err}");
    }
}
