use opentelemetry_sdk::ExportError;

use crate::xray_exporter::types::error::ConstraintError;

/// Errors that can occur when translating OpenTelemetry spans to X-Ray segments.
///
/// These errors represent failures during the translation process, including missing
/// required span data and constraint violations in the resulting segment documents.
///
/// # Examples
///
/// ```no_run
/// use opentelemetry_aws::xray_exporter::{error::TranslationError, SegmentTranslator};
/// use opentelemetry_sdk::trace::SpanData;
///
/// let translator = SegmentTranslator::new();
/// let spans: Vec<SpanData> = vec![]; // Your span data
///
/// match translator.translate_spans(&spans) {
///     Ok(documents) => {
///         println!("Successfully translated {} spans", documents.len());
///     }
///     Err(TranslationError::MissingSpanId) => {
///         eprintln!("A span is missing a valid span ID");
///     }
///     Err(TranslationError::ConstraintError(err)) => {
///         eprintln!("Segment document constraint violated: {}", err);
///     }
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TranslationError {
    /// The OpenTelemetry span is missing a span ID.
    ///
    /// This error occurs when a span has an invalid (zero) span ID, which is required
    /// for creating an X-Ray segment document.
    #[error("Missing the OTel Span ID")]
    MissingSpanId,

    /// A segment document constraint was violated during translation.
    ///
    /// This error wraps a [`ConstraintError`] that occurred while building the
    /// segment document from the span data.
    #[error("Document constraint violated: {0}")]
    ConstraintError(ConstraintError),
}

impl From<ConstraintError> for TranslationError {
    fn from(value: ConstraintError) -> Self {
        Self::ConstraintError(value)
    }
}

impl ExportError for TranslationError {
    fn exporter_name(&self) -> &'static str {
        "xray_exporter"
    }
}

pub(crate) type Result<T> = core::result::Result<T, TranslationError>;
