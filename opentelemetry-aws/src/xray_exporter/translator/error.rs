use opentelemetry_sdk::ExportError;

use crate::xray_exporter::types::error::ConstraintError;

/// Errors that can occur when translating an OpenTelemetry span to an X-Ray segment.
///
/// These errors represent failures during the translation of individual spans,
/// including missing required span data and constraint violations in the resulting
/// segment documents. The [`SegmentTranslator::translate_spans`] method handles
/// these errors internally — spans that fail translation are silently dropped
/// and logged (when the `internal-logs` feature is enabled) rather than
/// propagated to the caller.
///
/// [`SegmentTranslator::translate_spans`]: crate::xray_exporter::SegmentTranslator::translate_spans
///
/// # Examples
///
/// Matching on error variants:
///
/// ```
/// use opentelemetry_aws::xray_exporter::error::{TranslationError, ConstraintError};
///
/// fn handle_translation_error(err: TranslationError) {
///     match err {
///         TranslationError::MissingSpanId => {
///             eprintln!("span is missing a valid span ID");
///         }
///         TranslationError::ConstraintError(constraint) => {
///             eprintln!("segment constraint violated: {constraint}");
///         }
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
