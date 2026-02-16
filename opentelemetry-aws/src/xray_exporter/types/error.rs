use opentelemetry_sdk::ExportError;

/// Validation errors for segment document constraints.
#[allow(missing_docs)]
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConstraintError {
    #[error("Missing the ID")]
    MissingId,
    #[error("Missing the ParentId for non-nested subsegment")]
    MissingParentId,
    #[error("Missing the Name")]
    MissingName,
    #[error("Missing the StartTime")]
    MissingStartTime,
    #[error("Missing the TraceId")]
    MissingTraceId,
    #[error("Invalid ID: must be exactly 16 hexadecimal characters")]
    InvalidId,
    #[error("Invalid trace ID: must match pattern \\d+-[A-Fa-f0-9]*-[A-Fa-f0-9]{{24}} and be at least 35 characters")]
    InvalidTraceId,
    #[error("Invalid name: must be 1-200 characters and contain only allowed characters")]
    InvalidName,
    #[error("Invalid annotation key: must be 1-500 alphanumeric characters or underscores")]
    InvalidAnnotationKey,
    #[error("Invalid annotation value: string values must be at most 1000 characters")]
    InvalidAnnotationValue,
    #[error("No more than 50 annotations can be added to a XRay segment")]
    TooManyAnnotation,
    #[error("End time cannot be before start time")]
    EndTimeBeforeStartTime,
    #[error("The provided value is too long (more than {0} chars)")]
    StringTooLong(usize),
    #[error("Cannot add an empty StackFrame to the stack")]
    EmptyStackFrame,
    #[error("Cannot set a cause without any error flag set")]
    CauseWithoutError,
    #[error("Cause must be either by Reference OR with details")]
    CauseIdAndDetailsSet,
    #[error("Invalid Origin value: {0}")]
    InvalidOrigin(String),
}

impl ExportError for ConstraintError {
    fn exporter_name(&self) -> &'static str {
        "xray_exporter"
    }
}

pub(crate) type Result<T> = core::result::Result<T, ConstraintError>;
