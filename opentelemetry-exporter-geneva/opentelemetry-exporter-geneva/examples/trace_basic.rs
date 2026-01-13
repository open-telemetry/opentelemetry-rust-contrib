//! run with `$ cargo run --example trace_basic

use geneva_uploader::client::{GenevaClient, GenevaClientConfig};
use geneva_uploader::AuthMethod;
use opentelemetry::{global, trace::Tracer, KeyValue};
use opentelemetry_exporter_geneva::GenevaTraceExporter;
use opentelemetry_sdk::trace::{SdkTracerProvider, SimpleSpanProcessor};
use opentelemetry_sdk::Resource;
use std::env;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

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
        msi_resource: None,
    };

    let geneva_client = GenevaClient::new(config).expect("Failed to create GenevaClient");

    // Create Geneva trace exporter
    let exporter = GenevaTraceExporter::new(geneva_client);

    // Create simple span processor (exports spans immediately)
    let span_processor = SimpleSpanProcessor::new(exporter);

    // Create tracer provider
    let tracer_provider = SdkTracerProvider::builder()
        .with_span_processor(span_processor)
        .with_resource(
            Resource::builder()
                .with_service_name("geneva-trace-exporter-example")
                .build(),
        )
        .build();

    // Set the global tracer provider
    global::set_tracer_provider(tracer_provider.clone());

    // Get a tracer
    let tracer = global::tracer("geneva-trace-example");

    // Create some example spans
    println!("Creating example spans...");

    // Example 1: User registration flow
    {
        let _registration_span = tracer
            .span_builder("user_registration")
            .with_attributes(vec![
                KeyValue::new("user.id", "user123"),
                KeyValue::new("user.email", "user123@example.com"),
                KeyValue::new("operation.type", "registration"),
            ])
            .start(&tracer);

        // Database operation span
        {
            let _db_span = tracer
                .span_builder("database_query")
                .with_attributes(vec![
                    KeyValue::new("db.system", "postgresql"),
                    KeyValue::new("db.name", "users"),
                    KeyValue::new("db.operation", "INSERT"),
                    KeyValue::new(
                        "db.statement",
                        "INSERT INTO users (email, name) VALUES (?, ?)",
                    ),
                ])
                .start(&tracer);
            thread::sleep(Duration::from_millis(50)); // Simulate database work
        } // db_span ends here

        // Email operation span
        {
            let _email_span = tracer
                .span_builder("send_welcome_email")
                .with_attributes(vec![
                    KeyValue::new("http.method", "POST"),
                    KeyValue::new("http.url", "https://api.email-service.com/send"),
                    KeyValue::new("http.status_code", 200),
                    KeyValue::new("email.type", "welcome"),
                ])
                .start(&tracer);
            thread::sleep(Duration::from_millis(100)); // Simulate HTTP request
        } // email_span ends here
    } // registration_span ends here

    // Example 2: E-commerce checkout flow
    {
        let _checkout_span = tracer
            .span_builder("checkout_process")
            .with_attributes(vec![
                KeyValue::new("user.id", "user456"),
                KeyValue::new("cart.total", 99.99),
                KeyValue::new("currency", "USD"),
            ])
            .start(&tracer);

        // Payment processing span
        {
            let _payment_span = tracer
                .span_builder("process_payment")
                .with_attributes(vec![
                    KeyValue::new("payment.method", "credit_card"),
                    KeyValue::new("payment.amount", 99.99),
                    KeyValue::new("payment.processor", "stripe"),
                ])
                .start(&tracer);
            thread::sleep(Duration::from_millis(200)); // Simulate payment processing
        } // payment_span ends here

        // Inventory update span
        {
            let _inventory_span = tracer
                .span_builder("update_inventory")
                .with_attributes(vec![
                    KeyValue::new("product.id", "prod789"),
                    KeyValue::new("quantity.reserved", 2),
                    KeyValue::new("inventory.operation", "reserve"),
                ])
                .start(&tracer);
            thread::sleep(Duration::from_millis(30)); // Simulate inventory update
        } // inventory_span ends here
    } // checkout_span ends here

    // Example 3: Error scenario - failed login
    {
        let _failed_login_span = tracer
            .span_builder("user_login")
            .with_attributes(vec![
                KeyValue::new("user.email", "invalid@example.com"),
                KeyValue::new("login.result", "failed"),
                KeyValue::new("error.type", "authentication_error"),
            ])
            .start(&tracer);
        thread::sleep(Duration::from_millis(10)); // Simulate failed login
    } // failed_login_span ends here

    // Example 4: API request
    {
        let _api_span = tracer
            .span_builder("api_request")
            .with_attributes(vec![
                KeyValue::new("http.method", "GET"),
                KeyValue::new("http.route", "/api/users/:id"),
                KeyValue::new("http.status_code", 200),
                KeyValue::new("user.id", "user789"),
            ])
            .start(&tracer);
        thread::sleep(Duration::from_millis(75)); // Simulate API processing
    } // api_span ends here

    println!("Spans created and exported successfully!");

    // SimpleSpanProcessor exports spans immediately, so no need to wait
    println!("All spans have been exported to Geneva!");

    // Shutdown the tracer provider
    tracer_provider
        .shutdown()
        .expect("Failed to shutdown tracer provider");
    println!("Tracer provider shut down successfully!");
}
