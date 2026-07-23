//! High-level GenevaClient for user code. Wraps config_service and ingestion_service.

use crate::config_service::client::{AuthMethod, GenevaConfigClient, GenevaConfigClientConfig};
// ManagedIdentitySelector removed; no re-export needed.
use crate::ingestion_service::uploader::{
    GenevaUploader, GenevaUploaderConfig, GenevaUploaderError,
};
use crate::payload_encoder::otlp_encoder::OtlpEncoder;
use crate::payload_encoder::otlp_encoder::{
    lookup_obo_config, resolve_mapped_destination, stringify_routing_primitive, MetadataFields,
    RoutingPrimitive, SCOPE_NAME_ROUTING_KEY, SCOPE_VERSION_ROUTING_KEY,
};
pub use crate::payload_encoder::otlp_encoder::{
    LogsEventNameMapping, LogsEventNameRoutingKey, OboEventConfig, OboEventMap,
    SpanEventNameMapping, SpanEventNameRoutingKey,
};
use opentelemetry_proto::tonic::common::v1::{
    any_value::Value as ProtoAnyValue, InstrumentationScope, KeyValue,
};
use opentelemetry_proto::tonic::resource::v1::Resource;
use opentelemetry_proto::tonic::trace::v1::{ResourceSpans, Span};
use otap_df_pdata_views::views::logs::LogsDataView;
use std::borrow::Cow;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, info};

/// Public batch type (already LZ4 chunked compressed).
/// Produced by `OtlpEncoder::encode_log_batch` and returned to callers.
#[derive(Debug, Clone, PartialEq)]
pub struct EncodedBatch {
    pub event_name: String,
    pub(crate) data: Vec<u8>,
    pub(crate) metadata: crate::payload_encoder::central_blob::BatchMetadata,
    pub row_count: usize,
}

impl EncodedBatch {
    /// Returns the size in bytes of the compressed payload uploaded to Geneva.
    #[must_use]
    pub fn compressed_size(&self) -> usize {
        self.data.len()
    }
}

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
    pub msi_resource: Option<String>, // Required for Managed Identity variants
    pub logs: Option<LogsConfig>,
    pub spans: Option<TracesConfig>,
    pub obo_event_map: Option<OboEventMap>, // Per-event OBO config (None = no OBO)
}

#[derive(Clone, Debug)]
pub struct LogsConfig {
    pub default_event_name: Option<String>,
    pub event_name_mapping: Option<LogsEventNameMapping>,
}

#[derive(Clone, Debug)]
pub struct TracesConfig {
    pub default_event_name: Option<String>,
    pub event_name_mapping: Option<SpanEventNameMapping>,
}

/// Agent-fed credential source: the host agent resolves the
/// GIG token + endpoint + moniker and supplies them here, so the uploader skips
/// its own GCS config-service handshake. Queried per upload, so host token
/// rotation is observed without reconstructing the client.
pub trait AgentFedCredentialSource: Send + Sync + std::fmt::Debug {
    /// The current credential, or `None` if the host has not provisioned one yet.
    ///
    /// Async so an agent-fed source can resolve the token through an awaitable
    /// capability (e.g. otap's `bearer_token_provider`, whose `get_token` may
    /// perform a credential call on a cache miss) by awaiting it, rather than
    /// polling the future once and dropping it if it is not immediately ready.
    /// Returning a boxed future (instead of `async fn`) keeps the trait
    /// object-safe for `Arc<dyn AgentFedCredentialSource>`.
    fn current(&self) -> AgentFedCredentialFuture<'_>;
}

/// Future returned by [`AgentFedCredentialSource::current`].
pub type AgentFedCredentialFuture<'a> =
    Pin<Box<dyn Future<Output = Option<AgentFedCredential>> + Send + 'a>>;

/// A host-provided GIG credential snapshot (see [`AgentFedCredentialSource`]).
#[derive(Clone)]
pub struct AgentFedCredential {
    /// GIG bearer token (sent as `Authorization: Bearer`).
    pub token: String,
    /// GIG ingestion endpoint (base URL the upload POSTs to).
    pub endpoint: String,
    /// Account moniker for the upload.
    pub moniker: String,
}

impl std::fmt::Debug for AgentFedCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Redact the bearer token; expose only the non-secret routing fields.
        f.debug_struct("AgentFedCredential")
            .field("token", &"<redacted>")
            .field("endpoint", &self.endpoint)
            .field("moniker", &self.moniker)
            .finish()
    }
}

/// Error type returned by [`GenevaClient::upload_batch`].
///
/// Provides enough information for callers to implement retry strategies:
/// - [`HttpStatus`](UploadError::HttpStatus) carries the HTTP status code and
///   an optional `Retry-After` duration so callers can distinguish retriable
///   server errors (429, 5xx) from permanent client errors (4xx).
/// - [`Transport`](UploadError::Transport) indicates a network-level failure
///   (timeout, connection refused, DNS) that is typically retriable.
/// - [`Other`](UploadError::Other) covers config-service or internal errors.
#[derive(Debug)]
pub enum UploadError {
    /// Server returned a non-202 HTTP status.
    HttpStatus {
        status: u16,
        retry_after: Option<Duration>,
        message: String,
    },
    /// Network/transport failure (timeout, connection refused, DNS, etc.)
    Transport(String),
    /// Config service or other internal error.
    Other(String),
}

impl fmt::Display for UploadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HttpStatus {
                status, message, ..
            } => {
                write!(f, "upload failed with status {status}: {message}")
            }
            Self::Transport(msg) => write!(f, "transport error: {msg}"),
            Self::Other(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for UploadError {}

/// Main user-facing client for Geneva ingestion.
#[derive(Clone)]
pub struct GenevaClient {
    uploader: Arc<GenevaUploader>,
    encoder: OtlpEncoder,
    metadata_fields: MetadataFields,
    log_table_name: Arc<str>,
    log_event_name_mapping: Option<LogsEventNameMapping>,
    span_table_name: Arc<str>,
    span_event_name_mapping: Option<SpanEventNameMapping>,
    obo_event_map: Option<OboEventMap>,
}

fn span_routing_value_from_attributes(attributes: &[KeyValue], key: &str) -> Option<String> {
    for attr in attributes {
        if attr.key != key {
            continue;
        }
        let value = attr.value.as_ref()?.value.as_ref()?;
        return match value {
            ProtoAnyValue::StringValue(value) => {
                stringify_routing_primitive(RoutingPrimitive::Str(value))
            }
            ProtoAnyValue::IntValue(value) => {
                stringify_routing_primitive(RoutingPrimitive::Int(*value))
            }
            ProtoAnyValue::DoubleValue(value) => {
                stringify_routing_primitive(RoutingPrimitive::Double(*value))
            }
            ProtoAnyValue::BoolValue(value) => {
                stringify_routing_primitive(RoutingPrimitive::Bool(*value))
            }
            _ => None,
        };
    }
    None
}

fn span_routing_value_from_scope(
    scope: Option<&InstrumentationScope>,
    key: &str,
) -> Option<String> {
    let scope = scope?;
    match key {
        SCOPE_NAME_ROUTING_KEY => stringify_routing_primitive(RoutingPrimitive::Str(&scope.name)),
        SCOPE_VERSION_ROUTING_KEY => {
            stringify_routing_primitive(RoutingPrimitive::Str(&scope.version))
        }
        _ => span_routing_value_from_attributes(&scope.attributes, key),
    }
}

fn span_routing_value_from_resource(resource: Option<&Resource>, key: &str) -> Option<String> {
    resource.and_then(|resource| span_routing_value_from_attributes(&resource.attributes, key))
}

/// Span routing resolved once per instrumentation scope.
enum SpanScopeRouting<'a> {
    /// The routed event name is constant for every span in the scope (no mapping
    /// configured, or a resource-/scope-level routing key). Resolved once.
    Fixed(String),
    /// Routing depends on each span's own attributes; only `SpanAttribute` keys
    /// need per-span resolution.
    PerSpan {
        mapping: &'a SpanEventNameMapping,
        key: &'a str,
    },
}

/// Resolve the scope-invariant part of span routing exactly once per scope.
///
/// For resource-/scope-level routing keys the source value is constant across
/// every span in the scope, so the destination event name is computed here and
/// reused; only `SpanAttribute` routing is deferred to per-span resolution.
fn resolve_span_scope_routing<'a>(
    resource: Option<&Resource>,
    scope: Option<&InstrumentationScope>,
    table_name: &str,
    event_name_mapping: Option<&'a SpanEventNameMapping>,
) -> SpanScopeRouting<'a> {
    let Some(mapping) = event_name_mapping else {
        return SpanScopeRouting::Fixed(table_name.to_string());
    };

    match &mapping.routing_key {
        SpanEventNameRoutingKey::ResourceAttribute(key) => SpanScopeRouting::Fixed(
            span_routing_value_from_resource(resource, key)
                .and_then(|value| resolve_mapped_destination(&mapping.events, &value))
                .unwrap_or_else(|| table_name.to_string()),
        ),
        SpanEventNameRoutingKey::ScopeAttribute(key) => SpanScopeRouting::Fixed(
            span_routing_value_from_scope(scope, key)
                .and_then(|value| resolve_mapped_destination(&mapping.events, &value))
                .unwrap_or_else(|| table_name.to_string()),
        ),
        SpanEventNameRoutingKey::SpanAttribute(key) => SpanScopeRouting::PerSpan { mapping, key },
    }
}

/// Resolve the routed event name for a single span given its scope's routing.
///
/// Returns a [`Cow`] so the common paths (a resource-/scope-level `Fixed` name,
/// or the no-mapping default) borrow the scope-invariant name instead of
/// allocating per span; only a per-span mapping hit owns a new `String`.
fn resolve_span_event_name_in_scope<'a>(
    scope_routing: &'a SpanScopeRouting<'a>,
    span: &Span,
    table_name: &'a str,
) -> Cow<'a, str> {
    match scope_routing {
        SpanScopeRouting::Fixed(name) => Cow::Borrowed(name.as_str()),
        SpanScopeRouting::PerSpan { mapping, key } => {
            match span_routing_value_from_attributes(&span.attributes, key)
                .and_then(|value| resolve_mapped_destination(&mapping.events, &value))
            {
                Some(name) => Cow::Owned(name),
                None => Cow::Borrowed(table_name),
            }
        }
    }
}

/// Resolve span routing for a single span end-to-end. Retained for unit tests;
/// the hot path precomputes scope routing via [`resolve_span_scope_routing`].
#[cfg(test)]
fn resolve_span_event_name(
    resource: Option<&Resource>,
    scope: Option<&InstrumentationScope>,
    span: &Span,
    table_name: &str,
    event_name_mapping: Option<&SpanEventNameMapping>,
) -> String {
    let scope_routing = resolve_span_scope_routing(resource, scope, table_name, event_name_mapping);
    resolve_span_event_name_in_scope(&scope_routing, span, table_name).into_owned()
}

/// Stored client pieces shared by every constructor (all of `Self` except the
/// uploader and encoder), derived once so the constructors can't drift.
struct ClientParts {
    log_table_name: Arc<str>,
    log_event_name_mapping: Option<LogsEventNameMapping>,
    span_table_name: Arc<str>,
    span_event_name_mapping: Option<SpanEventNameMapping>,
    metadata_fields: MetadataFields,
    obo_event_map: Option<OboEventMap>,
}

impl GenevaClient {
    /// Validate any configured event-name mappings before building the client.
    fn validate_event_name_mappings(cfg: &GenevaClientConfig) -> Result<(), String> {
        if let Some(mapping) = cfg
            .logs
            .as_ref()
            .and_then(|logs| logs.event_name_mapping.as_ref())
        {
            mapping.validate()?;
        }
        if let Some(mapping) = cfg
            .spans
            .as_ref()
            .and_then(|spans| spans.event_name_mapping.as_ref())
        {
            mapping.validate()?;
        }
        Ok(())
    }

    /// Derive the shared uploader config + stored parts; constructors differ
    /// only in how they build the `GenevaUploader` from the returned config.
    fn derive_parts(cfg: GenevaClientConfig) -> (GenevaUploaderConfig, ClientParts) {
        let GenevaClientConfig {
            environment,
            namespace,
            config_major_version,
            tenant,
            role_name,
            role_instance,
            logs,
            spans,
            obo_event_map,
            ..
        } = cfg;

        let spans_present = spans.is_some();
        let logs_present = logs.is_some();
        let (log_default_event_name, log_event_name_mapping) = match logs {
            Some(logs) => (logs.default_event_name, logs.event_name_mapping),
            None => (None, None),
        };
        let (span_default_event_name, span_event_name_mapping) = match spans {
            Some(spans) => (spans.default_event_name, spans.event_name_mapping),
            None => (None, None),
        };

        let log_table_name: Arc<str> = log_default_event_name.as_deref().unwrap_or("Log").into();
        let span_table_name: Arc<str> = span_default_event_name.as_deref().unwrap_or("Span").into();

        match log_event_name_mapping.as_ref() {
            Some(mapping) => {
                let routing_key_desc = match &mapping.routing_key {
                    LogsEventNameRoutingKey::EventName => "event_name".to_string(),
                    LogsEventNameRoutingKey::ResourceAttribute(attr) => {
                        format!("resource_attribute({})", attr)
                    }
                    LogsEventNameRoutingKey::ScopeAttribute(attr) => {
                        format!("scope_attribute({})", attr)
                    }
                    LogsEventNameRoutingKey::LogRecordAttribute(attr) => {
                        format!("log_record_attribute({})", attr)
                    }
                };
                let events_desc = mapping
                    .events
                    .iter()
                    .map(|(k, v)| format!("{}→{}", k, v.as_deref().unwrap_or("<source>")))
                    .collect::<Vec<_>>()
                    .join(", ");
                info!(
                    name: "client.new.logs_config",
                    target: "geneva-uploader",
                    default_event_name = %log_default_event_name.as_deref().unwrap_or("<none>"),
                    routing_key = %routing_key_desc,
                    event_mappings = %events_desc,
                    "Configured logs event name routing"
                );
            }
            None if logs_present => {
                info!(
                    name: "client.new.logs_config",
                    target: "geneva-uploader",
                    default_event_name = %log_default_event_name.as_deref().unwrap_or("<none>"),
                    "Configured logs event name routing"
                );
            }
            None => {
                info!(
                    name: "client.new.logs_config",
                    target: "geneva-uploader",
                    "Logs config not initialized; using default values for log events"
                );
            }
        }

        match span_event_name_mapping.as_ref() {
            Some(mapping) => {
                let routing_key_desc = match &mapping.routing_key {
                    SpanEventNameRoutingKey::ResourceAttribute(attr) => {
                        format!("resource_attribute({})", attr)
                    }
                    SpanEventNameRoutingKey::ScopeAttribute(attr) => {
                        format!("scope_attribute({})", attr)
                    }
                    SpanEventNameRoutingKey::SpanAttribute(attr) => {
                        format!("span_attribute({})", attr)
                    }
                };
                let events_desc = mapping
                    .events
                    .iter()
                    .map(|(k, v)| format!("{}→{}", k, v.as_deref().unwrap_or("<source>")))
                    .collect::<Vec<_>>()
                    .join(", ");
                info!(
                    name: "client.new.spans_config",
                    target: "geneva-uploader",
                    default_event_name = %span_default_event_name.as_deref().unwrap_or("<none>"),
                    routing_key = %routing_key_desc,
                    event_mappings = %events_desc,
                    "Configured spans event name routing"
                );
            }
            None if spans_present => {
                info!(
                    name: "client.new.spans_config",
                    target: "geneva-uploader",
                    default_event_name = %span_default_event_name.as_deref().unwrap_or("<none>"),
                    "Configured spans event name routing"
                );
            }
            None => {
                info!(
                    name: "client.new.spans_config",
                    target: "geneva-uploader",
                    "Spans config not initialized; using default values for span events"
                );
            }
        }

        let source_identity = format!(
            "Tenant={}/Role={}/RoleInstance={}",
            tenant, role_name, role_instance
        );
        let config_version = format!("Ver{}v0", config_major_version);

        // Metadata fields that will appear as Bond schema fields in Geneva.
        let metadata_fields = MetadataFields::new(
            environment,
            config_version.clone(),
            tenant,
            role_name,
            role_instance,
            namespace,
            config_version,
        );

        let uploader_config = GenevaUploaderConfig {
            namespace: metadata_fields.namespace.clone(),
            source_identity,
            environment: metadata_fields.env_name.clone(),
            config_version: metadata_fields.event_version.clone(),
        };

        (
            uploader_config,
            ClientParts {
                log_table_name,
                log_event_name_mapping,
                span_table_name,
                span_event_name_mapping,
                metadata_fields,
                obo_event_map,
            },
        )
    }

    /// Assemble the final client from a built uploader and the shared parts.
    fn assemble(uploader: GenevaUploader, parts: ClientParts) -> Self {
        Self {
            uploader: Arc::new(uploader),
            encoder: OtlpEncoder::new(),
            metadata_fields: parts.metadata_fields,
            log_table_name: parts.log_table_name,
            log_event_name_mapping: parts.log_event_name_mapping,
            span_table_name: parts.span_table_name,
            span_event_name_mapping: parts.span_event_name_mapping,
            obo_event_map: parts.obo_event_map,
        }
    }

    pub fn new(cfg: GenevaClientConfig) -> Result<Self, String> {
        info!(
            name: "client.new",
            target: "geneva-uploader",
            endpoint = %cfg.endpoint,
            namespace = %cfg.namespace,
            account = %cfg.account,
            "Initializing GenevaClient"
        );

        Self::validate_event_name_mappings(&cfg)?;

        // Validate MSI resource presence for managed identity variants
        match cfg.auth_method {
            AuthMethod::SystemManagedIdentity
            | AuthMethod::UserManagedIdentity { .. }
            | AuthMethod::UserManagedIdentityByObjectId { .. }
            | AuthMethod::UserManagedIdentityByResourceId { .. } => {
                if cfg.msi_resource.is_none() {
                    debug!(
                        name: "client.new.validate_msi_resource",
                        target: "geneva-uploader",
                        "Validation failed: msi_resource must be provided for managed identity auth"
                    );
                    return Err(
                        "msi_resource must be provided for managed identity auth".to_string()
                    );
                }
            }
            AuthMethod::Certificate { .. } => {}
            AuthMethod::WorkloadIdentity { .. } => {}
            #[cfg(feature = "mock_auth")]
            AuthMethod::MockAuth => {}
        }

        let config_client_config = GenevaConfigClientConfig {
            endpoint: cfg.endpoint.clone(),
            environment: cfg.environment.clone(),
            account: cfg.account.clone(),
            namespace: cfg.namespace.clone(),
            region: cfg.region.clone(),
            config_major_version: cfg.config_major_version,
            auth_method: cfg.auth_method.clone(),
            msi_resource: cfg.msi_resource.clone(),
            #[cfg(test)]
            test_root_ca_pem: None,
        };
        let config_client =
            Arc::new(GenevaConfigClient::new(config_client_config).map_err(|e| {
                debug!(
                    name: "client.new.config_client_init",
                    target: "geneva-uploader",
                    error = %e,
                    "GenevaConfigClient init failed"
                );
                format!("GenevaConfigClient init failed: {e}")
            })?);

        let (uploader_config, parts) = Self::derive_parts(cfg);

        let uploader =
            GenevaUploader::from_config_client(config_client, uploader_config).map_err(|e| {
                debug!(
                    name: "client.new.uploader_init",
                    target: "geneva-uploader",
                    error = %e,
                    "GenevaUploader init failed"
                );
                format!("GenevaUploader init failed: {e}")
            })?;

        info!(
            name: "client.new.complete",
            target: "geneva-uploader",
            "GenevaClient initialized successfully"
        );

        Ok(Self::assemble(uploader, parts))
    }

    /// Agent-fed construction: the host supplies the GIG credential via
    /// `source`, so the uploader skips the GCS config-service handshake entirely.
    /// `cfg.auth_method` / `cfg.msi_resource` are ignored (no `GenevaConfigClient`
    /// is created). The source is queried per upload, so host token rotation is
    /// observed without rebuilding the client.
    pub fn with_agent_fed_source(
        cfg: GenevaClientConfig,
        source: Arc<dyn AgentFedCredentialSource>,
    ) -> Result<Self, String> {
        info!(
            name: "client.new.agent_fed",
            target: "geneva-uploader",
            namespace = %cfg.namespace,
            account = %cfg.account,
            "Initializing GenevaClient (agent-fed credential source)"
        );

        Self::validate_event_name_mappings(&cfg)?;

        let (uploader_config, parts) = Self::derive_parts(cfg);

        let uploader = GenevaUploader::from_agent_fed(source, uploader_config).map_err(|e| {
            debug!(
                name: "client.new.agent_fed.uploader_init",
                target: "geneva-uploader",
                error = %e,
                "GenevaUploader (agent-fed) init failed"
            );
            format!("GenevaUploader init failed: {e}")
        })?;

        Ok(Self::assemble(uploader, parts))
    }

    /// Encode logs from any [`LogsDataView`] implementation into LZ4-chunked
    /// compressed batches, grouped by event name.
    ///
    /// # What to implement
    ///
    /// Implement the following traits from `otap_df_pdata_views`:
    ///
    /// ```text
    /// LogsDataView
    /// └─ ResourceLogsView
    ///    └─ ScopeLogsView
    ///       └─ LogRecordView   ← one impl per log record type
    ///          └─ AnyValueView  (for body / attributes)
    ///          └─ AttributeView (for attributes)
    /// ```
    ///
    /// The `event_name` field on each log record controls which Geneva event
    /// table the record is routed to.  Records with no event name (or an
    /// empty one) are routed to the `"Log"` table.
    ///
    /// # Usage pattern
    ///
    /// ```ignore
    /// let batches = client.encode_and_compress_logs(&my_view)?;
    /// for batch in &batches {
    ///     client.upload_batch(batch).await?;
    /// }
    /// ```
    ///
    /// See `examples/view_basic.rs` for the common `RawLogsData` usage pattern
    /// and `examples/view_advanced.rs` for a full custom `LogsDataView`
    /// implementation.
    pub fn encode_and_compress_logs<T: LogsDataView>(
        &self,
        view: &T,
    ) -> Result<Vec<EncodedBatch>, String> {
        debug!(
            name: "client.encode_and_compress_logs",
            target: "geneva-uploader",
            "Encoding and compressing logs"
        );

        self.encoder
            .encode_logs_from_view(
                view,
                &self.metadata_fields,
                self.log_table_name.as_ref(),
                self.log_event_name_mapping.as_ref(),
                self.obo_event_map.as_ref(),
            )
            .map_err(|e| {
                debug!(
                    name: "client.encode_and_compress_logs.error",
                    target: "geneva-uploader",
                    error = %e,
                    "Logs compression failed"
                );
                format!("Compression failed: {e}")
            })
    }

    /// Encode OTLP spans into LZ4 chunked compressed batches.
    pub fn encode_and_compress_spans(
        &self,
        spans: &[ResourceSpans],
    ) -> Result<Vec<EncodedBatch>, String> {
        debug!(
            name: "client.encode_and_compress_spans",
            target: "geneva-uploader",
            resource_spans_count = spans.len(),
            "Encoding and compressing resource spans"
        );

        let mut routed_groups: Vec<(String, Vec<&Span>)> = Vec::new();
        let mut group_index: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();

        for resource_span in spans {
            let resource = resource_span.resource.as_ref();
            for scope_span in &resource_span.scope_spans {
                let scope = scope_span.scope.as_ref();
                let scope_routing = resolve_span_scope_routing(
                    resource,
                    scope,
                    self.span_table_name.as_ref(),
                    self.span_event_name_mapping.as_ref(),
                );
                for span in &scope_span.spans {
                    let event_name = resolve_span_event_name_in_scope(
                        &scope_routing,
                        span,
                        self.span_table_name.as_ref(),
                    );

                    match group_index.get(event_name.as_ref()) {
                        Some(&idx) => routed_groups[idx].1.push(span),
                        None => {
                            let event_name = event_name.into_owned();
                            group_index.insert(event_name.clone(), routed_groups.len());
                            routed_groups.push((event_name, vec![span]));
                        }
                    }
                }
            }
        }

        let mut batches = Vec::new();
        for (event_name, group_spans) in routed_groups {
            let encoded = self
                .encoder
                .encode_span_batch(
                    group_spans,
                    &self.metadata_fields,
                    &event_name,
                    self.obo_event_map.as_ref(),
                )
                .map_err(|e| {
                    debug!(
                        name: "client.encode_and_compress_spans.error",
                        target: "geneva-uploader",
                        error = %e,
                        event_name = %event_name,
                        "Span compression failed"
                    );
                    format!("Compression failed: {e}")
                })?;
            batches.extend(encoded);
        }

        Ok(batches)
    }

    /// Upload a single compressed batch.
    /// This allows for granular control over uploads, including custom retry logic for individual batches.
    pub async fn upload_batch(&self, batch: &EncodedBatch) -> Result<(), UploadError> {
        debug!(
            name: "client.upload_batch",
            target: "geneva-uploader",
            event_name = %batch.event_name,
            size = batch.data.len(),
            "Uploading batch"
        );

        // Look up per-event OBO config for this batch's event name
        let obo_config = lookup_obo_config(self.obo_event_map.as_ref(), &batch.event_name)
            .filter(|c| c.is_active());

        self.uploader
            .upload(
                batch.data.clone(),
                &batch.event_name,
                &batch.metadata,
                batch.row_count,
                obo_config,
            )
            .await
            .map(|_| {
                debug!(
                    name: "client.upload_batch.success",
                    target: "geneva-uploader",
                    event_name = %batch.event_name,
                    "Successfully uploaded batch"
                );
            })
            .map_err(|e| {
                debug!(
                    name: "client.upload_batch.error",
                    target: "geneva-uploader",
                    event_name = %batch.event_name,
                    error = %e,
                    "Geneva upload failed"
                );
                match e {
                    GenevaUploaderError::UploadFailed {
                        status,
                        retry_after,
                        message,
                    } => UploadError::HttpStatus {
                        status,
                        retry_after,
                        message,
                    },
                    GenevaUploaderError::Http(msg) => UploadError::Transport(msg),
                    other => UploadError::Other(other.to_string()),
                }
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry_proto::tonic::collector::logs::v1::ExportLogsServiceRequest;
    use opentelemetry_proto::tonic::logs::v1::{LogRecord, ResourceLogs, ScopeLogs};
    use opentelemetry_proto::tonic::trace::v1::{ResourceSpans, ScopeSpans, Span};
    use otap_df_pdata::views::otlp::bytes::logs::RawLogsData;
    use prost::Message as _;
    use std::collections::HashMap;

    fn build_config(logs: Option<&str>, spans: Option<&str>) -> GenevaClientConfig {
        GenevaClientConfig {
            endpoint: "https://example.test".to_string(),
            environment: "Test".to_string(),
            account: "acct".to_string(),
            namespace: "ns".to_string(),
            region: "eastus".to_string(),
            config_major_version: 2,
            auth_method: AuthMethod::WorkloadIdentity {
                resource: "https://monitor.azure.com".to_string(),
            },
            tenant: "tenant".to_string(),
            role_name: "role".to_string(),
            role_instance: "instance".to_string(),
            msi_resource: None,
            logs: logs.map(|default_event_name| LogsConfig {
                default_event_name: Some(default_event_name.to_owned()),
                event_name_mapping: None,
            }),
            spans: spans.map(|default_event_name| TracesConfig {
                default_event_name: Some(default_event_name.to_owned()),
                event_name_mapping: None,
            }),
            obo_event_map: None,
        }
    }

    fn build_client(logs: Option<&str>, spans: Option<&str>) -> GenevaClient {
        GenevaClient::new(build_config(logs, spans)).expect("client should initialize")
    }

    fn build_span_client(
        default_event_name: Option<&str>,
        mapping: Option<SpanEventNameMapping>,
    ) -> GenevaClient {
        let mut cfg = build_config(None, default_event_name);
        cfg.spans = Some(TracesConfig {
            default_event_name: default_event_name.map(str::to_owned),
            event_name_mapping: mapping,
        });
        GenevaClient::new(cfg).expect("span client should initialize")
    }

    fn make_span_event_name_mapping(
        routing_key: SpanEventNameRoutingKey,
        events: &[(&str, Option<&str>)],
    ) -> SpanEventNameMapping {
        let mut map = HashMap::new();
        for (source, destination) in events {
            map.insert((*source).to_string(), destination.map(str::to_string));
        }
        SpanEventNameMapping {
            routing_key,
            events: map,
        }
    }

    fn string_attr(key: &str, value: &str) -> KeyValue {
        KeyValue {
            key: key.to_string(),
            key_strindex: 0,
            value: Some(opentelemetry_proto::tonic::common::v1::AnyValue {
                value: Some(
                    opentelemetry_proto::tonic::common::v1::any_value::Value::StringValue(
                        value.to_string(),
                    ),
                ),
            }),
        }
    }

    fn int_attr(key: &str, value: i64) -> KeyValue {
        KeyValue {
            key: key.to_string(),
            key_strindex: 0,
            value: Some(opentelemetry_proto::tonic::common::v1::AnyValue {
                value: Some(
                    opentelemetry_proto::tonic::common::v1::any_value::Value::IntValue(value),
                ),
            }),
        }
    }

    fn double_attr(key: &str, value: f64) -> KeyValue {
        KeyValue {
            key: key.to_string(),
            key_strindex: 0,
            value: Some(opentelemetry_proto::tonic::common::v1::AnyValue {
                value: Some(
                    opentelemetry_proto::tonic::common::v1::any_value::Value::DoubleValue(value),
                ),
            }),
        }
    }

    fn bool_attr(key: &str, value: bool) -> KeyValue {
        KeyValue {
            key: key.to_string(),
            key_strindex: 0,
            value: Some(opentelemetry_proto::tonic::common::v1::AnyValue {
                value: Some(
                    opentelemetry_proto::tonic::common::v1::any_value::Value::BoolValue(value),
                ),
            }),
        }
    }

    fn bytes_attr(key: &str, value: Vec<u8>) -> KeyValue {
        KeyValue {
            key: key.to_string(),
            key_strindex: 0,
            value: Some(opentelemetry_proto::tonic::common::v1::AnyValue {
                value: Some(
                    opentelemetry_proto::tonic::common::v1::any_value::Value::BytesValue(value),
                ),
            }),
        }
    }

    fn build_span_request(
        resource_attrs: Vec<KeyValue>,
        scope_name: Option<&str>,
        scope_attrs: Vec<KeyValue>,
        span_attrs: Vec<KeyValue>,
    ) -> Vec<ResourceSpans> {
        vec![ResourceSpans {
            resource: (!resource_attrs.is_empty()).then_some(Resource {
                attributes: resource_attrs,
                ..Default::default()
            }),
            scope_spans: vec![ScopeSpans {
                scope: (scope_name.is_some() || !scope_attrs.is_empty()).then_some(
                    InstrumentationScope {
                        name: scope_name.unwrap_or_default().to_string(),
                        attributes: scope_attrs,
                        ..Default::default()
                    },
                ),
                spans: vec![Span {
                    attributes: span_attrs,
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        }]
    }

    #[test]
    fn new_rejects_logs_mapping_with_empty_events() {
        let mut cfg = build_config(None, None);
        cfg.logs = Some(LogsConfig {
            default_event_name: None,
            event_name_mapping: Some(LogsEventNameMapping {
                routing_key: LogsEventNameRoutingKey::EventName,
                events: HashMap::new(),
            }),
        });

        let err = match GenevaClient::new(cfg) {
            Ok(_) => panic!("empty mapping events must be rejected"),
            Err(err) => err,
        };
        assert!(err.contains("events must be non-empty"));
    }

    #[test]
    fn new_rejects_spans_mapping_with_empty_events() {
        let mut cfg = build_config(None, None);
        cfg.spans = Some(TracesConfig {
            default_event_name: None,
            event_name_mapping: Some(SpanEventNameMapping {
                routing_key: SpanEventNameRoutingKey::SpanAttribute("cluster".to_string()),
                events: HashMap::new(),
            }),
        });

        let err = match GenevaClient::new(cfg) {
            Ok(_) => panic!("empty span mapping events must be rejected"),
            Err(err) => err,
        };
        assert!(err.contains("events must be non-empty"));
    }

    #[test]
    fn new_rejects_logs_mapping_with_blank_source_key() {
        let mut cfg = build_config(None, None);
        cfg.logs = Some(LogsConfig {
            default_event_name: None,
            event_name_mapping: Some(LogsEventNameMapping {
                routing_key: LogsEventNameRoutingKey::EventName,
                events: HashMap::from([("   ".to_string(), Some("TableA".to_string()))]),
            }),
        });

        let err = match GenevaClient::new(cfg) {
            Ok(_) => panic!("blank source key must be rejected"),
            Err(err) => err,
        };
        assert!(err.contains("source keys must not be blank"));
    }

    #[test]
    fn new_rejects_spans_mapping_with_blank_routing_key_name() {
        let mut cfg = build_config(None, None);
        cfg.spans = Some(TracesConfig {
            default_event_name: None,
            event_name_mapping: Some(SpanEventNameMapping {
                routing_key: SpanEventNameRoutingKey::SpanAttribute("  ".to_string()),
                events: HashMap::from([("cluster-a".to_string(), Some("TraceA".to_string()))]),
            }),
        });

        let err = match GenevaClient::new(cfg) {
            Ok(_) => panic!("blank routing key name must be rejected"),
            Err(err) => err,
        };
        assert!(err.contains("attribute name must not be blank"));
    }

    #[test]
    fn default_event_name_unwrap_or_prefers_override_and_falls_back() {
        let configured = maybe_event_name(true);
        let missing = maybe_event_name(false);

        assert_eq!(configured.unwrap_or("Log"), "AppLog");
        assert_eq!(missing.unwrap_or("Log"), "Log");
    }

    fn maybe_event_name(configured: bool) -> Option<&'static str> {
        if configured {
            Some("AppLog")
        } else {
            None
        }
    }

    #[test]
    fn encode_and_compress_logs_uses_configured_default_event_name() {
        let client = build_client(Some("AppLog"), None);

        let request = ExportLogsServiceRequest {
            resource_logs: vec![ResourceLogs {
                scope_logs: vec![ScopeLogs {
                    log_records: vec![LogRecord::default()],
                    ..Default::default()
                }],
                ..Default::default()
            }],
        };

        let bytes = request.encode_to_vec();
        let view = RawLogsData::new(&bytes);
        let batches = client
            .encode_and_compress_logs(&view)
            .expect("log encoding should succeed");

        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].event_name, "AppLog");
    }

    #[test]
    fn encode_and_compress_spans_uses_configured_default_event_name() {
        let client = build_client(None, Some("AppTrace"));

        let spans = vec![ResourceSpans {
            scope_spans: vec![ScopeSpans {
                spans: vec![Span {
                    name: "span-name".to_string(),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        }];

        let batches = client
            .encode_and_compress_spans(&spans)
            .expect("span encoding should succeed");

        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].event_name, "AppTrace");
    }

    #[test]
    fn encode_and_compress_spans_no_mapping_shares_default_name_across_spans() {
        // No routing mapping: every span in the scope resolves to the default
        // table name via a borrowed Cow. Multiple spans must collapse into one
        // batch under that name without per-span ownership.
        let client = build_client(None, Some("AppTrace"));

        let spans = vec![ResourceSpans {
            scope_spans: vec![ScopeSpans {
                spans: vec![
                    Span {
                        name: "span-1".to_string(),
                        ..Default::default()
                    },
                    Span {
                        name: "span-2".to_string(),
                        ..Default::default()
                    },
                    Span {
                        name: "span-3".to_string(),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }],
            ..Default::default()
        }];

        let batches = client
            .encode_and_compress_spans(&spans)
            .expect("span encoding should succeed");

        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].event_name, "AppTrace");
        assert_eq!(batches[0].row_count, 3);
    }

    #[test]
    fn encode_and_compress_spans_routes_by_resource_scope_and_span_attributes() {
        let client = build_span_client(
            Some("AppTrace"),
            Some(make_span_event_name_mapping(
                SpanEventNameRoutingKey::ResourceAttribute("cluster".to_string()),
                &[("cluster-a", Some("ResourceTrace"))],
            )),
        );

        let resource_spans = build_span_request(
            vec![string_attr("cluster", "cluster-a")],
            Some("scope-a"),
            Vec::new(),
            Vec::new(),
        );

        let batches = client
            .encode_and_compress_spans(&resource_spans)
            .expect("span encoding should succeed");

        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].event_name, "ResourceTrace");
    }

    #[test]
    fn encode_and_compress_spans_resource_route_shared_across_spans_in_scope() {
        // The resource-level routing key is scope-invariant, so it is resolved
        // once per scope and reused for every span; multiple spans in one scope
        // must all land in a single batch under the resolved event name.
        let client = build_span_client(
            Some("AppTrace"),
            Some(make_span_event_name_mapping(
                SpanEventNameRoutingKey::ResourceAttribute("cluster".to_string()),
                &[("cluster-a", Some("ResourceTrace"))],
            )),
        );

        let resource_spans = vec![ResourceSpans {
            resource: Some(Resource {
                attributes: vec![string_attr("cluster", "cluster-a")],
                ..Default::default()
            }),
            scope_spans: vec![ScopeSpans {
                scope: Some(InstrumentationScope {
                    name: "scope-a".to_string(),
                    ..Default::default()
                }),
                spans: vec![
                    Span {
                        name: "span-1".to_string(),
                        ..Default::default()
                    },
                    Span {
                        name: "span-2".to_string(),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }],
            ..Default::default()
        }];

        let batches = client
            .encode_and_compress_spans(&resource_spans)
            .expect("span encoding should succeed");

        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].event_name, "ResourceTrace");
        assert_eq!(batches[0].row_count, 2);
    }

    #[test]
    fn encode_and_compress_spans_routes_by_scope_attribute() {
        let client = build_span_client(
            Some("AppTrace"),
            Some(make_span_event_name_mapping(
                SpanEventNameRoutingKey::ScopeAttribute("cluster".to_string()),
                &[("scope-a", Some("ScopeTrace"))],
            )),
        );

        let resource_spans = build_span_request(
            Vec::new(),
            Some("scope-a"),
            vec![string_attr("cluster", "scope-a")],
            Vec::new(),
        );

        let batches = client
            .encode_and_compress_spans(&resource_spans)
            .expect("span encoding should succeed");

        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].event_name, "ScopeTrace");
    }

    #[test]
    fn encode_and_compress_spans_routes_by_scope_name_with_passthrough_destination() {
        let client = build_span_client(
            Some("AppTrace"),
            Some(make_span_event_name_mapping(
                SpanEventNameRoutingKey::ScopeAttribute("scope.name".to_string()),
                &[("scope-a", Some(""))],
            )),
        );

        let resource_spans =
            build_span_request(Vec::new(), Some("scope-a"), Vec::new(), Vec::new());

        let batches = client
            .encode_and_compress_spans(&resource_spans)
            .expect("span encoding should succeed");

        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].event_name, "scope-a");
    }

    #[test]
    fn encode_and_compress_spans_routes_by_span_attribute_and_splits_batches() {
        let client = build_span_client(
            Some("AppTrace"),
            Some(make_span_event_name_mapping(
                SpanEventNameRoutingKey::SpanAttribute("cluster".to_string()),
                &[
                    ("cluster-a", Some("SpanTraceA")),
                    ("cluster-b", Some("SpanTraceB")),
                ],
            )),
        );

        let resource_spans = vec![ResourceSpans {
            scope_spans: vec![ScopeSpans {
                scope: Some(InstrumentationScope {
                    name: "scope-a".to_string(),
                    ..Default::default()
                }),
                spans: vec![
                    Span {
                        attributes: vec![string_attr("cluster", "cluster-a")],
                        ..Default::default()
                    },
                    Span {
                        attributes: vec![string_attr("cluster", "cluster-b")],
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }],
            ..Default::default()
        }];

        let batches = client
            .encode_and_compress_spans(&resource_spans)
            .expect("span encoding should succeed");

        let mut event_names: Vec<String> = batches
            .iter()
            .map(|batch| batch.event_name.clone())
            .collect();
        event_names.sort();
        assert_eq!(
            event_names,
            vec!["SpanTraceA".to_string(), "SpanTraceB".to_string()]
        );
    }

    #[test]
    fn encode_and_compress_spans_missing_mapping_key_falls_back_to_default_event_name() {
        let client = build_span_client(
            Some("AppTrace"),
            Some(make_span_event_name_mapping(
                SpanEventNameRoutingKey::SpanAttribute("cluster".to_string()),
                &[("known", Some("SpanTrace"))],
            )),
        );

        let resource_spans = build_span_request(
            Vec::new(),
            Some("scope-a"),
            Vec::new(),
            vec![string_attr("cluster", "unknown")],
        );

        let batches = client
            .encode_and_compress_spans(&resource_spans)
            .expect("span encoding should succeed");

        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].event_name, "AppTrace");
    }

    #[test]
    fn encode_and_compress_spans_routes_by_non_string_attribute_value() {
        // Non-string span attribute values are stringified before the mapping lookup.
        let client = build_span_client(
            Some("AppTrace"),
            Some(make_span_event_name_mapping(
                SpanEventNameRoutingKey::SpanAttribute("code".to_string()),
                &[("42", Some("IntTrace"))],
            )),
        );

        let resource_spans = build_span_request(
            Vec::new(),
            Some("scope-a"),
            Vec::new(),
            vec![int_attr("code", 42)],
        );

        let batches = client
            .encode_and_compress_spans(&resource_spans)
            .expect("span encoding should succeed");

        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].event_name, "IntTrace");
    }

    #[test]
    fn span_routing_value_from_attributes_handles_all_value_types() {
        let attrs = vec![
            string_attr("s", "sv"),
            int_attr("i", 7),
            double_attr("d", 2.5),
            bool_attr("b", true),
            bytes_attr("raw", b"xyz".to_vec()),
        ];

        assert_eq!(
            span_routing_value_from_attributes(&attrs, "s").as_deref(),
            Some("sv")
        );
        assert_eq!(
            span_routing_value_from_attributes(&attrs, "i").as_deref(),
            Some("7")
        );
        assert_eq!(
            span_routing_value_from_attributes(&attrs, "d").as_deref(),
            Some("2.5")
        );
        assert_eq!(
            span_routing_value_from_attributes(&attrs, "b").as_deref(),
            Some("true")
        );
        // Unsupported value types (bytes/array/kvlist) and missing keys yield None.
        assert_eq!(span_routing_value_from_attributes(&attrs, "raw"), None);
        assert_eq!(span_routing_value_from_attributes(&attrs, "absent"), None);
        // Blank/whitespace string values are treated as absent.
        let blank = vec![string_attr("s", "   ")];
        assert_eq!(span_routing_value_from_attributes(&blank, "s"), None);
    }

    #[test]
    fn span_routing_value_from_scope_reads_name_version_and_attributes() {
        let scope = InstrumentationScope {
            name: "scope-a".to_string(),
            version: "1.2.3".to_string(),
            attributes: vec![string_attr("cluster", "clusterA")],
            ..Default::default()
        };

        assert_eq!(
            span_routing_value_from_scope(Some(&scope), SCOPE_NAME_ROUTING_KEY).as_deref(),
            Some("scope-a")
        );
        assert_eq!(
            span_routing_value_from_scope(Some(&scope), SCOPE_VERSION_ROUTING_KEY).as_deref(),
            Some("1.2.3")
        );
        assert_eq!(
            span_routing_value_from_scope(Some(&scope), "cluster").as_deref(),
            Some("clusterA")
        );
        assert_eq!(span_routing_value_from_scope(Some(&scope), "absent"), None);
        // A missing scope yields None.
        assert_eq!(
            span_routing_value_from_scope(None, SCOPE_NAME_ROUTING_KEY),
            None
        );
        // Blank scope name/version are treated as absent.
        let blank = InstrumentationScope {
            name: "  ".to_string(),
            version: String::new(),
            ..Default::default()
        };
        assert_eq!(
            span_routing_value_from_scope(Some(&blank), SCOPE_NAME_ROUTING_KEY),
            None
        );
        assert_eq!(
            span_routing_value_from_scope(Some(&blank), SCOPE_VERSION_ROUTING_KEY),
            None
        );
    }

    #[test]
    fn span_routing_value_from_resource_reads_attributes_or_none() {
        let resource = Resource {
            attributes: vec![string_attr("region", "eastus")],
            ..Default::default()
        };

        assert_eq!(
            span_routing_value_from_resource(Some(&resource), "region").as_deref(),
            Some("eastus")
        );
        assert_eq!(
            span_routing_value_from_resource(Some(&resource), "absent"),
            None
        );
        // A missing resource yields None.
        assert_eq!(span_routing_value_from_resource(None, "region"), None);
    }

    #[test]
    fn resolve_span_event_name_falls_back_and_passes_through() {
        let span = Span {
            attributes: vec![string_attr("cluster", "clusterA")],
            ..Default::default()
        };

        // No mapping configured -> default table name.
        assert_eq!(
            resolve_span_event_name(None, None, &span, "Span", None),
            "Span"
        );

        let mapping = make_span_event_name_mapping(
            SpanEventNameRoutingKey::SpanAttribute("cluster".to_string()),
            &[("clusterA", Some("")), ("clusterB", Some("Premium"))],
        );

        // An empty destination passes the source value through unchanged.
        assert_eq!(
            resolve_span_event_name(None, None, &span, "Span", Some(&mapping)),
            "clusterA"
        );

        // The routing attribute is absent -> fall back to the default table name.
        let other = Span {
            attributes: vec![string_attr("region", "eastus")],
            ..Default::default()
        };
        assert_eq!(
            resolve_span_event_name(None, None, &other, "Span", Some(&mapping)),
            "Span"
        );
    }

    #[test]
    fn new_logs_client_accepts_all_attribute_routing_kinds() {
        // Exercises GenevaClient::new's logs-mapping description/validation for each
        // attribute routing kind (resource/scope/log-record).
        for routing_key in [
            LogsEventNameRoutingKey::ResourceAttribute("res".to_string()),
            LogsEventNameRoutingKey::ScopeAttribute("scope".to_string()),
            LogsEventNameRoutingKey::LogRecordAttribute("rec".to_string()),
        ] {
            let mut cfg = build_config(Some("AppLog"), None);
            cfg.logs = Some(LogsConfig {
                default_event_name: Some("AppLog".to_string()),
                event_name_mapping: Some(LogsEventNameMapping {
                    routing_key,
                    events: HashMap::from([("src".to_string(), Some("Dest".to_string()))]),
                }),
            });
            GenevaClient::new(cfg).expect("logs client with attribute routing should initialize");
        }
    }

    #[test]
    fn encode_and_compress_spans_groups_multiple_spans_with_same_route() {
        // Two spans resolving to the same destination share a single grouped batch,
        // exercising the "append to existing group" path.
        let client = build_span_client(
            Some("AppTrace"),
            Some(make_span_event_name_mapping(
                SpanEventNameRoutingKey::SpanAttribute("cluster".to_string()),
                &[("clusterA", Some("TraceA"))],
            )),
        );

        let resource_spans = vec![ResourceSpans {
            resource: None,
            scope_spans: vec![ScopeSpans {
                scope: Some(InstrumentationScope {
                    name: "s".to_string(),
                    ..Default::default()
                }),
                spans: vec![
                    Span {
                        attributes: vec![string_attr("cluster", "clusterA")],
                        ..Default::default()
                    },
                    Span {
                        attributes: vec![string_attr("cluster", "clusterA")],
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }],
            ..Default::default()
        }];

        let batches = client
            .encode_and_compress_spans(&resource_spans)
            .expect("span encoding should succeed");

        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].event_name, "TraceA");
        assert_eq!(batches[0].row_count, 2);
    }

    #[test]
    fn encode_and_compress_logs_uses_default_table_name_when_logs_config_absent() {
        let client = GenevaClient::new(GenevaClientConfig {
            endpoint: "https://example.test".to_string(),
            environment: "Test".to_string(),
            account: "acct".to_string(),
            namespace: "ns".to_string(),
            region: "eastus".to_string(),
            config_major_version: 2,
            auth_method: AuthMethod::WorkloadIdentity {
                resource: "https://monitor.azure.com".to_string(),
            },
            tenant: "tenant".to_string(),
            role_name: "role".to_string(),
            role_instance: "instance".to_string(),
            msi_resource: None,
            logs: None,
            spans: None,
            obo_event_map: None,
        })
        .expect("client should initialize without optional log config");

        let request = ExportLogsServiceRequest {
            resource_logs: vec![ResourceLogs {
                scope_logs: vec![ScopeLogs {
                    log_records: vec![LogRecord::default()],
                    ..Default::default()
                }],
                ..Default::default()
            }],
        };
        let bytes = request.encode_to_vec();
        let view = RawLogsData::new(&bytes);
        let batches = client
            .encode_and_compress_logs(&view)
            .expect("log encoding should succeed");

        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].event_name, "Log");
    }

    #[test]
    fn encode_and_compress_spans_uses_default_table_name_when_spans_config_absent() {
        let client = GenevaClient::new(GenevaClientConfig {
            endpoint: "https://example.test".to_string(),
            environment: "Test".to_string(),
            account: "acct".to_string(),
            namespace: "ns".to_string(),
            region: "eastus".to_string(),
            config_major_version: 2,
            auth_method: AuthMethod::WorkloadIdentity {
                resource: "https://monitor.azure.com".to_string(),
            },
            tenant: "tenant".to_string(),
            role_name: "role".to_string(),
            role_instance: "instance".to_string(),
            msi_resource: None,
            logs: None,
            spans: None,
            obo_event_map: None,
        })
        .expect("client should initialize without optional span config");

        let spans = vec![ResourceSpans {
            scope_spans: vec![ScopeSpans {
                spans: vec![Span {
                    name: "span-name".to_string(),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        }];

        let batches = client
            .encode_and_compress_spans(&spans)
            .expect("span encoding should succeed");

        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].event_name, "Span");
    }

    #[test]
    fn with_agent_fed_source_builds_usable_client() {
        #[derive(Debug)]
        struct StubSource;
        impl AgentFedCredentialSource for StubSource {
            fn current(&self) -> AgentFedCredentialFuture<'_> {
                Box::pin(async {
                    Some(AgentFedCredential {
                        token: "secret-token".to_string(),
                        endpoint: "https://ingest.example".to_string(),
                        moniker: "moniker".to_string(),
                    })
                })
            }
        }

        let client = GenevaClient::with_agent_fed_source(
            build_config(Some("AppLog"), None),
            Arc::new(StubSource),
        )
        .expect("agent-fed client should initialize");

        // Prove the agent-fed client is usable end-to-end for encoding, just
        // like a config-service client.
        let request = ExportLogsServiceRequest {
            resource_logs: vec![ResourceLogs {
                scope_logs: vec![ScopeLogs {
                    log_records: vec![LogRecord::default()],
                    ..Default::default()
                }],
                ..Default::default()
            }],
        };
        let bytes = request.encode_to_vec();
        let view = RawLogsData::new(&bytes);
        let batches = client
            .encode_and_compress_logs(&view)
            .expect("log encoding should succeed");
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].event_name, "AppLog");
    }

    #[test]
    fn agent_fed_credential_debug_redacts_token() {
        let cred = AgentFedCredential {
            token: "top-secret".to_string(),
            endpoint: "https://ingest.example".to_string(),
            moniker: "moniker".to_string(),
        };
        let rendered = format!("{cred:?}");
        assert!(
            !rendered.contains("top-secret"),
            "token must be redacted: {rendered}"
        );
        assert!(rendered.contains("<redacted>"));
        assert!(rendered.contains("moniker"));
    }
}
