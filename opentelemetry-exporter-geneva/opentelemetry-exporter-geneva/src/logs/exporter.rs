use core::fmt;
use geneva_uploader::Uploader;
use opentelemetry_proto::transform::common::tonic::ResourceAttributesWithSchema;
use opentelemetry_proto::transform::logs::tonic::group_logs_by_resource_and_scope;
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::logs::LogBatch;
use std::sync::{atomic, Arc};

/// An OpenTelemetry exporter that writes logs to Geneva exporter
pub struct GenevaExporter {
    resource: ResourceAttributesWithSchema,
    _is_shutdown: atomic::AtomicBool,
    uploader: Arc<Uploader>,
}

impl GenevaExporter {
    /// Create a new GenavaExporter
    pub fn new(uploader: Arc<Uploader>) -> Self {
        Self {
            resource: ResourceAttributesWithSchema::default(),
            _is_shutdown: atomic::AtomicBool::new(false),
            uploader,
        }
    }
}

impl Default for GenevaExporter {
    fn default() -> Self {
        GenevaExporter {
            resource: ResourceAttributesWithSchema::default(),
            _is_shutdown: atomic::AtomicBool::new(false),
            uploader: Arc::new(Uploader),
        }
    }
}

impl fmt::Debug for GenevaExporter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Genava exporter")
    }
}

impl opentelemetry_sdk::logs::LogExporter for GenevaExporter {
    /// Export logs to stdout
    async fn export(&self, _batch: LogBatch<'_>) -> OTelSdkResult {
        //serialize to otlp format
        let otlp = group_logs_by_resource_and_scope(_batch, &self.resource);
        //TODO send to Geneva using geneva-uploader
        let _ = self.uploader.upload_logs(otlp).await;

        Ok(())
    }

    fn set_resource(&mut self, resource: &opentelemetry_sdk::Resource) {
        self.resource = resource.into();
    }
}
