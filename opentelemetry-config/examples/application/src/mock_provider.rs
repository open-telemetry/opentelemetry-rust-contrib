use opentelemetry_config::ConfigurationError;

use opentelemetry_sdk::{
    error::OTelSdkResult,
    metrics::{
        data::ResourceMetrics, exporter::PushMetricExporter, MeterProviderBuilder, PeriodicReader,
    },
};

use std::time::Duration;

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PeriodicConfig {
    pub interval: Option<u64>,
    pub exporter: MockCustomConfig,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct MockCustomConfig {
    pub custom: CustomData,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct CustomData {
    pub custom_string_field: String,
    pub custom_int_field: i32,
}

pub(crate) struct MockPeriodicExporterProvider {}

impl MockPeriodicExporterProvider {
    pub fn register_mock_exporter_factory(
        mut meter_provider_builder: MeterProviderBuilder,
        periodic_config_yaml: &str,
    ) -> Result<MeterProviderBuilder, ConfigurationError> {
        let periodic_config: PeriodicConfig =
            serde_yaml::from_str(periodic_config_yaml).map_err(|e| {
                ConfigurationError::InvalidConfiguration(format!(
                    "Failed to parse PeriodicConfig: {}",
                    e
                ))
            })?;

        let config_exporter = periodic_config.exporter;

        println!(
            "Configuring MockCustomExporter with string field: {} and int field: {}",
            config_exporter.custom.custom_string_field, config_exporter.custom.custom_int_field
        );

        let exporter = MockCustomExporter {
            custom_config: config_exporter,
        };

        let interval_millis = periodic_config.interval.unwrap_or(60000);

        // TODO: Add timeout from config
        let reader = PeriodicReader::builder(exporter)
            .with_interval(std::time::Duration::from_millis(interval_millis))
            .build();

        meter_provider_builder = meter_provider_builder.with_reader(reader);
        Ok(meter_provider_builder)
    }
}

struct MockCustomExporter {
    custom_config: MockCustomConfig,
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
