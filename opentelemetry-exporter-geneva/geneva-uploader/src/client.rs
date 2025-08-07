//! High-level GenevaClient for user code. Wraps config_service and ingestion_service.

use crate::config_service::client::{AuthMethod, GenevaConfigClient, GenevaConfigClientConfig, MsiIdentityType};
use crate::ingestion_service::uploader::{GenevaUploader, GenevaUploaderConfig};
use crate::payload_encoder::lz4_chunked_compression::lz4_chunked_compression;
use crate::payload_encoder::otlp_encoder::OtlpEncoder;
use futures::stream::{self, StreamExt};
use opentelemetry_proto::tonic::logs::v1::ResourceLogs;
use std::sync::Arc;

/// Configuration for GenevaClient (user-facing)
#[derive(Clone, Debug)]
pub struct GenevaClientConfig {
    pub endpoint: String,
    pub environment: String,
    pub account: String,
    pub namespace: String,
    pub region: String,
    pub config_major_version: u32,
    pub auth_method: AuthMethod,
    pub tenant: String,
    pub role_name: String,
    pub role_instance: String,
    /// Maximum number of concurrent uploads. If None, defaults to number of CPU cores.
    pub max_concurrent_uploads: Option<usize>,
    // Add event name/version here if constant, or per-upload if you want them per call.
}

impl GenevaClientConfig {
    /// Configure the client to use system-assigned managed identity
    ///
    /// # Example
    /// ```rust,no_run
    /// # use geneva_uploader::GenevaClientConfig;
    /// let config = GenevaClientConfig {
    ///     endpoint: "https://geneva.example.com".to_string(),
    ///     environment: "prod".to_string(),
    ///     account: "myaccount".to_string(),
    ///     namespace: "myservice".to_string(),
    ///     region: "westus2".to_string(),
    ///     config_major_version: 1,
    ///     auth_method: Default::default(), // placeholder
    ///     tenant: "mytenant".to_string(),
    ///     role_name: "myrole".to_string(),
    ///     role_instance: "instance1".to_string(),
    ///     max_concurrent_uploads: None,
    /// }.with_system_assigned_identity();
    /// ```
    pub fn with_system_assigned_identity(mut self) -> Self {
        self.auth_method = AuthMethod::ManagedIdentity {
            identity: None,
            fallback_to_default: false,
        };
        self
    }

    /// Configure the client to use user-assigned managed identity by Client ID
    ///
    /// # Arguments
    /// * `client_id` - The Client ID (Application ID) of the user-assigned managed identity
    /// * `fallback` - Whether to fallback to system-assigned identity if user-assigned fails
    ///
    /// # Example
    /// ```rust,no_run
    /// # use geneva_uploader::GenevaClientConfig;
    /// let config = GenevaClientConfig {
    ///     // ... other fields
    /// #   endpoint: "https://geneva.example.com".to_string(),
    /// #   environment: "prod".to_string(),
    /// #   account: "myaccount".to_string(),
    /// #   namespace: "myservice".to_string(),
    /// #   region: "westus2".to_string(),
    /// #   config_major_version: 1,
    /// #   auth_method: Default::default(),
    /// #   tenant: "mytenant".to_string(),
    /// #   role_name: "myrole".to_string(),
    /// #   role_instance: "instance1".to_string(),
    /// #   max_concurrent_uploads: None,
    /// }.with_user_assigned_client_id("12345678-1234-1234-1234-123456789012".to_string(), true);
    /// ```
    pub fn with_user_assigned_client_id(mut self, client_id: String, fallback: bool) -> Self {
        self.auth_method = AuthMethod::ManagedIdentity {
            identity: Some(MsiIdentityType::ClientId(client_id)),
            fallback_to_default: fallback,
        };
        self
    }

    /// Configure the client to use user-assigned managed identity by Object ID
    ///
    /// # Arguments
    /// * `object_id` - The Object ID of the user-assigned managed identity in Azure AD
    /// * `fallback` - Whether to fallback to system-assigned identity if user-assigned fails
    ///
    /// # Example
    /// ```rust,no_run
    /// # use geneva_uploader::GenevaClientConfig;
    /// let config = GenevaClientConfig {
    ///     // ... other fields
    /// #   endpoint: "https://geneva.example.com".to_string(),
    /// #   environment: "prod".to_string(),
    /// #   account: "myaccount".to_string(),
    /// #   namespace: "myservice".to_string(),
    /// #   region: "westus2".to_string(),
    /// #   config_major_version: 1,
    /// #   auth_method: Default::default(),
    /// #   tenant: "mytenant".to_string(),
    /// #   role_name: "myrole".to_string(),
    /// #   role_instance: "instance1".to_string(),
    /// #   max_concurrent_uploads: None,
    /// }.with_user_assigned_object_id("87654321-4321-4321-4321-210987654321".to_string(), false);
    /// ```
    pub fn with_user_assigned_object_id(mut self, object_id: String, fallback: bool) -> Self {
        self.auth_method = AuthMethod::ManagedIdentity {
            identity: Some(MsiIdentityType::ObjectId(object_id)),
            fallback_to_default: fallback,
        };
        self
    }

    /// Configure the client to use user-assigned managed identity by Resource ID
    ///
    /// # Arguments
    /// * `resource_id` - The full ARM Resource ID of the user-assigned managed identity
    /// * `fallback` - Whether to fallback to system-assigned identity if user-assigned fails
    ///
    /// # Example
    /// ```rust,no_run
    /// # use geneva_uploader::GenevaClientConfig;
    /// let config = GenevaClientConfig {
    ///     // ... other fields
    /// #   endpoint: "https://geneva.example.com".to_string(),
    /// #   environment: "prod".to_string(),
    /// #   account: "myaccount".to_string(),
    /// #   namespace: "myservice".to_string(),
    /// #   region: "westus2".to_string(),
    /// #   config_major_version: 1,
    /// #   auth_method: Default::default(),
    /// #   tenant: "mytenant".to_string(),
    /// #   role_name: "myrole".to_string(),
    /// #   role_instance: "instance1".to_string(),
    /// #   max_concurrent_uploads: None,
    /// }.with_user_assigned_resource_id(
    ///     "/subscriptions/sub-id/resourceGroups/rg/providers/Microsoft.ManagedIdentity/userAssignedIdentities/my-identity".to_string(),
    ///     false
    /// );
    /// ```
    pub fn with_user_assigned_resource_id(mut self, resource_id: String, fallback: bool) -> Self {
        self.auth_method = AuthMethod::ManagedIdentity {
            identity: Some(MsiIdentityType::ResourceId(resource_id)),
            fallback_to_default: fallback,
        };
        self
    }
}

/// Main user-facing client for Geneva ingestion.
#[derive(Clone)]
pub struct GenevaClient {
    uploader: Arc<GenevaUploader>,
    encoder: OtlpEncoder,
    metadata: String,
    max_concurrent_uploads: usize,
}

impl GenevaClient {
    /// Construct a new client with minimal configuration. Fetches and caches ingestion info as needed.
    pub async fn new(cfg: GenevaClientConfig) -> Result<Self, String> {
        // Build config client config
        let config_client_config = GenevaConfigClientConfig {
            endpoint: cfg.endpoint,
            environment: cfg.environment.clone(),
            account: cfg.account,
            namespace: cfg.namespace.clone(),
            region: cfg.region,
            config_major_version: cfg.config_major_version,
            auth_method: cfg.auth_method,
        };
        let config_client = Arc::new(
            GenevaConfigClient::new(config_client_config)
                .map_err(|e| format!("GenevaConfigClient init failed: {e}"))?,
        );

        let source_identity = format!(
            "Tenant={}/Role={}/RoleInstance={}",
            cfg.tenant, cfg.role_name, cfg.role_instance
        );

        // Define config_version before using it
        let config_version = format!("Ver{}v0", cfg.config_major_version);

        // Metadata string for the blob
        let metadata = format!(
            "namespace={}/eventVersion={}/tenant={}/role={}/roleinstance={}",
            cfg.namespace, config_version, cfg.tenant, cfg.role_name, cfg.role_instance,
        );

        // Uploader config
        let uploader_config = GenevaUploaderConfig {
            namespace: cfg.namespace.clone(),
            source_identity,
            environment: cfg.environment,
            config_version: config_version.clone(),
        };

        let uploader = GenevaUploader::from_config_client(config_client, uploader_config)
            .await
            .map_err(|e| format!("GenevaUploader init failed: {e}"))?;
        let max_concurrent_uploads = cfg.max_concurrent_uploads.unwrap_or_else(|| {
            // TODO - Use a more sophisticated method to determine concurrency if needed
            // currently using number of CPU cores
            std::thread::available_parallelism()
                .map(|p| p.get())
                .unwrap_or(4)
        });
        Ok(Self {
            uploader: Arc::new(uploader),
            encoder: OtlpEncoder::new(),
            metadata,
            max_concurrent_uploads,
        })
    }

    /// Upload OTLP logs (as ResourceLogs).
    pub async fn upload_logs(&self, logs: &[ResourceLogs]) -> Result<(), String> {
        let log_iter = logs
            .iter()
            .flat_map(|resource_log| resource_log.scope_logs.iter())
            .flat_map(|scope_log| scope_log.log_records.iter());
        // TODO: Investigate using tokio::spawn_blocking for event encoding to avoid blocking
        // the async executor thread for CPU-intensive work.
        let blobs = self.encoder.encode_log_batch(log_iter, &self.metadata);

        // create an iterator that yields futures for each upload
        let upload_futures = blobs.into_iter().map(|batch| {
            async move {
                // TODO: Investigate using tokio::spawn_blocking for LZ4 compression to avoid blocking
                // the async executor thread for CPU-intensive work.
                let compressed_blob = lz4_chunked_compression(&batch.data).map_err(|e| {
                    format!("LZ4 compression failed: {e} Event: {}", batch.event_name)
                })?;
                self.uploader
                    .upload(compressed_blob, &batch.event_name, &batch.metadata)
                    .await
                    .map(|_| ())
                    .map_err(|e| format!("Geneva upload failed: {e} Event: {}", batch.event_name))
            }
        });
        // Execute uploads concurrently with configurable concurrency
        let errors: Vec<String> = stream::iter(upload_futures)
            .buffer_unordered(self.max_concurrent_uploads)
            .filter_map(|result| async move { result.err() })
            .collect()
            .await;

        // Return error if any uploads failed
        if !errors.is_empty() {
            return Err(format!("Upload failures: {}", errors.join("; ")));
        }
        Ok(())
    }
}
