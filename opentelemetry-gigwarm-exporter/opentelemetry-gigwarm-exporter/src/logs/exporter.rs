use core::fmt;
use gigwarm_uploader::GigWarmUploader;
use opentelemetry_proto::transform::common::tonic::ResourceAttributesWithSchema;
use opentelemetry_proto::transform::logs::tonic::group_logs_by_resource_and_scope;
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::logs::LogBatch;
use std::sync::{atomic, Arc};

/// An OpenTelemetry exporter that writes Logs GIG/warm exporter
pub struct GigWarmExporter {
    resource: ResourceAttributesWithSchema,
    _is_shutdown: atomic::AtomicBool,
    uploader: Arc<GigWarmUploader>,
}

impl GigWarmExporter {
    /// Create a new GigWarmExporter
    pub fn new(uploader: Arc<GigWarmUploader>) -> Self {
        Self {
            resource: ResourceAttributesWithSchema::default(),
            _is_shutdown: atomic::AtomicBool::new(false),
            uploader,
        }
    }
}

impl Default for GigWarmExporter {
    fn default() -> Self {
        GigWarmExporter {
            resource: ResourceAttributesWithSchema::default(),
            _is_shutdown: atomic::AtomicBool::new(false),
            uploader: Arc::new(GigWarmUploader),
        }
    }
}

impl fmt::Debug for GigWarmExporter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("GIG/warm exporter")
    }
}

impl opentelemetry_sdk::logs::LogExporter for GigWarmExporter {
    /// Export logs to stdout
    async fn export(&self, _batch: LogBatch<'_>) -> OTelSdkResult {
        //serialize to otlp format
        let otlp = group_logs_by_resource_and_scope(_batch, &self.resource);
        //send to gigwarm using geneva-uploader
        let _ = self.uploader.upload_logs(otlp).await;

        Ok(())
    }

    fn set_resource(&mut self, resource: &opentelemetry_sdk::Resource) {
        self.resource = resource.into();
    }
}
