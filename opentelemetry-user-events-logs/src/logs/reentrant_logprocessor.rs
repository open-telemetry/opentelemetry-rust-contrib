use std::fmt::Debug;

use opentelemetry::InstrumentationScope;
use opentelemetry_sdk::logs::LogExporter;
use opentelemetry_sdk::{
    error::OTelSdkResult,
    logs::{LogBatch, SdkLogRecord},
};

/// This export processor exports without synchronization.
/// This is currently only used in users_event exporter, where we know
/// that the underlying exporter is safe under concurrent calls

#[derive(Debug)]
pub struct ReentrantLogProcessor<T: LogExporter> {
    exporter: T,
}

impl<T: LogExporter> ReentrantLogProcessor<T> {
    /// constructor that accepts an exporter instance
    pub fn new(exporter: T) -> Self {
        ReentrantLogProcessor { exporter }
    }
}

impl<T: LogExporter> opentelemetry_sdk::logs::LogProcessor for ReentrantLogProcessor<T> {
    fn emit(&self, record: &mut SdkLogRecord, scope: &InstrumentationScope) {
        let log_tuple = &[(record as &SdkLogRecord, scope)];
        // TODO: Using futures_executor::block_on can make the code non reentrant safe
        // if that crate starts emitting logs that are bridged to OTel.
        let _ = futures_executor::block_on(self.exporter.export(LogBatch::new(log_tuple)));
    }

    // Nothing to flush
    fn force_flush(&self) -> OTelSdkResult {
        Ok(())
    }

    // Nothing to shutdown
    fn shutdown(&self) -> OTelSdkResult {
        // TODO: Actually invoke shutdown on the exporter
        // This cannot be done today as it requires mutable reference to exporter.
        Ok(())
    }

    #[cfg(feature = "spec_unstable_logs_enabled")]
    fn event_enabled(
        &self,
        level: opentelemetry::logs::Severity,
        target: &str,
        name: Option<&str>,
    ) -> bool {
        self.exporter.event_enabled(level, target, name)
    }
}
