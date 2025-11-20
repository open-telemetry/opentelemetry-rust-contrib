use opentelemetry_config::{
    model::metrics::reader::Periodic, ConfigurationError,
};

use opentelemetry_sdk::{
    error::OTelSdkResult,
    metrics::{
        data::ResourceMetrics, exporter::PushMetricExporter, MeterProviderBuilder, PeriodicReader,
    },
};

use std::time::Duration;

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

pub(crate) struct MockPeriodicReaderProvider {}

impl MockPeriodicReaderProvider {
    pub fn register_mock_reader_factory(
        mut meter_provider_builder: MeterProviderBuilder,
        periodic_config: &Periodic,
    ) -> Result<MeterProviderBuilder, ConfigurationError> {
        let config = serde_yaml::from_value::<MockCustomConfig>(periodic_config.exporter.clone())
            .map_err(|e| {
            ConfigurationError::InvalidConfiguration(format!(
                "Failed to parse MockCustomConfig: {}",
                e
            ))
        })?;
        println!(
            "Configuring MockCustomExporter with string field: {} and int field: {}",
            config.custom.custom_string_field, config.custom.custom_int_field
        );

        let exporter = MockCustomExporter {
            custom_config: config,
        };

        // TODO: Add timeout from config
        let reader = PeriodicReader::builder(exporter)
            .with_interval(std::time::Duration::from_millis(periodic_config.interval))
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
