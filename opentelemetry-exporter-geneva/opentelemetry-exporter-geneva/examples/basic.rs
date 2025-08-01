//! run with `$ cargo run --example basic

use geneva_uploader::client::{GenevaClient, GenevaClientConfig};
use geneva_uploader::AuthMethod;
use opentelemetry_appender_tracing::layer;
use opentelemetry_exporter_geneva::GenevaExporter;
use opentelemetry_sdk::logs::log_processor_with_async_runtime::BatchLogProcessor;
use opentelemetry_sdk::runtime::Tokio;
use opentelemetry_sdk::{
    logs::{BatchConfig, SdkLoggerProvider},
    Resource,
};
use std::env;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;
use tracing::{error, info, warn};
use tracing_subscriber::{prelude::*, EnvFilter};

/*
export GENEVA_ENDPOINT="https://abc.azurewebsites.net"
export GENEVA_ENVIRONMENT="Test"
export GENEVA_ACCOUNT="myaccount"
export GENEVA_NAMESPACE="myns"
export GENEVA_REGION="eastus"
export GENEVA_CERT_PATH="/tmp/client.p12"
export GENEVA_CERT_PASSWORD="password"
export GENEVA_CONFIG_MAJOR_VERSION=2
*/

#[tokio::main]
async fn main() {
    let endpoint = env::var("GENEVA_ENDPOINT").expect("GENEVA_ENDPOINT is required");
    let environment = env::var("GENEVA_ENVIRONMENT").expect("GENEVA_ENVIRONMENT is required");
    let account = env::var("GENEVA_ACCOUNT").expect("GENEVA_ACCOUNT is required");
    let namespace = env::var("GENEVA_NAMESPACE").expect("GENEVA_NAMESPACE is required");
    let region = env::var("GENEVA_REGION").expect("GENEVA_REGION is required");
    let cert_path =
        PathBuf::from(env::var("GENEVA_CERT_PATH").expect("GENEVA_CERT_PATH is required"));
    let cert_password = env::var("GENEVA_CERT_PASSWORD").expect("GENEVA_CERT_PASSWORD is required");
    let config_major_version: u32 = env::var("GENEVA_CONFIG_MAJOR_VERSION")
        .expect("GENEVA_CONFIG_MAJOR_VERSION is required")
        .parse()
        .expect("GENEVA_CONFIG_MAJOR_VERSION must be a u32");

    let tenant = env::var("GENEVA_TENANT").unwrap_or_else(|_| "default-tenant".to_string());
    let role_name = env::var("GENEVA_ROLE_NAME").unwrap_or_else(|_| "default-role".to_string());
    let role_instance =
        env::var("GENEVA_ROLE_INSTANCE").unwrap_or_else(|_| "default-instance".to_string());

    let config = GenevaClientConfig {
        endpoint,
        environment,
        account,
        namespace,
        region,
        config_major_version,
        auth_method: AuthMethod::Certificate {
            path: cert_path,
            password: cert_password,
        },
        tenant,
        role_name,
        role_instance,
        max_concurrent_uploads: None, // Use default
        user_agent_suffix: Some("BasicExample"),
    };

    let geneva_client = GenevaClient::new(config)
        .await
        .expect("Failed to create GenevaClient");

    let exporter = GenevaExporter::new(geneva_client);
    let batch_processor = BatchLogProcessor::builder(exporter, Tokio)
        .with_batch_config(BatchConfig::default())
        .build();

    let provider: SdkLoggerProvider = SdkLoggerProvider::builder()
        .with_resource(
            Resource::builder()
                .with_service_name("geneva-exporter-example")
                .build(),
        )
        .with_log_processor(batch_processor)
        .build();

    // To prevent a telemetry-induced-telemetry loop, OpenTelemetry's own internal
    // logging is properly suppressed. However, logs emitted by external components
    // (such as reqwest, tonic, etc.) are not suppressed as they do not propagate
    // OpenTelemetry context. Until this issue is addressed
    // (https://github.com/open-telemetry/opentelemetry-rust/issues/2877),
    // filtering like this is the best way to suppress such logs.
    //
    // The filter levels are set as follows:
    // - Allow `info` level and above by default.
    // - Completely restrict logs from `hyper`, `tonic`, `h2`, and `reqwest`.
    //
    // Note: This filtering will also drop logs from these components even when
    // they are used outside of the OTLP Exporter.
    let filter_otel = EnvFilter::new("info")
        .add_directive("hyper=off".parse().unwrap())
        .add_directive("opentelemetry=off".parse().unwrap())
        .add_directive("tonic=off".parse().unwrap())
        .add_directive("h2=off".parse().unwrap())
        .add_directive("reqwest=off".parse().unwrap());
    let otel_layer = layer::OpenTelemetryTracingBridge::new(&provider).with_filter(filter_otel);

    // Create a new tracing::Fmt layer to print the logs to stdout. It has a
    // default filter of `info` level and above, and `debug` and above for logs
    // from OpenTelemetry crates. The filter levels can be customized as needed.
    let filter_fmt = EnvFilter::new("info")
        .add_directive("hyper=debug".parse().unwrap())
        .add_directive("reqwest=debug".parse().unwrap())
        .add_directive("opentelemetry=debug".parse().unwrap());
    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_thread_names(true)
        .with_filter(filter_fmt);

    tracing_subscriber::registry()
        .with(otel_layer)
        .with(fmt_layer)
        .init();

    // User registration event
    info!(name: "Log", target: "my-system", event_id = 20, user_name = "user1", user_email = "user1@opentelemetry.io", message = "Registration successful");
    // User checkout event
    info!(name: "Log", target: "my-system", event_id = 51, user_name = "user2", user_email = "user2@opentelemetry.io", message = "Checkout successful");
    // Login event
    info!(name: "Log", target: "my-system", event_id = 30, user_name = "user3", user_email = "user3@opentelemetry.io", message = "User login successful");
    // Payment processed
    info!(name: "Log", target: "my-system", event_id = 52, user_name = "user2", user_email = "user2@opentelemetry.io", message = "Payment processed successfully");
    // Error event - Failed login
    error!(name: "Log", target: "my-system", event_id = 31, user_name = "user4", user_email = "user4@opentelemetry.io", message = "Login failed - invalid credentials");
    // Warning event - Cart abandoned
    warn!(name: "Log", target: "my-system", event_id = 53, user_name = "user5", user_email = "user5@opentelemetry.io", message = "Shopping cart abandoned");

    // Password reset
    info!(name: "Log", target: "my-system", event_id = 32, user_name = "user1", user_email = "user1@opentelemetry.io", message = "Password reset requested");

    // Order shipped
    info!(name: "Log", target: "my-system", event_id = 54, user_name = "user2", user_email = "user2@opentelemetry.io", message = "Order shipped successfully");

    // sleep for a while
    println!("Sleeping for 5 seconds...");
    thread::sleep(Duration::from_secs(5));
    let _ = provider.shutdown();
    println!("Shutting down provider");
}
