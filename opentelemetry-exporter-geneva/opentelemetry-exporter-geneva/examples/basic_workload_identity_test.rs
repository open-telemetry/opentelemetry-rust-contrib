//! run with `$ cargo run --example basic_workload_identity_test`

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
use std::thread;
use std::time::Duration;
use tracing::{error, info, warn};
use tracing_subscriber::{prelude::*, EnvFilter};

/*
Environment variables required:

export GENEVA_ENDPOINT="https://abc.azurewebsites.net"
export GENEVA_ENVIRONMENT="Test"
export GENEVA_ACCOUNT="PipelineAgent2Demo"
export GENEVA_NAMESPACE="PAdemo2"
export GENEVA_REGION="eastus"
export GENEVA_CONFIG_MAJOR_VERSION=2
export MONITORING_GCS_AUTH_ID_TYPE="AuthWorkloadIdentity"
export GENEVA_WORKLOAD_IDENTITY_RESOURCE="https://abc.azurewebsites.net" # Resource (audience) base for token exchange

# Azure Workload Identity configuration:
export AZURE_CLIENT_ID="<your-client-id>"           # Azure AD Application (client) ID
export AZURE_TENANT_ID="<your-tenant-id>"           # Azure AD Tenant ID
export AZURE_FEDERATED_TOKEN_FILE="/var/run/secrets/azure/tokens/azure-identity-token" # Path to service account token (Kubernetes default)

# Optional: Override the token file path
# export WORKLOAD_IDENTITY_TOKEN_FILE="/custom/path/to/token"
*/

#[tokio::main]
async fn main() {
    let endpoint = env::var("GENEVA_ENDPOINT").expect("GENEVA_ENDPOINT is required");
    let environment = env::var("GENEVA_ENVIRONMENT").expect("GENEVA_ENVIRONMENT is required");
    let account = env::var("GENEVA_ACCOUNT").expect("GENEVA_ACCOUNT is required");
    let namespace = env::var("GENEVA_NAMESPACE").expect("GENEVA_NAMESPACE is required");
    let region = env::var("GENEVA_REGION").expect("GENEVA_REGION is required");
    let config_major_version: u32 = env::var("GENEVA_CONFIG_MAJOR_VERSION")
        .expect("GENEVA_CONFIG_MAJOR_VERSION is required")
        .parse()
        .expect("GENEVA_CONFIG_MAJOR_VERSION must be a u32");

    let tenant = env::var("GENEVA_TENANT").unwrap_or_else(|_| "default-tenant".to_string());
    let role_name = env::var("GENEVA_ROLE_NAME").unwrap_or_else(|_| "default-role".to_string());
    let role_instance =
        env::var("GENEVA_ROLE_INSTANCE").unwrap_or_else(|_| "default-instance".to_string());

    // Determine authentication method based on environment variables
    let auth_method = match env::var("MONITORING_GCS_AUTH_ID_TYPE").as_deref() {
        Ok("AuthWorkloadIdentity") => {
            let resource = env::var("GENEVA_WORKLOAD_IDENTITY_RESOURCE")
                .expect("GENEVA_WORKLOAD_IDENTITY_RESOURCE required for Workload Identity auth");

            // Note: AZURE_CLIENT_ID, AZURE_TENANT_ID, and AZURE_FEDERATED_TOKEN_FILE
            // are read automatically by the azure_identity crate from environment variables.
            // These are typically set by the Azure Workload Identity webhook in Kubernetes.
            AuthMethod::WorkloadIdentity {
                resource,
            }
        }
        _ => panic!(
            "This example requires Workload Identity authentication. Set MONITORING_GCS_AUTH_ID_TYPE=AuthWorkloadIdentity"
        ),
    };

    let config = GenevaClientConfig {
        endpoint,
        environment,
        account,
        namespace,
        region,
        config_major_version,
        tenant,
        role_name,
        role_instance,
        auth_method,
        msi_resource: None, // Not used for Workload Identity
    };

    // GenevaClient::new is synchronous (returns Result), so no await is needed here.
    let geneva_client = GenevaClient::new(config).expect("Failed to create GenevaClient");

    let exporter = GenevaExporter::new(geneva_client);
    let batch_processor = BatchLogProcessor::builder(exporter, Tokio)
        .with_batch_config(BatchConfig::default())
        .build();

    let provider: SdkLoggerProvider = SdkLoggerProvider::builder()
        .with_resource(
            Resource::builder()
                .with_service_name("geneva-exporter-workload-identity-test")
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
        .add_directive("opentelemetry=debug".parse().unwrap())
        .add_directive("geneva-uploader=debug".parse().unwrap());
    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_thread_names(true)
        .with_filter(filter_fmt);

    tracing_subscriber::registry()
        .with(otel_layer)
        .with(fmt_layer)
        .init();

    // Generate logs to trigger batch processing and GCS calls
    info!(name: "Log", target: "my-system", event_id = 20, user_name = "user1", user_email = "user1@opentelemetry.io", message = "Registration successful");
    info!(name: "Log", target: "my-system", event_id = 51, user_name = "user2", user_email = "user2@opentelemetry.io", message = "Checkout successful");
    info!(name: "Log", target: "my-system", event_id = 30, user_name = "user3", user_email = "user3@opentelemetry.io", message = "User login successful");
    info!(name: "Log", target: "my-system", event_id = 52, user_name = "user2", user_email = "user2@opentelemetry.io", message = "Payment processed successfully");
    error!(name: "Log", target: "my-system", event_id = 31, user_name = "user4", user_email = "user4@opentelemetry.io", message = "Login failed - invalid credentials");
    warn!(name: "Log", target: "my-system", event_id = 53, user_name = "user5", user_email = "user5@opentelemetry.io", message = "Shopping cart abandoned");
    info!(name: "Log", target: "my-system", event_id = 32, user_name = "user1", user_email = "user1@opentelemetry.io", message = "Password reset requested");
    info!(name: "Log", target: "my-system", event_id = 54, user_name = "user2", user_email = "user2@opentelemetry.io", message = "Order shipped successfully");

    println!("Sleeping for 30 seconds...");
    thread::sleep(Duration::from_secs(30));

    let _ = provider.shutdown();
    println!("Shutting down provider");
}
