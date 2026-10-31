use opentelemetry_config::ConfigurationError;

use opentelemetry_sdk::{
    error::OTelSdkResult,
    metrics::{data::ResourceMetrics, exporter::PushMetricExporter},
};

use std::time::Duration;

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct CustomData {
    pub custom_string_field: String,
    pub custom_int_field: i32,
}

pub(crate) struct MockPeriodicExporterProvider {}

impl MockPeriodicExporterProvider {
    pub fn register_mock_exporter_factory(
        config: &serde_yaml::Value,
    ) -> Result<MockCustomExporter, ConfigurationError> {
        let custom_config: CustomData = serde_yaml::from_value(config.clone()).map_err(|e| {
            ConfigurationError::InvalidConfiguration(format!("Failed to parse CustomData: {}", e))
        })?;

        println!(
            "Configuring MockCustomExporter with string field: {} and int field: {}",
            custom_config.custom_string_field, custom_config.custom_int_field
        );

        Ok(MockCustomExporter { custom_config })
    }
}

pub(crate) struct MockCustomExporter {
    custom_config: CustomData,
}

impl PushMetricExporter for MockCustomExporter {
    async fn export(&self, metrics: &ResourceMetrics) -> OTelSdkResult {
        println!(
            "MockCustomExporter exporting metrics {:?} with custom config: {:?}",
            metrics, self.custom_config
        );
        Ok(())
    }

    fn force_flush(&self) -> OTelSdkResult {
        println!("MockCustomExporter force flushing metrics.");
        Ok(())
    }

    fn shutdown_with_timeout(&self, timeout: Duration) -> OTelSdkResult {
        println!(
            "MockCustomExporter shutting down with timeout: {:?}",
            timeout
        );
        Ok(())
    }

    fn shutdown(&self) -> OTelSdkResult {
        self.shutdown_with_timeout(Duration::from_secs(5))
    }

    fn temporality(&self) -> opentelemetry_sdk::metrics::Temporality {
        opentelemetry_sdk::metrics::Temporality::Cumulative
    }
}
