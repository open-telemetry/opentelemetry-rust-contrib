//! AWS X-Ray translator module for converting OpenTelemetry spans to X-Ray segments.
//!
//! This module provides the core functionality for translating OpenTelemetry spans
//! into AWS X-Ray segments and subsegments, handling various AWS service metadata
//! and maintaining compatibility with AWS X-Ray's data format.

mod attribute_processing;

#[cfg(feature = "subsegment-nesting")]
mod document_builder_tree;
#[cfg(feature = "subsegment-nesting")]
use document_builder_tree::DocumentBuilderHeaderTree;

pub(crate) mod error;
mod utils;

use utils::{sanitize_annotation_key, translate_timestamp};

use opentelemetry::{trace::SpanKind, KeyValue, SpanId};
use opentelemetry_sdk::{trace::SpanData, Resource};

use crate::xray_exporter::{
    translator::attribute_processing::{
        get_annotation, get_any_value,
        value_builder::{
            AnyValueBuilder, AwsOperationBuilder, AwsXraySdkBuilder, BeanstalkDeploymentIdBuilder,
            CauseBuilder, CloudwatchLogGroupBuilder, HttpRequestUrlBuilder,
            HttpResponseContentLengthBuilder, SegmentNameBuilder, SegmentOriginBuilder,
            SqlUrlBuilder, SubsegmentNamespaceBuilder, ValueBuilder,
        },
        DispatchTable, SpanAttributeProcessor,
    },
    types::{
        DocumentBuilder, DocumentBuilderType, SegmentDocument, SegmentDocumentBuilder,
        SubsegmentDocumentBuilder,
    },
};

use error::{Result, TranslationError};

/// Translator for converting OpenTelemetry spans to AWS X-Ray segment documents.
///
/// This translator handles the conversion of OpenTelemetry's tracing data model into
/// AWS X-Ray's segment format. It processes span attributes, maps them to X-Ray fields,
/// and handles AWS-specific metadata such as service information, HTTP data, and SQL queries.
///
/// # Configuration
///
/// The translator can be configured to control which attributes are indexed (searchable
/// in X-Ray), which log groups to associate with segments, and whether to perform
/// timestamp validation of the [opentelemetry::TraceId].
///
/// # Attribute Processing
///
/// Attributes are processed through the following pipeline:
///
/// 1. **Recognized attributes** are mapped to specific X-Ray fields (e.g., `http.method` → `http.request.method`)
/// 2. **Prefix routing** — attributes with special prefixes are force-routed:
///    - `annotation.` prefix → indexed as an annotation (prefix stripped, e.g., `annotation.user_id` → annotation `user_id`)
///    - `metadata.` prefix → added as metadata (prefix stripped, e.g., `metadata.debug_info` → metadata `debug_info`)
/// 3. **Indexed attributes** (configured via [`index_all_attrs`], [`with_indexed_attr`], or
///    [`with_indexed_attrs`]) are added as annotations (searchable, max 50 per segment).
///    If annotation insertion fails (incompatible type or limit reached), the attribute
///    falls back to metadata.
/// 4. **Metadata attributes** — remaining attributes are added as metadata only if at least
///    one of the following is true:
///    - [`metadata_all_attrs`] is enabled
///    - The attribute key is in the explicit metadata list (see [`with_metadata_attr`])
///    - The attribute was requested for indexing but failed (fallback)
///    - The attribute has a `metadata.` prefix
///
/// [`index_all_attrs`]: SegmentTranslator::index_all_attrs
/// [`with_indexed_attr`]: SegmentTranslator::with_indexed_attr
/// [`with_indexed_attrs`]: SegmentTranslator::with_indexed_attrs
/// [`metadata_all_attrs`]: SegmentTranslator::metadata_all_attrs
/// [`with_metadata_attr`]: SegmentTranslator::with_metadata_attr
///
/// # Examples
///
/// Basic usage:
///
/// ```
/// use opentelemetry_aws::xray_exporter::SegmentTranslator;
///
/// let translator = SegmentTranslator::new();
/// ```
///
/// With indexed attributes:
///
/// ```
/// use opentelemetry_aws::xray_exporter::SegmentTranslator;
///
/// let translator = SegmentTranslator::new()
///     .with_indexed_attr("service.name".to_string())
///     .with_indexed_attr("http.method".to_string())
///     .with_indexed_attr("http.status_code".to_string());
/// ```
///
/// Index all attributes as annotations:
///
/// ```
/// use opentelemetry_aws::xray_exporter::SegmentTranslator;
///
/// let translator = SegmentTranslator::new()
///     .index_all_attrs();
/// ```
///
/// With CloudWatch log groups:
///
/// ```
/// use opentelemetry_aws::xray_exporter::SegmentTranslator;
///
/// let translator = SegmentTranslator::new()
///     .with_log_group_name("/aws/lambda/my-function".to_string())
///     .with_log_group_name("/aws/ecs/my-service".to_string());
/// ```
#[derive(Debug)]
pub struct SegmentTranslator {
    indexed_attrs: Vec<String>,
    index_all_attrs: bool,
    metadata_attrs: Vec<String>,
    metadata_all_attrs: bool,
    log_group_names: Vec<String>,
    skip_timestamp_validation: bool,
    #[cfg(feature = "subsegment-nesting")]
    always_nest_subsegments: bool,
    resource: Option<Resource>,
    dispatch_table: DispatchTable,
}

impl Default for SegmentTranslator {
    fn default() -> Self {
        Self::new()
    }
}

/// An enum that holds either a segment or subsegment builder.
///
/// This type is used internally during the translation process to handle both
/// segments (created from server spans) and subsegments (created from other span kinds)
/// in a unified way.
///
/// The translator creates the appropriate builder type based on the span kind:
/// - [`SpanKind::Server`] creates a [`Segment`] variant
/// - All other span kinds create a [`Subsegment`] variant
///
/// [`SpanKind::Server`]: opentelemetry::trace::SpanKind::Server
/// [`Segment`]: AnyDocumentBuilder::Segment
/// [`Subsegment`]: AnyDocumentBuilder::Subsegment
#[derive(Debug)]
enum AnyDocumentBuilder<'span> {
    /// A segment document builder (for server spans).
    Segment(SegmentDocumentBuilder<'span>),
    /// A subsegment document builder (for client, internal, producer, and consumer spans).
    Subsegment(SubsegmentDocumentBuilder<'span>),
}

impl<'span> AnyDocumentBuilder<'span> {
    fn build(self) -> Result<SegmentDocument<'span>> {
        match self {
            AnyDocumentBuilder::Segment(builder) => Ok(builder.build()?),
            AnyDocumentBuilder::Subsegment(builder) => Ok(builder.build()?),
        }
    }
}

/// [AnyDocumentBuilder] will be registered on the [DispatchTable] using
/// the [ValueBuilder] count. Said another way, it uses the next avaiable
/// [ProcessorId] of the [DispatchTable].
///
/// [ValueBuilder]: attribute_processing::value_builder::ValueBuilder
/// [ProcessorId]: attribute_processing::ProcessorId
const ANY_DOCUMENT_BUILDER_PROCESSOR_ID: usize = AnyValueBuilder::count();

impl SegmentTranslator {
    /// Creates a new segment translator with default configuration.
    ///
    /// # Examples
    ///
    /// ```
    /// use opentelemetry_aws::xray_exporter::SegmentTranslator;
    ///
    /// let translator = SegmentTranslator::new()
    ///     .with_indexed_attr("service.name".to_string())
    ///     .with_log_group_name("/aws/lambda/my-function".to_string());
    /// ```
    pub fn new() -> Self {
        let mut st = Self {
            indexed_attrs: Default::default(),
            index_all_attrs: Default::default(),
            metadata_attrs: Default::default(),
            metadata_all_attrs: Default::default(),
            log_group_names: Default::default(),
            skip_timestamp_validation: Default::default(),
            #[cfg(feature = "subsegment-nesting")]
            always_nest_subsegments: Default::default(),
            resource: Default::default(),
            dispatch_table: Default::default(),
        };

        // Register the additional builders on the dispatch table
        AnyValueBuilder::register_builders(&mut st.dispatch_table);

        // Register the AnyDocumentBuilder with the next available ProcessorId
        // i.e. the AnyValueBuilder count
        st.dispatch_table.register::<{
            const fn __len<T, const N: usize>(_: &[T; N]) -> usize {
                N
            }
            __len(&AnyDocumentBuilder::HANDLERS)
        }, AnyDocumentBuilder>(ANY_DOCUMENT_BUILDER_PROCESSOR_ID);

        st
    }

    /// Enables indexing of all attributes as annotations.
    ///
    /// All span attributes not mapped to specific X-Ray fields will be added as annotations
    /// (searchable). X-Ray has a limit of 50 annotations per segment; excess attributes
    /// will be added as metadata.
    ///
    /// # Examples
    ///
    /// ```
    /// use opentelemetry_aws::xray_exporter::SegmentTranslator;
    ///
    /// let translator = SegmentTranslator::new()
    ///     .index_all_attrs();
    /// ```
    pub fn index_all_attrs(mut self) -> Self {
        self.index_all_attrs = true;
        self
    }

    /// Enables routing all non-recognized, non-indexed attributes to X-Ray metadata.
    ///
    /// When enabled, every span attribute that is not mapped to a specific X-Ray field
    /// and not successfully indexed as an annotation will be added as metadata
    /// (not searchable, unlimited).
    ///
    /// If [`index_all_attrs`] is also enabled, this flag is redundant: attributes
    /// that fail annotation insertion already fall back to metadata automatically.
    ///
    /// [`index_all_attrs`]: SegmentTranslator::index_all_attrs
    ///
    /// # Examples
    ///
    /// ```
    /// use opentelemetry_aws::xray_exporter::SegmentTranslator;
    ///
    /// let translator = SegmentTranslator::new()
    ///     .metadata_all_attrs();
    /// ```
    pub fn metadata_all_attrs(mut self) -> Self {
        self.metadata_all_attrs = true;
        self
    }

    /// Skips timestamp validation during translation.
    ///
    /// Disables validation that trace ID timestamps are within acceptable bounds
    /// (not more than 30 days old or 5 minutes in the future). Useful for testing
    /// or processing historical data.
    ///
    /// # Examples
    ///
    /// ```
    /// use opentelemetry_aws::xray_exporter::SegmentTranslator;
    ///
    /// let translator = SegmentTranslator::new()
    ///     .skip_timestamp_validation();
    /// ```
    pub fn skip_timestamp_validation(mut self) -> Self {
        self.skip_timestamp_validation = true;
        self
    }

    #[cfg(feature = "subsegment-nesting")]
    /// Enables nesting of subsegments within their parent segments.
    ///
    /// When enabled, subsegments are nested within their parent segments or subsegments
    /// in the batch, establishing a hierarchical structure. Subsegments without parents
    /// in the batch are returned as top-level documents.
    ///
    /// This also sets precursor IDs for sequential subsegments that share the same parent,
    /// allowing X-Ray to visualize the execution order of sibling subsegments.
    ///
    /// Note that if you are exporting to the X-Ray service, this should not be needed
    /// as it takes care of recomposing the traces nesting by itself.
    ///
    /// # Examples
    ///
    /// ```
    /// use opentelemetry_aws::xray_exporter::SegmentTranslator;
    ///
    /// let translator = SegmentTranslator::new()
    ///     .always_nest_subsegments();
    /// ```
    pub fn always_nest_subsegments(mut self) -> Self {
        self.always_nest_subsegments = true;
        self
    }

    /// Adds a single attribute to be indexed as an annotation.
    ///
    /// Indexed attributes become searchable annotations in the X-Ray console.
    /// Limit of 50 annotations per segment.
    ///
    /// # Examples
    ///
    /// ```
    /// use opentelemetry_aws::xray_exporter::SegmentTranslator;
    ///
    /// let translator = SegmentTranslator::new()
    ///     .with_indexed_attr("service.name".to_string())
    ///     .with_indexed_attr("http.method".to_string());
    /// ```
    pub fn with_indexed_attr(mut self, attr: String) -> Self {
        // Keep the indexed_attr sorted
        // If the attribute is already present (Result::Ok), skip it
        if let Err(i) = self.indexed_attrs.binary_search(&attr) {
            self.indexed_attrs.insert(i, attr);
        }
        self
    }

    /// Adds multiple attributes to be indexed as annotations.
    ///
    /// # Examples
    ///
    /// ```
    /// use opentelemetry_aws::xray_exporter::SegmentTranslator;
    ///
    /// let attrs = vec![
    ///     "service.name".to_string(),
    ///     "http.method".to_string(),
    ///     "http.status_code".to_string(),
    /// ];
    ///
    /// let translator = SegmentTranslator::new()
    ///     .with_indexed_attrs(attrs);
    /// ```
    pub fn with_indexed_attrs(mut self, attrs: impl IntoIterator<Item = String>) -> Self {
        for attr in attrs {
            // Keep the indexed_attr sorted
            // If the attribute is already present (Result::Ok), skip it
            if let Err(i) = self.indexed_attrs.binary_search(&attr) {
                self.indexed_attrs.insert(i, attr);
            }
        }
        self
    }

    /// Adds a single attribute key to be explicitly routed to X-Ray metadata.
    ///
    /// Attributes whose keys match will be added as metadata (not searchable,
    /// unlimited) regardless of other configuration. Duplicate keys are ignored.
    /// Lookup uses binary search for efficiency.
    ///
    /// See [`metadata_all_attrs`] for routing *all* remaining attributes to metadata.
    ///
    /// [`metadata_all_attrs`]: SegmentTranslator::metadata_all_attrs
    ///
    /// # Examples
    ///
    /// ```
    /// use opentelemetry_aws::xray_exporter::SegmentTranslator;
    ///
    /// let translator = SegmentTranslator::new()
    ///     .with_metadata_attr("custom.field".to_string());
    /// ```
    pub fn with_metadata_attr(mut self, attr: String) -> Self {
        // Keep the metadata_attrs sorted
        // If the attribute is already present (Result::Ok), skip it
        if let Err(i) = self.metadata_attrs.binary_search(&attr) {
            self.metadata_attrs.insert(i, attr);
        }
        self
    }
    /// Adds multiple attribute keys to be explicitly routed to X-Ray metadata.
    ///
    /// This is a convenience method equivalent to calling [`with_metadata_attr`]
    /// for each key in the iterator. Duplicate keys are ignored.
    ///
    /// [`with_metadata_attr`]: SegmentTranslator::with_metadata_attr
    ///
    /// # Examples
    ///
    /// ```
    /// use opentelemetry_aws::xray_exporter::SegmentTranslator;
    ///
    /// let translator = SegmentTranslator::new()
    ///     .with_metadata_attrs(vec![
    ///         "custom.field".to_string(),
    ///         "another.field".to_string(),
    ///     ]);
    /// ```
    pub fn with_metadata_attrs(mut self, attrs: impl IntoIterator<Item = String>) -> Self {
        for attr in attrs {
            // Keep the metadata_attrs sorted
            // If the attribute is already present (Result::Ok), skip it
            if let Err(i) = self.metadata_attrs.binary_search(&attr) {
                self.metadata_attrs.insert(i, attr);
            }
        }
        self
    }

    /// Sets the indexed attributes, replacing any previously configured.
    ///
    /// # Examples
    ///
    /// ```
    /// use opentelemetry_aws::xray_exporter::SegmentTranslator;
    ///
    /// let translator = SegmentTranslator::new()
    ///     .set_indexed_attrs(vec!["service.name".to_string()]);
    /// ```
    pub fn set_indexed_attrs(mut self, mut indexed_attrs: Vec<String>) -> Self {
        // Ensure it is sorted
        indexed_attrs.sort();
        self.indexed_attrs = indexed_attrs;
        self
    }

    /// Adds a CloudWatch log group name to associate with segments.
    ///
    /// Allows X-Ray to link traces with CloudWatch logs.
    ///
    /// # Examples
    ///
    /// ```
    /// use opentelemetry_aws::xray_exporter::SegmentTranslator;
    ///
    /// let translator = SegmentTranslator::new()
    ///     .with_log_group_name("/aws/lambda/my-function".to_string());
    /// ```
    pub fn with_log_group_name(mut self, log_group_name: String) -> Self {
        self.log_group_names.push(log_group_name);
        self
    }

    /// Adds multiple CloudWatch log group names.
    ///
    /// # Examples
    ///
    /// ```
    /// use opentelemetry_aws::xray_exporter::SegmentTranslator;
    ///
    /// let log_groups = vec![
    ///     "/aws/lambda/function1".to_string(),
    ///     "/aws/lambda/function2".to_string(),
    /// ];
    ///
    /// let translator = SegmentTranslator::new()
    ///     .with_log_group_names(log_groups);
    /// ```
    pub fn with_log_group_names(
        mut self,
        log_group_names: impl IntoIterator<Item = String>,
    ) -> Self {
        self.log_group_names.extend(log_group_names);
        self
    }

    /// Sets the CloudWatch log group names, replacing any previously configured.
    ///
    /// # Examples
    ///
    /// ```
    /// use opentelemetry_aws::xray_exporter::SegmentTranslator;
    ///
    /// let translator = SegmentTranslator::new()
    ///     .set_log_group_names(vec!["/aws/lambda/my-function".to_string()]);
    /// ```
    pub fn set_log_group_names(mut self, log_group_names: Vec<String>) -> Self {
        self.log_group_names = log_group_names;
        self
    }

    /// Sets the resource for this translator.
    ///
    /// Resource attributes (e.g., service name, version, deployment environment) apply to
    /// all spans and are processed along with span attributes during translation.
    ///
    /// Typically called by [`XrayExporter`] when the resource is set on the tracer provider.
    ///
    /// [`XrayExporter`]: crate::xray_exporter::XrayExporter
    pub fn set_resource(&mut self, resource: &Resource) {
        self.resource.replace(resource.clone());
    }
}

impl SegmentTranslator {
    /// Translates a batch of OpenTelemetry spans into X-Ray segment documents.
    ///
    /// Converts spans to segment/subsegment documents based on span kind, establishes
    /// parent-child relationships, nests subsegments within parents when present, and
    /// sets precursor IDs for sequential subsegments.
    ///
    /// Spans that fail translation (e.g., due to missing required fields, constraint
    /// violations, or timestamp validation failures) are silently dropped. When the
    /// `internal-logs` feature is enabled, these failures are logged. As a result, the
    /// returned [`Vec`] may contain fewer documents than the number of input spans.
    ///
    /// # Zero-Copy Translation
    ///
    /// This method takes a reference to a slice of [`SpanData`] and returns [`SegmentDocument`]s
    /// whose lifetime is bound to the input slice. The translation process is designed to be
    /// zero-copy wherever possible:
    ///
    /// - **String fields** (e.g., segment names) borrow directly from the span data
    /// - **String arrays** are referenced through trait objects (`&'span dyn StrList`)
    ///   without copying the underlying data
    ///
    /// Allocation happens mainly for XRay fields that require combining multiple OpenTelemetry fields,
    /// such as http.request.url (see [ADOT documentation])
    ///
    /// This design minimizes allocations during translation, making it efficient for high-throughput
    /// scenarios. However, it means the returned [`SegmentDocument`]s cannot outlive the input
    /// span data slice.
    ///
    /// **Note**: The returned documents are in arbitrary order and may not match the input
    /// span order. The translation process uses a [`HashMap`] to establish parent-child
    /// relationships, which does not preserve ordering.
    ///
    /// # Examples
    ///
    /// ```
    /// use opentelemetry_aws::xray_exporter::SegmentTranslator;
    /// use opentelemetry_sdk::trace::SpanData;
    ///
    /// let translator = SegmentTranslator::new();
    /// let spans: Vec<SpanData> = vec![]; // Your span data
    /// let documents = translator.translate_spans(&spans);
    /// ```
    ///
    /// [ADOT documentation]: https://aws-otel.github.io/docs/getting-started/x-ray#otel-span-http-attributes-translation
    #[cfg_attr(feature = "internal-logs", tracing::instrument(skip(self, batch)))]
    pub fn translate_spans<'span, 'translator: 'span>(
        &'translator self,
        batch: &'span [SpanData],
    ) -> Vec<SegmentDocument<'span>> {
        #[cfg(feature = "internal-logs")]
        tracing::debug!("Received {} spans", batch.len());

        #[cfg(feature = "subsegment-nesting")]
        if self.always_nest_subsegments {
            self._translate_spans_nested(batch)
        } else {
            self._translate_spans_simple(batch)
        }

        #[cfg(not(feature = "subsegment-nesting"))]
        self._translate_spans_simple(batch)
    }

    fn _translate_spans_simple<'span, 'translator: 'span>(
        &'translator self,
        batch: &'span [SpanData],
    ) -> Vec<SegmentDocument<'span>> {
        batch
            .iter()
            .filter_map(|span_data| {
                match self
                    .translate_span(span_data)
                    .and_then(|builder| builder.build())
                {
                    Ok(segment) => Some(segment),
                    Err(e) => {
                        #[cfg(feature = "internal-logs")]
                        tracing::error!(message="A segment or subsegment was lost", error=?e);
                        #[cfg(feature = "internal-logs")]
                        tracing::debug!(error=?e, ?span_data);
                        None
                    }
                }
            })
            .collect()
    }

    #[cfg(feature = "subsegment-nesting")]
    fn _translate_spans_nested<'span, 'translator: 'span>(
        &'translator self,
        batch: &'span [SpanData],
    ) -> Vec<SegmentDocument<'span>> {
        use crate::xray_exporter::types::{
            error::ConstraintError, DocumentBuilderHeader, Id, TraceId,
        };
        use std::collections::HashMap;

        // MARKER ALLOC
        let mut document_builders: HashMap<(TraceId, Id), AnyDocumentBuilder> =
            HashMap::with_capacity(batch.len());

        // MARKER ALLOC
        let mut document_builder_headers_tree = DocumentBuilderHeaderTree::new(batch.len());

        for span_data in batch.iter() {
            match
            self.translate_span(span_data).and_then(|builder| {
                let header = match &builder {
                    AnyDocumentBuilder::Segment(builder) => builder.header(),
                    AnyDocumentBuilder::Subsegment(builder) => builder.header(),
                };

                let id = header.id.ok_or(ConstraintError::MissingId)?;
                let trace_id = header.trace_id.ok_or(ConstraintError::MissingTraceId)?;
                if document_builders.insert((trace_id, id), builder).is_some() {
                    #[cfg(feature = "internal-logs")]
                    tracing::error!("Duplicated builder (id: {id}; trace-id: {trace_id:?}), a segment or subsegment was lost");
                } else {
                    document_builder_headers_tree
                        .add(header)
                        .expect("id and trace_id always present at this point");
                }
                Ok(())
            }) {
                Ok(_) => {},
                Err(e) => {
                    #[cfg(feature = "internal-logs")]
                    tracing::error!(message="A segment or subsegment was lost", error=?e);
                    #[cfg(feature = "internal-logs")]
                    tracing::debug!(error=?e, ?span_data);
                }
            }
        }

        let mut last_seen_parent_id = None;
        let mut last_seens_endtime = None;
        let mut precursors = Vec::new();
        // Process every subsegment with two goals:
        // 1. Add the precursor Ids to the subsegment, if any
        // 2. Include the subsegment builder into its parent builder, if it is in the batch
        for header in document_builder_headers_tree.iter() {
            let DocumentBuilderHeader {
                id,
                parent_id,
                trace_id,
                start_time,
                end_time,
            } = header;
            let id = id.expect("id always set at this point");
            let trace_id = trace_id.expect("trace_id always set at this point");
            let start_time = start_time.expect("start_time always set at this point");

            // We are precessing subsegments that have parent,
            // Skip builders that are not interesting
            let Some(parent_id) = parent_id else {
                continue;
            };
            let document_builders_index = &(trace_id, id);
            // the ID cannot be missing from the hashmap at this point (unless there are duplicates???)
            let AnyDocumentBuilder::Subsegment(subsegment_builder) = document_builders
                .get_mut(document_builders_index)
                .expect("builders in subsegments are always also in document_builders")
            else {
                continue;
            };

            if last_seen_parent_id != Some(parent_id) {
                last_seen_parent_id = Some(parent_id);
                // We changed parent, clear precursors
                precursors.clear();
            }

            // I don't know if this is really necessary or usefull,
            // the goal is to ensure that the precedent segment (if any)
            // indeed ended before this subsegment started.
            // Else we ignore the segment for the purpose of precursors?
            if last_seens_endtime.is_none()
                || last_seens_endtime
                    .is_some_and(|last_seens_endtime| last_seens_endtime <= start_time)
            {
                // If there are precursors to set, do it
                if !precursors.is_empty() {
                    // MARKER ALLOC
                    subsegment_builder.precursor_ids(precursors.clone());
                }
                // If there is an end_time (so, it is not in_progress)
                // - Update the last_seens_endtime
                // - Add the id of the present segment to the precursor list
                if let Some(end_time) = end_time {
                    last_seens_endtime = Some(end_time);
                    // MARKER ALLOC
                    precursors.push(id);
                }
            }

            let document_builders_parent_index = &(trace_id, parent_id);
            // Now, verify if we got the parent_id, if so add this builder as a subsegment of the parent
            if document_builders.contains_key(document_builders_parent_index) {
                if let AnyDocumentBuilder::Subsegment(subsegment) = document_builders
                    .remove(document_builders_index)
                    .expect("builders in subsegments are always also in document_builders")
                {
                    match document_builders
                        .get_mut(document_builders_parent_index)
                        .expect("verified parent_id is present")
                    {
                        AnyDocumentBuilder::Segment(parent) => {
                            // MARKER ALLOC
                            parent.subsegment(subsegment);
                        }
                        AnyDocumentBuilder::Subsegment(parent) => {
                            // MARKER ALLOC
                            parent.subsegment(subsegment);
                        }
                    }
                };
            }
        }

        // MARKER ALLOC
        // At this point we can FINALLY return a Vec<SegmentDocument>
        document_builders
            .into_values()
            .filter_map(|document_builder| match document_builder.build() {
                Ok(segment) => Some(segment),
                Err(e) => {
                    #[cfg(feature = "internal-logs")]
                    tracing::error!(message="A segment or subsegment was lost", error=?e);
                    None
                }
            })
            .collect()
    }

    /// Translates a single OpenTelemetry span into a segment or subsegment builder.
    ///
    /// Converts span to the appropriate builder based on span kind (Server → Segment,
    /// others → Subsegment) and processes attributes according to semantic conventions.
    ///
    /// # Errors
    ///
    /// Returns a [`TranslationError`] if:
    /// - The span has an invalid (zero) span ID
    /// - Segment document constraints are violated
    /// - Timestamp validation fails (unless disabled)
    ///
    /// [`TranslationError`]: error::TranslationError
    #[cfg_attr(feature = "internal-logs", tracing::instrument(skip(self, span_data)))]
    fn translate_span<'span, 'translator: 'span>(
        &'translator self,
        span_data: &'span SpanData,
    ) -> Result<AnyDocumentBuilder<'span>> {
        #[cfg(feature = "internal-logs")]
        tracing::trace!(?span_data);

        let SpanData {
            span_kind,
            name,
            attributes,
            events,
            status,
            ..
        } = span_data;

        let span_kind_is_server = matches!(span_kind, SpanKind::Server);
        let span_is_remote = matches!(span_kind, SpanKind::Client | SpanKind::Producer);

        // Additional builders that must keeps track of multiple attribute values
        // in order to compute the value of a specific XRay (sub)Segment field.
        //
        // Note: Technically, we don't need all of them depending if we are building
        // a segment or a subsegment, but it is just cheaper to put them all
        // in a stack-array than to allocate a Vec for only those we need
        let mut additionnal_builders = AnyValueBuilder::array(
            AwsOperationBuilder::default(),
            AwsXraySdkBuilder::default(),
            BeanstalkDeploymentIdBuilder::default(),
            CauseBuilder::new(events, status, span_is_remote),
            CloudwatchLogGroupBuilder::new(&self.log_group_names),
            HttpRequestUrlBuilder::new(span_kind_is_server),
            HttpResponseContentLengthBuilder::default(),
            SegmentNameBuilder::new(name.as_ref(), span_kind_is_server),
            SegmentOriginBuilder::default(),
            SqlUrlBuilder::default(),
            SubsegmentNamespaceBuilder::new(matches!(span_kind, SpanKind::Client)),
        );

        // Construct the DocumentBuilder we need (Segment or Subsegment)
        let mut any_segment_builder = {
            match span_data.span_kind {
                SpanKind::Server => {
                    AnyDocumentBuilder::Segment(self.init_document_builder(span_data)?)
                }
                _ => AnyDocumentBuilder::Subsegment(self.init_document_builder(span_data)?),
            }
        };

        // Create an iterator over all the attributes:
        //  - those of the Resource
        //  - those of the span
        //
        // TODO: This means we are processing the Resource attributes again and again,
        // but they do not change. We could find a way to preprocess and just copy them
        // The lifetime should work out as the preprocessed resources would have 'translator and
        // 'translator outlives 'span.
        let attribute_iterator = self
            .resource
            .iter()
            .flat_map(|r: &Resource| r.iter())
            .chain(attributes.iter().map(|kv: &KeyValue| (&kv.key, &kv.value)));
        // Process all the attributes
        for (key, value) in attribute_iterator {
            let key = key.as_str();
            let (should_index, should_metadata, key) =
                if let Some(key) = key.strip_prefix("annotation.") {
                    (true, false, key)
                } else if let Some(key) = key.strip_prefix("metadata.") {
                    (false, true, key)
                } else {
                    (false, false, key)
                };

            // Track the attribute value inclusion
            let mut attribute_included = false;

            // Ask the dispatch table for a list of builders that are interested in the attribute
            for &(builder_id, handler_index) in self.dispatch_table.dispatch(key) {
                let handler_result = match builder_id {
                    ANY_DOCUMENT_BUILDER_PROCESSOR_ID => AnyDocumentBuilder::HANDLERS
                        [handler_index]
                        .1(
                        &mut any_segment_builder, value
                    ),
                    builder_id => {
                        // We know the builder_id will correspond to the correct index because:
                        // 1. additionnal_builders is constructed through AnyValueBuilder::array(...)
                        // 2. builders are registered on the dispatch_table through AnyValueBuilder::register_builders()
                        // The macro constructing AnyValueBuilder ensures everything stays coherent.
                        additionnal_builders[builder_id].process(handler_index, value)
                    }
                };
                // attribute_included will be `true` if any handler returns `true`
                attribute_included = attribute_included || handler_result;
            }

            let should_index = should_index
                || (!attribute_included && self.index_all_attrs)
                || self
                    .indexed_attrs
                    .binary_search_by(|s| s.as_str().cmp(key))
                    .is_ok();

            // If the attribute was not included and we index all attributes
            // Or
            // If the attribute key is in the explicit list of attribute to index
            // Then we try to include it in the annotations
            if should_index {
                // If the value is compatible with metadata
                if let Some(annotation) = get_annotation(value) {
                    let sanitized_key = sanitize_annotation_key(key);
                    // Reset attribute_included to true or false depending on the success of metadata inclusion:
                    // Rational:
                    // We wanted to index that key for some reason, and the value was not compatible with that,
                    // or there are already too much annotations, so next best thing is to add it to metadata...
                    attribute_included = match &mut any_segment_builder {
                        AnyDocumentBuilder::Segment(builder) => {
                            builder.annotation(sanitized_key, annotation).is_ok()
                        }
                        AnyDocumentBuilder::Subsegment(builder) => {
                            builder.annotation(sanitized_key, annotation).is_ok()
                        }
                    };
                } else {
                    // Reset attribute_included to false in this case:
                    // Rational:
                    // We wanted to index it for some reason, and the value was not compatible with that,
                    // so next best thing is to add it to metadata...
                    attribute_included = false;
                }
            }

            let should_metadata = should_metadata && !should_index
                || should_index && !attribute_included
                || (!attribute_included && self.metadata_all_attrs)
                || self
                    .metadata_attrs
                    .binary_search_by(|s| s.as_str().cmp(key))
                    .is_ok();

            // If still not included, add to metadata
            if should_metadata {
                if let Some(value) = get_any_value(value) {
                    match &mut any_segment_builder {
                        AnyDocumentBuilder::Segment(builder) => {
                            builder.metadata(key, value);
                        }
                        AnyDocumentBuilder::Subsegment(builder) => {
                            builder.metadata(key, value);
                        }
                    }
                }
            }
        }

        // Consume and resolve all the additional builders
        for additionnal_builder in additionnal_builders {
            // MARKER MAYBE ALLOC (if the resolve method needs to format a String)
            additionnal_builder.resolve(&mut any_segment_builder)?;
        }

        Ok(any_segment_builder)
    }

    /// Initializes a document builder with common fields from span data.
    ///
    /// Pre-allocates capacity for annotations and metadata based on attribute count.
    fn init_document_builder<'span, DBT: DocumentBuilderType>(
        &self,
        span_data: &'span SpanData,
    ) -> Result<DocumentBuilder<'span, DBT>> {
        let SpanData {
            span_context,
            parent_span_id,
            start_time,
            end_time,
            attributes,
            ..
        } = span_data;

        let start_time = *start_time;
        let end_time = *end_time;
        let parent_span_id = *parent_span_id;
        let attribute_count = attributes.len();

        let mut builder = DocumentBuilder::default();
        let trace_id = span_context.trace_id();
        if trace_id != opentelemetry::TraceId::INVALID {
            builder.trace_id(trace_id.into(), self.skip_timestamp_validation)?;
        }

        let span_id = span_context.span_id();
        if span_id == SpanId::INVALID {
            return Err(TranslationError::MissingSpanId);
        }
        builder
            .id(span_id.into())
            .start_time(translate_timestamp(start_time))
            .end_time(translate_timestamp(end_time))?;

        if parent_span_id != SpanId::INVALID {
            builder.parent_id(parent_span_id.into());
        }

        let (max_annotations, max_metadata) = if self.index_all_attrs {
            (attribute_count, 0)
        } else {
            let max_annotations = attribute_count.min(self.indexed_attrs.len());
            (max_annotations, attribute_count - max_annotations)
        };

        Ok(builder
            .with_annotation_capacity(max_annotations)
            .with_metadata_capacity(max_metadata))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use opentelemetry::{
        trace::{SpanContext, SpanId, TraceFlags, TraceId, TraceState},
        InstrumentationScope, KeyValue,
    };
    use opentelemetry_sdk::trace::{SpanData, SpanEvents, SpanLinks};

    /// Creates a valid X-Ray trace ID with the current timestamp.
    fn create_valid_trace_id() -> TraceId {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as u128;
        // trace_id layout: 32-bit timestamp in the upper bits, 96-bit random in the lower bits
        let random_part: u128 = 0xabcdef0123456789abcdef01;
        let trace_id = (timestamp << 96) | random_part;
        TraceId::from_bytes(trace_id.to_be_bytes())
    }

    /// Creates a minimal SpanData for testing.
    fn create_span(
        trace_id: TraceId,
        span_id: SpanId,
        kind: SpanKind,
        attributes: Vec<KeyValue>,
    ) -> SpanData {
        let span_context = SpanContext::new(
            trace_id,
            span_id,
            TraceFlags::SAMPLED,
            false,
            TraceState::default(),
        );
        let start_time = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let end_time = start_time + Duration::from_millis(100);

        SpanData {
            span_context,
            parent_span_id: SpanId::INVALID,
            parent_span_is_remote: false,
            span_kind: kind,
            name: "test-span".into(),
            start_time,
            end_time,
            attributes,
            dropped_attributes_count: 0,
            events: SpanEvents::default(),
            links: SpanLinks::default(),
            status: opentelemetry::trace::Status::Unset,
            instrumentation_scope: InstrumentationScope::builder("test").build(),
        }
    }

    // =========================================================================
    // Tests for init_document_builder — invalid span IDs
    // =========================================================================

    #[test]
    fn test_init_document_builder_invalid_trace_id() {
        // A span with TraceId::INVALID (all zeros) — the translator skips setting
        // trace_id on the builder. For a Server span this means the resulting
        // Segment builder will lack a trace_id, causing a ConstraintError::MissingTraceId
        // when build() is called. translate_spans silently drops such spans.
        let translator = SegmentTranslator::new().skip_timestamp_validation();
        let span = create_span(
            TraceId::INVALID,
            SpanId::from_bytes([0, 0, 0, 0, 0, 0, 0, 1]),
            SpanKind::Server,
            vec![],
        );

        let batch = [span];
        let documents = translator.translate_spans(&batch);
        // The span is dropped because the segment builder has no trace_id
        assert!(
            documents.is_empty(),
            "Span with INVALID trace_id should be silently dropped"
        );
    }

    #[test]
    fn test_init_document_builder_invalid_span_id() {
        // A span with SpanId::INVALID (all zeros) → TranslationError::MissingSpanId
        let translator = SegmentTranslator::new().skip_timestamp_validation();
        let span = create_span(
            create_valid_trace_id(),
            SpanId::INVALID,
            SpanKind::Server,
            vec![],
        );

        let batch = [span];
        let documents = translator.translate_spans(&batch);
        assert!(
            documents.is_empty(),
            "Span with INVALID span_id should be silently dropped (MissingSpanId)"
        );
    }

    // =========================================================================
    // Tests for attribute routing — annotation./metadata. prefix stripping
    // =========================================================================

    #[test]
    fn test_annotation_prefix_stripping() {
        // An attribute with "annotation." prefix should have the prefix stripped
        // and be added as an annotation.
        let translator = SegmentTranslator::new().skip_timestamp_validation();
        let span = create_span(
            create_valid_trace_id(),
            SpanId::from_bytes([0, 0, 0, 0, 0, 0, 0, 1]),
            SpanKind::Server,
            vec![KeyValue::new("annotation.my_key", "my_value")],
        );
        let batch = [span];
        let documents = translator.translate_spans(&batch);
        assert_eq!(documents.len(), 1);

        let json: serde_json::Value = serde_json::to_value(&documents[0]).unwrap();

        // The stripped key "my_key" should appear in annotations
        let annotations = json.get("annotations").expect("annotations should exist");
        assert_eq!(
            annotations.get("my_key"),
            Some(&serde_json::json!("my_value")),
            "annotation.my_key should be stripped to my_key in annotations"
        );

        // It should NOT appear in metadata
        assert!(
            json.get("metadata").is_none() || json["metadata"].get("my_key").is_none(),
            "annotation-prefixed key should not appear in metadata"
        );
    }

    #[test]
    fn test_metadata_prefix_stripping() {
        // An attribute with "metadata." prefix should have the prefix stripped
        // and be added as metadata.
        let translator = SegmentTranslator::new().skip_timestamp_validation();
        let span = create_span(
            create_valid_trace_id(),
            SpanId::from_bytes([0, 0, 0, 0, 0, 0, 0, 2]),
            SpanKind::Server,
            vec![KeyValue::new("metadata.my_key", "my_value")],
        );

        let batch = [span];
        let documents = translator.translate_spans(&batch);
        assert_eq!(documents.len(), 1);

        let json: serde_json::Value = serde_json::to_value(&documents[0]).unwrap();

        // The stripped key "my_key" should appear in metadata
        let metadata = json.get("metadata").expect("metadata should exist");
        assert_eq!(
            metadata.get("my_key"),
            Some(&serde_json::json!("my_value")),
            "metadata.my_key should be stripped to my_key in metadata"
        );

        // It should NOT appear in annotations
        assert!(
            json.get("annotations").is_none() || json["annotations"].get("my_key").is_none(),
            "metadata-prefixed key should not appear in annotations"
        );
    }

    // =========================================================================
    // Tests for with_indexed_attr deduplication
    // =========================================================================

    #[test]
    fn test_with_indexed_attr_deduplication() {
        // Calling with_indexed_attr twice with the same key should not create
        // duplicates. We verify indirectly: the attribute should appear as an
        // annotation (proving it's in the indexed list) and only once.
        let translator = SegmentTranslator::new()
            .skip_timestamp_validation()
            .with_indexed_attr("same_key".to_string())
            .with_indexed_attr("same_key".to_string());

        let span = create_span(
            create_valid_trace_id(),
            SpanId::from_bytes([0, 0, 0, 0, 0, 0, 0, 3]),
            SpanKind::Server,
            vec![KeyValue::new("same_key", "the_value")],
        );

        let batch = [span];
        let documents = translator.translate_spans(&batch);
        assert_eq!(documents.len(), 1);

        let json: serde_json::Value = serde_json::to_value(&documents[0]).unwrap();

        // The key should appear as an annotation (indexed)
        let annotations = json.get("annotations").expect("annotations should exist");
        assert_eq!(
            annotations.get("same_key"),
            Some(&serde_json::json!("the_value")),
            "same_key should be indexed as annotation despite duplicate with_indexed_attr calls"
        );
    }

    // =========================================================================
    // Tests for with_metadata_attr deduplication
    // =========================================================================

    #[test]
    fn test_with_metadata_attr_deduplication() {
        // Calling with_metadata_attr twice with the same key should not create
        // duplicates. We verify indirectly: the attribute should appear in
        // metadata (proving it's in the metadata list) and only once.
        let translator = SegmentTranslator::new()
            .skip_timestamp_validation()
            .with_metadata_attr("same_key".to_string())
            .with_metadata_attr("same_key".to_string());

        let span = create_span(
            create_valid_trace_id(),
            SpanId::from_bytes([0, 0, 0, 0, 0, 0, 0, 4]),
            SpanKind::Server,
            vec![KeyValue::new("same_key", "the_value")],
        );

        let batch = [span];
        let documents = translator.translate_spans(&batch);
        assert_eq!(documents.len(), 1);

        let json: serde_json::Value = serde_json::to_value(&documents[0]).unwrap();

        // The key should appear in metadata
        let metadata = json.get("metadata").expect("metadata should exist");
        assert_eq!(
            metadata.get("same_key"),
            Some(&serde_json::json!("the_value")),
            "same_key should be in metadata despite duplicate with_metadata_attr calls"
        );

        // Verify it appears exactly once (not duplicated in the JSON object)
        let metadata_obj = metadata.as_object().expect("metadata should be an object");
        let same_key_count = metadata_obj.keys().filter(|k| *k == "same_key").count();
        assert_eq!(
            same_key_count, 1,
            "same_key should appear exactly once in metadata"
        );
    }

    // =========================================================================
    // Tests for indexed attributes overriding metadata prefix
    // =========================================================================

    #[test]
    fn test_metadata_prefix_overridden_by_indexed_attr() {
        // When an attribute has the "metadata." prefix but its stripped key is
        // explicitly registered as an indexed attribute, the indexed-attr lookup
        // wins: should_index becomes true, which flips should_metadata to false
        // (line 876: should_metadata = should_metadata && !should_index).
        // The attribute should therefore appear in annotations, not metadata.
        let translator = SegmentTranslator::new()
            .skip_timestamp_validation()
            .with_indexed_attr("my_indexed_attr".to_string());

        let span = create_span(
            create_valid_trace_id(),
            SpanId::from_bytes([0, 0, 0, 0, 0, 0, 0, 5]),
            SpanKind::Server,
            vec![KeyValue::new("metadata.my_indexed_attr", "hello")],
        );

        let batch = [span];
        let documents = translator.translate_spans(&batch);
        assert_eq!(documents.len(), 1);

        let json: serde_json::Value = serde_json::to_value(&documents[0]).unwrap();

        // The stripped key should appear in annotations (indexed wins)
        let annotations = json.get("annotations").expect("annotations should exist");
        assert_eq!(
            annotations.get("my_indexed_attr"),
            Some(&serde_json::json!("hello")),
            "metadata.my_indexed_attr should be routed to annotations when the key is indexed"
        );

        // It should NOT appear in metadata
        assert!(
            json.get("metadata").is_none() || json["metadata"].get("my_indexed_attr").is_none(),
            "indexed attr should not appear in metadata even with metadata. prefix"
        );
    }
}
