use serde::Serialize;
use std::borrow::Cow;

use crate::field_setter;

use super::{
    aws::{AwsData, AwsDataBuilder},
    cause::{ErrorDetails, ErrorDetailsBuilder},
    error::{ConstraintError, Result},
    http::{HttpData, HttpDataBuilder},
    id::{Id, TraceId},
    service::{ServiceData, ServiceDataBuilder},
    sql::{SqlData, SqlDataBuilder},
    utils::{annotation_key_regex, name_regex, verify_string_length, MaybeSkip},
    value::{AnnotationValue, AnyValue, Namespace, Origin, StrList, VectorMap},
};

/// An opaque structure that represents an AWS X-Ray segment document.
///
/// This structure encapsulates all the data needed to represent a complete X-Ray trace segment
/// or subsegment, including timing information, service metadata, HTTP details, AWS-specific
/// data, annotations, and nested subsegments.
///
/// `SegmentDocument` instances are created by the [`SegmentTranslator`] when converting
/// OpenTelemetry spans to X-Ray format. Users do not construct these directly, but instead
/// interact with them through the [`SegmentDocumentExporter`] trait when implementing
/// custom exporters.
///
/// The document can be serialized to JSON format for transmission to the X-Ray service
/// using the provided serialization methods.
///
/// # Examples
///
/// Using segment documents in a custom exporter:
///
/// ```no_run
/// use opentelemetry_aws::xray_exporter::{SegmentDocument, SegmentDocumentExporter};
/// use std::future::Future;
///
/// struct MyExporter;
///
/// impl SegmentDocumentExporter for MyExporter {
///     type Error = std::io::Error;
///     async fn export_segment_documents(&self, batch: Vec<SegmentDocument<'_>>) -> Result<(), Self::Error> {
///         for document in batch {
///             // Serialize to compact JSON
///             let json = document.to_string();
///             println!("Exporting: {}", json);
///
///             // Or serialize to bytes for network transmission
///             let bytes = document.to_bytes();
///             // send_to_xray(bytes).await?;
///         }
///     Ok(())
///     }
/// }
/// ```
///
/// Serializing segment documents:
///
/// ```no_run
/// # use opentelemetry_aws::xray_exporter::{SegmentDocument, SegmentTranslator};
/// # use opentelemetry_sdk::trace::SpanData;
/// # fn example(translator: &SegmentTranslator, spans: &[SpanData]) -> Result<(), Box<dyn std::error::Error>> {
/// let documents = translator.translate_spans(spans);
///
/// for document in &documents {
///     // Compact JSON string
///     let json = document.to_string();
///
///     // Pretty-formatted JSON for debugging
///     let pretty = document.to_string_pretty();
///
///     // Byte vector for efficient transmission
///     let bytes = document.to_bytes();
///
///     // Write directly to a buffer
///     let mut buffer = Vec::new();
///     document.to_writer(&mut buffer);
/// }
/// # Ok(())
/// # }
/// ```
///
/// [`SegmentTranslator`]: crate::xray_exporter::SegmentTranslator
/// [`SegmentDocumentExporter`]: crate::xray_exporter::SegmentDocumentExporter
#[derive(Debug, Serialize)]
pub struct SegmentDocument<'a> {
    /// The type of subsegment (value set to "subsegment" for independent subsegments)
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    subsegment_type: Option<&'static str>,

    /// The logical name of the service that handled the request (1-200 characters)
    name: &'a str,

    /// A 64-bit identifier for the segment, unique among segments in the same trace
    id: Id,

    /// A unique identifier that connects all segments and subsegments from a single request
    #[serde(skip_serializing_if = "Option::is_none")]
    trace_id: Option<TraceId>,

    /// The start time of the segment in floating point seconds since epoch
    start_time: f64,

    /// The end time of the segment in floating point seconds since epoch
    #[serde(skip_serializing_if = "Option::is_none")]
    end_time: Option<f64>,

    /// Service version information
    #[serde(skip_serializing_if = "MaybeSkip::skip")]
    service: ServiceData<'a>,

    /// A string that identifies the user who sent the request (max 250 characters)
    #[serde(skip_serializing_if = "MaybeSkip::skip")]
    user: Option<Cow<'a, str>>,

    /// The AWS origin of the segment (e.g., AWS::EC2::Instance)
    #[serde(skip_serializing_if = "Option::is_none")]
    origin: Option<Origin>,

    /// The ID of the parent segment or subsegment
    #[serde(skip_serializing_if = "Option::is_none")]
    parent_id: Option<Id>,

    /// The namespace for subsegments (`aws` or `remote`).
    #[serde(skip_serializing_if = "Option::is_none")]
    namespace: Option<Namespace>,

    /// HTTP request and response information
    #[serde(skip_serializing_if = "MaybeSkip::skip")]
    http: HttpData<'a>,

    /// AWS-specific information about the resource running the application
    #[serde(skip_serializing_if = "MaybeSkip::skip")]
    aws: AwsData<'a>,

    /// CloudWatch Logs configuration associated with this segment.
    #[serde(skip_serializing_if = "MaybeSkip::skip")]
    cloudwatch_logs: CloudwatchLogs<'a>,

    /// Nested type providing details about errors and exception for this segment.
    /// It is flatten and contains the `fault`, `error`, `throttle` and `cause` fields.
    #[serde(flatten)]
    error_details: ErrorDetails<'a>,

    /// SQL database query information
    #[serde(skip_serializing_if = "MaybeSkip::skip")]
    sql: SqlData<'a>,

    /// Indexed string key-value pairs for search
    #[serde(skip_serializing_if = "VectorMap::is_empty")]
    annotations: VectorMap<Cow<'a, str>, AnnotationValue<'a>>,

    /// Non-indexed metadata organized by namespace
    #[serde(skip_serializing_if = "VectorMap::is_empty")]
    metadata: VectorMap<&'a str, AnyValue<'a>>,

    #[cfg(feature = "subsegment-nesting")]
    /// Array of subsegment objects
    #[serde(skip_serializing_if = "Vec::is_empty")]
    subsegments: Vec<SegmentDocument<'a>>,

    #[cfg(feature = "subsegment-nesting")]
    /// Array of subsegment IDs that identifies subsegments with the same parent that completed prior to this subsegment
    #[serde(skip_serializing_if = "Vec::is_empty")]
    precursor_ids: Vec<Id>,
}

impl SegmentDocument<'_> {
    /// Serializes the segment document to a compact JSON string.
    ///
    /// This is a wrapper helper around [`serde_json::to_string`] that converts the segment
    /// document into the JSON format expected by the AWS X-Ray service.
    #[allow(clippy::inherent_to_string)]
    #[inline(always)]
    pub fn to_string(&self) -> String {
        serde_json::to_string(self).unwrap()
    }

    /// Serializes the segment document to a pretty-formatted JSON string.
    ///
    /// This is a wrapper helper around [`serde_json::to_string_pretty`] that produces
    /// human-readable JSON with proper indentation, useful for debugging.
    #[inline(always)]
    pub fn to_string_pretty(&self) -> String {
        serde_json::to_string_pretty(self).unwrap()
    }

    /// Serializes the segment document to a byte vector.
    ///
    /// This is a wrapper helper around [`serde_json::to_vec`] that converts to compact
    /// JSON byte vector for efficient transmission or storage.
    #[inline(always)]
    pub fn to_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap()
    }

    /// Serializes the segment document directly into a buffer.
    ///
    /// This is a wrapper helper around [`serde_json::to_writer`] that is more efficient
    /// than creating a separate byte vector when you have a buffer available.
    #[inline(always)]
    pub fn to_writer<W: std::io::Write>(&self, buf: W) {
        serde_json::to_writer(buf, self).unwrap()
    }
}

pub(super) mod builder_type {
    use super::{ConstraintError, DocumentBuilder, Result};

    mod _private {
        pub trait Sealed {}
        impl Sealed for super::Segment {}
        impl Sealed for super::Subsegment {}
    }
    pub(crate) trait DocumentBuilderType:
        _private::Sealed + core::fmt::Debug + Default
    {
        fn segment_type() -> Option<&'static str>;
        fn verify_constraints(sbd: &DocumentBuilder<Self>) -> Result<()>;
    }
    #[derive(Debug, Default)]
    pub(crate) struct Segment;
    #[derive(Debug, Default)]
    pub(crate) struct Subsegment;
    impl DocumentBuilderType for Segment {
        fn segment_type() -> Option<&'static str> {
            None
        }

        fn verify_constraints(sbd: &DocumentBuilder<Self>) -> Result<()> {
            if sbd.trace_id.is_none() {
                return Err(ConstraintError::MissingTraceId);
            }

            Ok(())
        }
    }
    impl DocumentBuilderType for Subsegment {
        fn segment_type() -> Option<&'static str> {
            Some("subsegment")
        }

        fn verify_constraints(sbd: &DocumentBuilder<Self>) -> Result<()> {
            if !sbd.nested {
                if sbd.parent_id.is_none() {
                    return Err(ConstraintError::MissingParentId);
                }
                if sbd.trace_id.is_none() {
                    return Err(ConstraintError::MissingTraceId);
                }
            }

            Ok(())
        }
    }
}
use builder_type::DocumentBuilderType;

/// Builder for constructing segment or subsegment documents.
#[derive(Debug, Default)]
pub(crate) struct DocumentBuilder<'a, DBT: DocumentBuilderType> {
    nested: bool,
    name: Option<&'a str>,
    id: Option<Id>,
    trace_id: Option<TraceId>,
    start_time: Option<f64>,
    end_time: Option<f64>,
    service: ServiceDataBuilder<'a>,
    user: Option<Cow<'a, str>>,
    origin: Option<Origin>,
    namespace: Option<Namespace>,
    parent_id: Option<Id>,
    http: HttpDataBuilder<'a, DBT>,
    aws: AwsDataBuilder<'a, DBT>,
    cloudwatch_logs: CloudwatchLogsBuilder<'a>,
    sql: SqlDataBuilder<'a>,
    error_details: ErrorDetailsBuilder<'a>,
    annotations: VectorMap<Cow<'a, str>, AnnotationValue<'a>>,
    metadata: VectorMap<&'a str, AnyValue<'a>>,
    #[cfg(feature = "subsegment-nesting")]
    subsegments: Vec<SubsegmentDocumentBuilder<'a>>,
    #[cfg(feature = "subsegment-nesting")]
    precursor_ids: Vec<Id>,
}

/// Builder for top-level X-Ray segment documents.
pub(crate) type SegmentDocumentBuilder<'a> = DocumentBuilder<'a, builder_type::Segment>;

/// Builder for X-Ray subsegment documents.
pub(crate) type SubsegmentDocumentBuilder<'a> = DocumentBuilder<'a, builder_type::Subsegment>;

#[cfg(feature = "subsegment-nesting")]
/// Core identifying and timing information for segment/subsegment builders.
#[derive(Debug, Clone, Copy)]
pub(crate) struct DocumentBuilderHeader {
    /// The unique identifier for this segment or subsegment.
    pub id: Option<Id>,
    /// The ID of the parent segment or subsegment.
    pub parent_id: Option<Id>,
    /// The trace ID connecting all segments in this request.
    pub trace_id: Option<TraceId>,
    /// The start time in seconds since Unix epoch.
    pub start_time: Option<f64>,
    /// The end time in seconds since Unix epoch.
    pub end_time: Option<f64>,
}

impl<'a, DBT: DocumentBuilderType> DocumentBuilder<'a, DBT> {
    /// Maximum age for trace timestamps (30 days in seconds).
    const MAX_TRACE_ID_AGE: u64 = 60 * 60 * 24 * 30;
    /// Maximum clock skew allowed (5 minutes in seconds).
    const MAX_TRACE_ID_SKEW: u64 = 60 * 5;

    pub fn with_annotation_capacity(mut self, capacity: usize) -> Self {
        self.annotations.reserve(capacity);
        self
    }

    pub fn with_metadata_capacity(mut self, capacity: usize) -> Self {
        self.metadata.reserve(capacity);
        self
    }

    /////////////
    // GETTERS //
    /////////////

    pub fn http(&mut self) -> &mut HttpDataBuilder<'a, DBT> {
        &mut self.http
    }

    pub fn aws(&mut self) -> &mut AwsDataBuilder<'a, DBT> {
        &mut self.aws
    }

    pub fn cloudwatch_logs(&mut self) -> &mut CloudwatchLogsBuilder<'a> {
        &mut self.cloudwatch_logs
    }

    pub fn error_details(&mut self) -> &mut ErrorDetailsBuilder<'a> {
        &mut self.error_details
    }

    #[cfg(feature = "subsegment-nesting")]
    pub fn header(&self) -> DocumentBuilderHeader {
        DocumentBuilderHeader {
            id: self.id,
            parent_id: self.parent_id,
            trace_id: self.trace_id,
            start_time: self.start_time,
            end_time: self.end_time,
        }
    }

    /////////////
    // SETTERS //
    /////////////

    field_setter!(id:Id);
    field_setter!(start_time:f64);
    field_setter!(parent_id:Id);

    /// Sets the trace ID.
    ///
    /// # Errors
    ///
    /// Returns `ConstraintError::InvalidTraceId` if the TraceId timestamp is older than 30 days or more than 5 minutes in the future.
    pub fn trace_id(
        &mut self,
        trace_id: TraceId,
        skip_timestamp_validation: bool,
    ) -> Result<&mut Self> {
        let trace_id = if skip_timestamp_validation {
            Ok(trace_id)
        } else {
            use std::time::{SystemTime, UNIX_EPOCH};
            let trace_id_timestamp = trace_id.timestamp() as u64;
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("EPOCH is earlier")
                .as_secs();
            if trace_id_timestamp < now + Self::MAX_TRACE_ID_SKEW
                && trace_id_timestamp > now - Self::MAX_TRACE_ID_AGE
            {
                Ok(trace_id)
            } else {
                Err(ConstraintError::InvalidTraceId)
            }
        }?;
        self.trace_id = Some(trace_id);
        Ok(self)
    }

    /// Sets the end time.
    ///
    /// # Errors
    ///
    /// Returns `ConstraintError::EndTimeBeforeStartTime` if end_time is before start_time.
    /// Returns `ConstraintError::EndTimeAndInProgressConflict` if in_progress is already true.
    pub fn end_time(&mut self, end_time: f64) -> Result<&mut Self> {
        if let Some(start_time) = self.start_time {
            if end_time < start_time {
                return Err(ConstraintError::EndTimeBeforeStartTime);
            }
        }
        self.end_time = Some(end_time);
        Ok(self)
    }

    /// Sets an annotation.
    ///
    /// # Errors
    ///
    /// Returns `ConstraintError::InvalidAnnotationKey` if the key doesn't match the pattern (1-500 alphanumeric characters or underscores).
    /// Returns `ConstraintError::InvalidAnnotationValue` if a string value is longer than 1000 characters.
    pub fn annotation(
        &mut self,
        key: Cow<'a, str>,
        annotation: AnnotationValue<'a>,
    ) -> Result<&mut Self> {
        if self.annotations.len() >= 50 {
            return Err(ConstraintError::TooManyAnnotation);
        }
        if !annotation_key_regex().is_match(&key) {
            return Err(ConstraintError::InvalidAnnotationKey);
        }
        if let AnnotationValue::String(s) = annotation {
            if s.len() > 1000 {
                return Err(ConstraintError::InvalidAnnotationValue);
            }
        }
        self.annotations.insert(key, annotation);
        Ok(self)
    }

    /// Sets metadata.
    pub fn metadata(&mut self, key: &'a str, value: AnyValue<'a>) -> &mut Self {
        self.metadata.insert(key, value);
        self
    }

    #[cfg(feature = "subsegment-nesting")]
    /// Adds a subsegment to the segment by consuming a [SubsegmentBuilder].
    pub fn subsegment(
        &mut self,
        mut subsegment_builder: SubsegmentDocumentBuilder<'a>,
    ) -> &mut Self {
        subsegment_builder.nested = true;
        self.subsegments.push(subsegment_builder);
        self
    }

    /// Builds the segment document.
    ///
    /// # Errors
    ///
    /// Returns `ConstraintError` if required fields are missing or invalid.
    pub fn build(self) -> Result<SegmentDocument<'a>> {
        let name = self.name.ok_or(ConstraintError::MissingName)?;
        let id = self.id.ok_or(ConstraintError::MissingId)?;
        let start_time = self.start_time.ok_or(ConstraintError::MissingStartTime)?;

        DBT::verify_constraints(&self)?;

        #[cfg(feature = "subsegment-nesting")]
        let subsegments = self
            .subsegments
            .into_iter()
            .map(|ssb| ssb.build())
            .collect::<Result<_>>()?;

        Ok(SegmentDocument {
            subsegment_type: (!self.nested).then(|| DBT::segment_type()).flatten(),
            name,
            id,
            start_time,
            trace_id: (!self.nested).then_some(self.trace_id).flatten(),
            end_time: self.end_time,
            error_details: self.error_details.build()?,
            origin: self.origin,
            parent_id: (!self.nested).then_some(self.parent_id).flatten(),
            namespace: self.namespace,
            user: self.user,
            http: self.http.build(),
            aws: self.aws.build(),
            cloudwatch_logs: self.cloudwatch_logs.build(),
            service: self.service.build(),
            sql: self.sql.build(),
            annotations: self.annotations,
            metadata: self.metadata,
            #[cfg(feature = "subsegment-nesting")]
            subsegments,
            #[cfg(feature = "subsegment-nesting")]
            precursor_ids: self.precursor_ids,
        })
    }
}

impl<'a> SegmentDocumentBuilder<'a> {
    field_setter!(origin:Origin);

    pub fn service(&mut self) -> &mut ServiceDataBuilder<'a> {
        &mut self.service
    }

    /// Sets the segment name.
    ///
    /// Must be 1-200 characters containing Unicode letters, numbers, whitespace, and the symbols:
    /// `_`, `.`, `:`, `/`, `%`, `&`, `#`, `=`, `+`, `\`, `-`, `@`.
    ///
    /// # Errors
    ///
    /// Returns `ConstraintError::InvalidName` if the name is empty, longer than 200 characters, or doesn't match the required pattern.
    pub fn name(&mut self, name: &'a str) -> Result<&mut Self> {
        if name.is_empty() {
            return Err(ConstraintError::InvalidName);
        }
        verify_string_length(name, 200).map_err(|_| ConstraintError::InvalidName)?;

        if !name_regex().is_match(name) {
            return Err(ConstraintError::InvalidName);
        }

        self.name = Some(name);
        Ok(self)
    }

    /// Sets the user identifier.
    ///
    /// # Errors
    ///
    /// Returns `ConstraintError::StringTooLong(250)` if longer than 250 characters.
    pub fn user(&mut self, user: Cow<'a, str>) -> Result<&mut Self> {
        verify_string_length(user.as_ref(), 250)?;
        self.user = Some(user);
        Ok(self)
    }
}
impl<'a> SubsegmentDocumentBuilder<'a> {
    field_setter!(namespace:Namespace);

    pub fn sql(&mut self) -> &mut SqlDataBuilder<'a> {
        &mut self.sql
    }

    /// Sets the subsegment name.
    ///
    /// Must be 1-250 characters containing Unicode letters, numbers, whitespace, and the symbols:
    /// `_`, `.`, `:`, `/`, `%`, `&`, `#`, `=`, `+`, `\`, `-`, `@`.
    ///
    /// # Errors
    ///
    /// Returns `ConstraintError::InvalidName` if the name is empty, longer than 250 characters, or doesn't match the required pattern.
    pub fn name(&mut self, name: &'a str) -> Result<&mut Self> {
        if name.is_empty() {
            return Err(ConstraintError::InvalidName);
        }

        verify_string_length(name, 250).map_err(|_| ConstraintError::InvalidName)?;

        if !name_regex().is_match(name) {
            return Err(ConstraintError::InvalidName);
        }

        self.name = Some(name);
        Ok(self)
    }

    #[cfg(feature = "subsegment-nesting")]
    /// Adds precursor IDs.
    ///
    /// Sets IDs of subsegments with the same parent that completed before this one.
    pub fn precursor_ids(&mut self, precursor_ids: Vec<Id>) -> &mut Self {
        self.precursor_ids = precursor_ids;
        self
    }
}

/// Information about the CloudWatch log group related to the segment.
#[derive(Debug, Serialize)]
pub(super) struct CloudwatchLogs<'a> {
    /// The ARNs of the CloudWatch LogGroups
    #[serde(skip_serializing_if = "MaybeSkip::skip")]
    arn: Option<&'a dyn StrList>,

    /// The ARNs or names of the CloudWatch LogGroups
    #[serde(skip_serializing_if = "MaybeSkip::skip")]
    log_group: Option<&'a dyn StrList>,
}

impl MaybeSkip for CloudwatchLogs<'_> {
    /// Returns true if this EC2 metadata is empty (all fields are None)
    fn skip(&self) -> bool {
        self.arn.skip() && self.log_group.skip()
    }
}

/// Builder for constructing CloudWatch Logs metadata.
#[derive(Debug, Default)]
pub(crate) struct CloudwatchLogsBuilder<'a> {
    arn: Option<&'a dyn StrList>,
    log_group: Option<&'a dyn StrList>,
}

impl<'a> CloudwatchLogsBuilder<'a> {
    field_setter!(arn:&'a dyn StrList);
    field_setter!(log_group:&'a dyn StrList);

    /// Builds the `CloudwatchLogs` instance.
    fn build(self) -> CloudwatchLogs<'a> {
        CloudwatchLogs {
            arn: self.arn,
            log_group: self.log_group,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::borrow::Cow;
    use std::sync::Mutex;
    use std::time::{SystemTime, UNIX_EPOCH};

    use rand::{
        distr::{Distribution, StandardUniform},
        rngs::StdRng,
        Rng, SeedableRng,
    };
    fn rng_gen<T>() -> T
    where
        StandardUniform: Distribution<T>,
    {
        thread_local! {
            static RNG : Mutex<StdRng> = Mutex::new(StdRng::seed_from_u64(42));
        }
        RNG.with(|rng| rng.lock().unwrap().random())
    }

    // Helper function to create a valid trace ID with a specific timestamp offset
    fn create_trace_id_with_offset(offset_seconds: i64) -> TraceId {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("EPOCH is earlier")
            .as_secs();
        let timestamp = if offset_seconds >= 0 {
            now + offset_seconds as u64
        } else {
            now - offset_seconds.unsigned_abs()
        };
        let random_part: u128 = rng_gen();
        // Create TraceId from u128: timestamp (32 bits) + random (96 bits)
        let trace_id_u128 = ((timestamp as u128) << 96) | (random_part >> 32);
        TraceId::from(trace_id_u128)
    }

    // Helper function to create a minimal valid segment builder
    fn create_minimal_segment_builder() -> SegmentDocumentBuilder<'static> {
        let mut builder = SegmentDocumentBuilder::default();
        builder.name("test-segment").unwrap();
        builder.id(Id::from(0x12345u64));
        builder.start_time(1234567890.0);
        builder
            .trace_id(create_trace_id_with_offset(0), false)
            .unwrap();
        builder
    }

    // Helper function to create a minimal valid subsegment builder
    fn create_minimal_subsegment_builder() -> SubsegmentDocumentBuilder<'static> {
        let mut builder = SubsegmentDocumentBuilder::default();
        builder.name("test-subsegment").unwrap();
        builder.id(Id::from(0x54321u64));
        builder.parent_id(Id::from(12345u64));
        builder.start_time(1234567890.0);
        builder
            .trace_id(create_trace_id_with_offset(0), false)
            .unwrap();
        builder
    }

    // Helper function to create a minimal valid subsegment builder
    fn create_minimal_subsegment_without_parent_builder() -> SubsegmentDocumentBuilder<'static> {
        let mut builder = SubsegmentDocumentBuilder::default();
        builder.name("test-subsegment").unwrap();
        builder.id(Id::from(54321u64));
        builder.start_time(1234567890.0);
        builder
            .trace_id(create_trace_id_with_offset(0), false)
            .unwrap();
        builder
    }

    // Tests for DocumentBuilder::trace_id

    #[test]
    fn trace_id_valid() {
        let mut builder = SegmentDocumentBuilder::default();

        // Valid: current timestamp
        let trace_id_now = create_trace_id_with_offset(0);
        assert!(builder.trace_id(trace_id_now, false).is_ok());

        // Valid: timestamp 29 days in the past (within 30 day limit)
        let mut builder2 = SegmentDocumentBuilder::default();
        let trace_id_29_days_ago = create_trace_id_with_offset(-(29 * 24 * 60 * 60));
        assert!(builder2.trace_id(trace_id_29_days_ago, false).is_ok());

        // Valid: timestamp 4 minutes in the future (within 5 minute skew)
        let mut builder3 = SegmentDocumentBuilder::default();
        let trace_id_4_min_future = create_trace_id_with_offset(4 * 60);
        assert!(builder3.trace_id(trace_id_4_min_future, false).is_ok());
    }

    #[test]
    fn trace_id_invalid() {
        // Invalid: timestamp 31 days in the past (exceeds 30 day limit)
        let mut builder1 = SegmentDocumentBuilder::default();
        let trace_id_31_days_ago = create_trace_id_with_offset(-(31 * 24 * 60 * 60));
        assert!(matches!(
            builder1.trace_id(trace_id_31_days_ago, false),
            Err(ConstraintError::InvalidTraceId)
        ));

        // Invalid: timestamp 6 minutes in the future (exceeds 5 minute skew)
        let mut builder2 = SegmentDocumentBuilder::default();
        let trace_id_6_min_future = create_trace_id_with_offset(6 * 60);
        assert!(matches!(
            builder2.trace_id(trace_id_6_min_future, false),
            Err(ConstraintError::InvalidTraceId)
        ));
    }

    // Tests for DocumentBuilder::end_time

    #[test]
    fn end_time_valid() {
        // Valid: end_time equals start_time
        let mut builder1 = SegmentDocumentBuilder::default();
        builder1.start_time(1234567890.0);
        assert!(builder1.end_time(1234567890.0).is_ok());

        // Valid: end_time after start_time
        let mut builder2 = SegmentDocumentBuilder::default();
        builder2.start_time(1234567890.0);
        assert!(builder2.end_time(1234567900.0).is_ok());
    }

    #[test]
    fn end_time_invalid() {
        // Invalid: end_time before start_time
        let mut builder = SegmentDocumentBuilder::default();
        builder.start_time(1234567890.0);
        assert!(matches!(
            builder.end_time(1234567880.0),
            Err(ConstraintError::EndTimeBeforeStartTime)
        ));
    }

    // Tests for DocumentBuilder::annotation

    #[test]
    fn annotation_valid() {
        let mut builder = create_minimal_segment_builder();

        // Valid annotation with string value
        assert!(builder
            .annotation("valid_key".into(), AnnotationValue::String("value"))
            .is_ok());

        // Valid annotation with number value
        assert!(builder
            .annotation("number_key".into(), AnnotationValue::Int(42))
            .is_ok());

        // Valid annotation with boolean value
        assert!(builder
            .annotation("bool_key".into(), AnnotationValue::Boolean(true))
            .is_ok());

        // Valid annotation with 1000 character string (at limit)
        let long_value = "a".repeat(1000);
        assert!(builder
            .annotation(
                "long_value_key".into(),
                AnnotationValue::String(&long_value)
            )
            .is_ok());
    }

    #[test]
    fn annotation_invalid_key() {
        let mut builder = create_minimal_segment_builder();

        // Invalid key: contains special characters not allowed
        assert!(matches!(
            builder.annotation("invalid-key!".into(), AnnotationValue::String("value")),
            Err(ConstraintError::InvalidAnnotationKey)
        ));

        // Invalid key: contains spaces
        assert!(matches!(
            builder.annotation("invalid key".into(), AnnotationValue::String("value")),
            Err(ConstraintError::InvalidAnnotationKey)
        ));
    }

    #[test]
    fn annotation_invalid_value() {
        let mut builder = create_minimal_segment_builder();

        // Invalid value: string longer than 1000 characters
        let too_long_value = "a".repeat(1001);
        assert!(matches!(
            builder.annotation("key".into(), AnnotationValue::String(&too_long_value)),
            Err(ConstraintError::InvalidAnnotationValue)
        ));
    }

    // Tests for DocumentBuilder::build

    #[test]
    fn build_valid_segment() {
        let builder = create_minimal_segment_builder();
        let result = builder.build();
        assert!(result.is_ok());

        let document = result.unwrap();
        let json = document.to_string();
        assert!(json.contains("\"name\":\"test-segment\""));
        assert!(json.contains("\"id\":\"0000000000012345\""));
    }

    #[test]
    fn build_valid_subsegment() {
        let builder = create_minimal_subsegment_builder();
        let result = builder.build();
        assert!(result.is_ok());

        let document = result.unwrap();
        let json = document.to_string();
        assert!(json.contains("\"name\":\"test-subsegment\""));
        assert!(json.contains("\"id\":\"0000000000054321\""));
    }

    #[test]
    fn build_missing_id() {
        let mut builder = SegmentDocumentBuilder::default();
        builder.name("test-segment").unwrap();
        builder.start_time(1234567890.0);
        builder
            .trace_id(create_trace_id_with_offset(0), false)
            .unwrap();
        // Missing: id

        assert!(matches!(builder.build(), Err(ConstraintError::MissingId)));
    }

    #[test]
    fn build_missing_name() {
        let mut builder = SegmentDocumentBuilder::default();
        builder.id(Id::from(12345u64));
        builder.start_time(1234567890.0);
        builder
            .trace_id(create_trace_id_with_offset(0), false)
            .unwrap();
        // Missing: name

        assert!(matches!(builder.build(), Err(ConstraintError::MissingName)));
    }

    #[test]
    fn build_missing_start_time() {
        let mut builder = SegmentDocumentBuilder::default();
        builder.name("test-segment").unwrap();
        builder.id(Id::from(12345u64));
        builder
            .trace_id(create_trace_id_with_offset(0), false)
            .unwrap();
        // Missing: start_time

        assert!(matches!(
            builder.build(),
            Err(ConstraintError::MissingStartTime)
        ));
    }

    #[test]
    fn build_missing_trace_id() {
        let mut builder = SegmentDocumentBuilder::default();
        builder.name("test-segment").unwrap();
        builder.id(Id::from(12345u64));
        builder.start_time(1234567890.0);
        // Missing: trace_id (required for segments)

        assert!(matches!(
            builder.build(),
            Err(ConstraintError::MissingTraceId)
        ));
    }

    #[test]
    fn build_missing_parent_id() {
        let builder = create_minimal_subsegment_without_parent_builder();
        // Missing: parent_id (required for subsegments that are not nested)

        assert!(matches!(
            builder.build(),
            Err(ConstraintError::MissingParentId)
        ));
        let mut seg_builder = create_minimal_segment_builder();
        let subseg_builder = create_minimal_subsegment_without_parent_builder();
        seg_builder.subsegment(subseg_builder);
        // Missing: parent_id, but nested
        assert!(seg_builder.build().is_ok());
    }

    #[test]
    fn build_constraint_violations() {
        // Test that constraint violations during build are caught
        let mut builder = SegmentDocumentBuilder::default();
        builder.name("test-segment").unwrap();
        builder.id(Id::from(12345u64));
        builder.start_time(1234567890.0);
        builder
            .trace_id(create_trace_id_with_offset(0), false)
            .unwrap();

        // Add a subsegment with missing required fields
        let mut subsegment = SubsegmentDocumentBuilder::default();
        subsegment.name("subseg").unwrap();
        // Missing id, parent_id, start_time for subsegment

        builder.subsegment(subsegment);

        // Build should fail due to subsegment constraint violations
        let result = builder.build();
        assert!(result.is_err());
    }

    // Tests for SegmentDocumentBuilder::name

    #[test]
    fn segment_name_valid() {
        let mut builder = SegmentDocumentBuilder::default();

        // Valid: simple name
        assert!(builder.name("simple_name").is_ok());

        // Valid: name with allowed special characters
        let mut builder2 = SegmentDocumentBuilder::default();
        assert!(builder2
            .name("name_with.special:chars/test%20#value")
            .is_ok());

        // Valid: 200 character name (at limit)
        let mut builder3 = SegmentDocumentBuilder::default();
        let long_name = "a".repeat(200);
        let long_name_static: &'static str = Box::leak(long_name.into_boxed_str());
        assert!(builder3.name(long_name_static).is_ok());

        // Valid: Unicode characters
        let mut builder4 = SegmentDocumentBuilder::default();
        assert!(builder4.name("测试名称").is_ok());
    }

    #[test]
    fn segment_name_invalid() {
        // Invalid: empty name
        let mut builder1 = SegmentDocumentBuilder::default();
        assert!(matches!(
            builder1.name(""),
            Err(ConstraintError::InvalidName)
        ));

        // Invalid: name longer than 200 characters
        let mut builder2 = SegmentDocumentBuilder::default();
        let too_long = "a".repeat(201);
        let too_long_static: &'static str = Box::leak(too_long.into_boxed_str());
        assert!(matches!(
            builder2.name(too_long_static),
            Err(ConstraintError::InvalidName)
        ));

        // Invalid: name with disallowed characters (e.g., control characters)
        let mut builder3 = SegmentDocumentBuilder::default();
        assert!(matches!(
            builder3.name("name\x00with\x01control"),
            Err(ConstraintError::InvalidName)
        ));
    }

    // Tests for SegmentDocumentBuilder::user

    #[test]
    fn user_valid() {
        let mut builder = SegmentDocumentBuilder::default();

        // Valid: user string within 250 character limit
        assert!(builder.user(Cow::Borrowed("test-user")).is_ok());

        // Valid: 250 character user string (at limit)
        let mut builder2 = SegmentDocumentBuilder::default();
        let long_user = "u".repeat(250);
        assert!(builder2.user(Cow::Owned(long_user)).is_ok());
    }

    #[test]
    fn user_invalid() {
        let mut builder = SegmentDocumentBuilder::default();

        // Invalid: user string longer than 250 characters
        let too_long_user = "u".repeat(251);
        assert!(matches!(
            builder.user(Cow::Owned(too_long_user)),
            Err(ConstraintError::StringTooLong(250))
        ));
    }

    // Tests for SubsegmentDocumentBuilder::name

    #[test]
    fn subsegment_name_valid() {
        let mut builder = SubsegmentDocumentBuilder::default();

        // Valid: simple name
        assert!(builder.name("simple_subsegment").is_ok());

        // Valid: name with allowed special characters
        let mut builder2 = SubsegmentDocumentBuilder::default();
        assert!(builder2
            .name("subseg_with.special:chars/test%20#value")
            .is_ok());

        // Valid: 250 character name (at limit)
        let mut builder3 = SubsegmentDocumentBuilder::default();
        let long_name = "s".repeat(250);
        let long_name_static: &'static str = Box::leak(long_name.into_boxed_str());
        assert!(builder3.name(long_name_static).is_ok());

        // Valid: Unicode characters
        let mut builder4 = SubsegmentDocumentBuilder::default();
        assert!(builder4.name("子段名称").is_ok());
    }

    #[test]
    fn subsegment_name_invalid() {
        // Invalid: empty name
        let mut builder1 = SubsegmentDocumentBuilder::default();
        assert!(matches!(
            builder1.name(""),
            Err(ConstraintError::InvalidName)
        ));

        // Invalid: name longer than 250 characters
        let mut builder2 = SubsegmentDocumentBuilder::default();
        let too_long = "s".repeat(251);
        let too_long_static: &'static str = Box::leak(too_long.into_boxed_str());
        assert!(matches!(
            builder2.name(too_long_static),
            Err(ConstraintError::InvalidName)
        ));

        // Invalid: name with disallowed characters (e.g., control characters)
        let mut builder3 = SubsegmentDocumentBuilder::default();
        assert!(matches!(
            builder3.name("subseg\x00with\x01control"),
            Err(ConstraintError::InvalidName)
        ));
    }

    // Tests for DocumentBuilder::annotation — TooManyAnnotation error path

    #[test]
    fn annotation_too_many() {
        let mut builder = create_minimal_segment_builder();

        // Add 50 annotations successfully
        for i in 0..50 {
            let key = format!("key_{i}");
            assert!(
                builder
                    .annotation(Cow::Owned(key), AnnotationValue::Int(i))
                    .is_ok(),
                "annotation {i} should succeed"
            );
        }

        // The 51st annotation should fail with TooManyAnnotation
        assert!(matches!(
            builder.annotation(Cow::Borrowed("key_50"), AnnotationValue::Int(50)),
            Err(ConstraintError::TooManyAnnotation)
        ));
    }

    // Tests for DocumentBuilder::end_time — edge case: no start_time set

    #[test]
    fn end_time_without_start_time() {
        // Calling end_time() when start_time is None should succeed (no validation error)
        let mut builder = SegmentDocumentBuilder::default();
        assert!(builder.end_time(1234567890.0).is_ok());
    }

    // Tests for DocumentBuilder::trace_id with skip_timestamp_validation=true

    #[test]
    fn trace_id_skip_timestamp_validation() {
        let mut builder = SegmentDocumentBuilder::default();

        // Ancient timestamp (epoch 100) — would fail normal validation but should
        // succeed with skip_timestamp_validation=true
        let ancient_trace_id = TraceId::from(100u128 << 96);
        assert!(builder.trace_id(ancient_trace_id, true).is_ok());
    }

    // Tests for SegmentDocument serialization — skip_serializing_if behavior

    #[test]
    fn serialization_minimal_segment_omits_optional_fields() {
        let builder = create_minimal_segment_builder();
        let document = builder.build().unwrap();
        let json = document.to_string();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        let obj = parsed.as_object().unwrap();

        // Required fields must be present
        assert!(obj.contains_key("name"), "name must be present");
        assert!(obj.contains_key("id"), "id must be present");
        assert!(obj.contains_key("start_time"), "start_time must be present");
        assert!(obj.contains_key("trace_id"), "trace_id must be present");

        // Optional fields must be absent on a minimal segment
        assert!(!obj.contains_key("end_time"), "end_time must be absent");
        assert!(!obj.contains_key("http"), "http must be absent");
        assert!(!obj.contains_key("sql"), "sql must be absent");
        assert!(!obj.contains_key("aws"), "aws must be absent");
        assert!(!obj.contains_key("cause"), "cause must be absent");
        assert!(
            !obj.contains_key("annotations"),
            "annotations must be absent"
        );
        assert!(!obj.contains_key("metadata"), "metadata must be absent");
        assert!(
            !obj.contains_key("subsegments"),
            "subsegments must be absent"
        );
        assert!(!obj.contains_key("user"), "user must be absent");
        assert!(!obj.contains_key("origin"), "origin must be absent");
        assert!(!obj.contains_key("parent_id"), "parent_id must be absent");
        assert!(!obj.contains_key("namespace"), "namespace must be absent");
        assert!(!obj.contains_key("service"), "service must be absent");
        assert!(
            !obj.contains_key("type"),
            "type must be absent for segments"
        );
        assert!(
            !obj.contains_key("cloudwatch_logs"),
            "cloudwatch_logs must be absent"
        );
        assert!(
            !obj.contains_key("precursor_ids"),
            "precursor_ids must be absent"
        );
    }

    #[test]
    fn serialization_segment_with_optional_fields() {
        let mut builder = create_minimal_segment_builder();
        builder.end_time(1234567900.0).unwrap();
        builder.user(Cow::Borrowed("test_user")).unwrap();
        builder
            .annotation(Cow::Borrowed("my_key"), AnnotationValue::String("my_value"))
            .unwrap();
        builder.http().request.method(Cow::Borrowed("GET")).unwrap();

        let document = builder.build().unwrap();
        let json = document.to_string();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        let obj = parsed.as_object().unwrap();

        // Populated optional fields must be present
        assert!(obj.contains_key("end_time"), "end_time must be present");
        assert!(obj.contains_key("user"), "user must be present");
        assert!(
            obj.contains_key("annotations"),
            "annotations must be present"
        );
        assert!(obj.contains_key("http"), "http must be present");

        // Verify values
        assert_eq!(obj["end_time"].as_f64().unwrap(), 1234567900.0);
        assert_eq!(obj["user"].as_str().unwrap(), "test_user");
        assert!(obj["annotations"]
            .as_object()
            .unwrap()
            .contains_key("my_key"));
        assert!(obj["http"].as_object().unwrap().contains_key("request"));
    }
}
