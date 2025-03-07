//! The user_events exporter will enable applications to use OpenTelemetry API
//! to capture the telemetry events, and write to user_events subsystem.

#![warn(missing_debug_implementations, missing_docs)]

mod logs;

pub use logs::*;

#[cfg(test)]
mod tests {

    use super::*;
    use opentelemetry_appender_tracing::layer;
    use opentelemetry_sdk::logs::LoggerProviderBuilder;
    use std::process::Command;
    use tracing::error;
    use tracing_subscriber::{layer::SubscriberExt, EnvFilter, Layer};

    #[test]
    fn integration_test_basic() {
        check_user_events_available().expect("Kernel does not support user_events. Verify your distribution/kernel supports user_events: https://docs.kernel.org/trace/user_events.html.");
        let logger_provider = LoggerProviderBuilder::default()
            .with_user_event_exporter("myprovider")
            .build();
        let filter_otel =
            EnvFilter::new("info").add_directive("opentelemetry=off".parse().unwrap());
        let otel_layer = layer::OpenTelemetryTracingBridge::new(&logger_provider);
        let otel_layer = otel_layer.with_filter(filter_otel);

        let subscriber = tracing_subscriber::registry().with(otel_layer);
        let _guard = tracing::subscriber::set_default(subscriber);

        error!(
            name: "my-event-name",
            event_id = 20,
            user_name = "otel user",
            user_email = "otel.user@opentelemtry.com");
    }

    fn check_user_events_available() -> Result<(), String> {
        let output = Command::new("sudo")
            .arg("cat")
            .arg("/sys/kernel/tracing/user_events_status")
            .output()
            .map_err(|e| format!("Failed to execute command: {}", e))?;

        if output.status.success() {
            Ok(())
        } else {
            Err(format!(
                "Command executed with failing error code: {}",
                String::from_utf8_lossy(&output.stderr)
            ))
        }
    }
}
