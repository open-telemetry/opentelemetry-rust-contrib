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

use std::borrow::Cow;

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
/// Attributes are processed as follow:
///
/// 1. **Recognized attributes** are mapped to specific X-Ray fields (e.g., `http.method` → `http.request.method`)
/// 2. **Indexed attributes** (if configured) are added as annotations (searchable, max 50)
/// 3. **Remaining attributes** are added as metadata (not searchable, unlimited)
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
    /// # Errors
    ///
    /// Returns a [`TranslationError`] if:
    /// - A span is missing required fields (e.g., TraceId for a Server Span)
    /// - Segment document constraints are violated during building
    /// - Timestamp validation fails (unless disabled)
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use opentelemetry_aws::xray_exporter::SegmentTranslator;
    /// use opentelemetry_sdk::trace::SpanData;
    ///
    /// let translator = SegmentTranslator::new();
    /// let spans: Vec<SpanData> = vec![]; // Your span data
    /// let documents = translator.translate_spans(&spans)?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// [`TranslationError`]: error::TranslationError
    /// [ADOT documentation]: https://aws-otel.github.io/docs/getting-started/x-ray#otel-span-http-attributes-translation
    #[cfg_attr(feature = "internal-logs", tracing::instrument(skip(batch)))]
    pub fn translate_spans<'span, 'translator: 'span>(
        &'translator self,
        batch: &'span [SpanData],
    ) -> Result<Vec<SegmentDocument<'span>>> {
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
    ) -> Result<Vec<SegmentDocument<'span>>> {
        batch
            .iter()
            .map(|span_data| self.translate_span(span_data)?.build())
            .collect()
    }

    #[cfg(feature = "subsegment-nesting")]
    fn _translate_spans_nested<'span, 'translator: 'span>(
        &'translator self,
        batch: &'span [SpanData],
    ) -> Result<Vec<SegmentDocument<'span>>> {
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
            let builder = self.translate_span(span_data)?;
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
                document_builder_headers_tree.add(header)?;
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
            .map(|document_builder| document_builder.build())
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
        tracing::debug!("{:?}", span_data);

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

        // The Span may have a aws.xray.annotations attribute with a &[&str] slice
        // that contains additional attributes to add as annotations
        // If that's the case, we extend the indexed_attributes with whatever is in this slice
        // It means we have to pre-process every attributes before the real processing...
        let aws_xray_annotations = attributes
            .iter()
            .find(|kv| kv.key.as_str() == "aws.xray.annotations");
        let indexed_attrs: Cow<'_, [String]> = {
            use opentelemetry::{Array, Value};
            if let Some(KeyValue {
                value: Value::Array(Array::String(lst)),
                ..
            }) = aws_xray_annotations
            {
                let mut indexed_attrs = self.indexed_attrs.clone();
                for new_attr in lst {
                    let new_attr = new_attr.to_string();
                    match indexed_attrs.binary_search(&new_attr) {
                        Ok(_) => (),
                        Err(i) => indexed_attrs.insert(i, new_attr),
                    }
                }
                Cow::Owned(indexed_attrs)
            } else {
                Cow::Borrowed(self.indexed_attrs.as_slice())
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

            // If the attribute was not included and we index all attributes
            // Or
            // If the attribute key is in the explicit list of attribute to index
            // Then we try to include it in the annotations
            if (!attribute_included && self.index_all_attrs)
                || indexed_attrs
                    .binary_search_by(|s| s.as_str().cmp(key))
                    .is_ok()
            {
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

            // If still not included, add to metadata
            if !attribute_included {
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
