use std::borrow::Cow;

use serde::Serialize;

use super::{
    error::{ConstraintError, Result},
    id::Id,
    utils::MaybeSkip,
    value::StrList,
};
use crate::{field_setter, flag_setter};

/// Information about errors and exceptions that occurred for a segment or subsegment.
#[derive(Debug, Serialize)]
pub(super) struct ErrorDetails<'a> {
    /// Set to true if a fault occurred (5XX server error)
    #[serde(skip_serializing_if = "MaybeSkip::skip")]
    fault: bool,

    /// Set to true if an error occurred (4XX client error)
    #[serde(skip_serializing_if = "MaybeSkip::skip")]
    error: bool,

    /// Set to true if the request was throttled (429 Too Many Requests)
    #[serde(skip_serializing_if = "MaybeSkip::skip")]
    throttle: bool,

    /// Information about the cause of an error or fault
    #[serde(skip_serializing_if = "Option::is_none")]
    cause: Option<Cause<'a>>,
}

/// Information about the cause of an error or fault.
///
/// A cause can be either a reference to another segment by ID, or detailed
/// information including working directory, paths, and exceptions.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub(super) enum Cause<'a> {
    /// Reference to an exception by its 16-character ID
    Reference(Id),
    /// Detailed cause information
    Details(CauseData<'a>),
}

/// Detailed information about the cause of an error or fault.
#[derive(Debug, Serialize)]
pub(super) struct CauseData<'a> {
    /// The full path of the working directory when the exception occurred
    #[serde(skip_serializing_if = "MaybeSkip::skip")]
    working_directory: Option<Cow<'a, str>>,

    /// Array of paths to libraries or modules in use when the exception occurred
    #[serde(skip_serializing_if = "MaybeSkip::skip")]
    paths: Option<&'a dyn StrList>,

    /// Array of exception objects providing detailed error information
    #[serde(skip_serializing_if = "Vec::is_empty")]
    exceptions: Vec<Exception<'a>>,
}

/// Builder for constructing error details with fault/error flags and cause information.
#[derive(Debug, Default)]
pub(crate) struct ErrorDetailsBuilder<'a> {
    fault: bool,
    error: bool,
    throttle: bool,
    id: Option<Id>,
    working_directory: Option<Cow<'a, str>>,
    paths: Option<&'a dyn StrList>,
    exceptions: Vec<Exception<'a>>,
}

impl<'a> ErrorDetailsBuilder<'a> {
    flag_setter!(fault);
    flag_setter!(error);
    flag_setter!(throttle);

    // field_setter!(id:Id);
    // field_setter!(working_directory);

    // /// Sets the array of library/module paths.
    // ///
    // /// # Arguments
    // ///
    // /// * `paths` - Array of paths to libraries or modules in use when the exception occurred
    // pub fn paths(&mut self, paths: &'a dyn StrList) -> &mut Self {
    //     self.paths = Some(paths);
    //     self
    // }

    /// Add an exception
    pub fn exception(&mut self, exception: Exception<'a>) -> &mut Self {
        self.exceptions.push(exception);
        self
    }

    /// Builds the `ErrorDetails` instance.
    ///
    /// # Errors
    ///
    /// Returns `ConstraintError::CauseWithoutError` if a cause field is set without any error flag being true.
    /// Returns `ConstraintError::CauseIdAndDetailsSet` if `id` **and** any other cause field was set.
    pub(super) fn build(self) -> Result<ErrorDetails<'a>> {
        let cause = match self {
            Self {
                id: Some(id),
                working_directory: None,
                paths,
                exceptions,
                ..
            } if paths.is_none() && exceptions.is_empty() => Some(Cause::Reference(id)),
            Self {
                id: None,
                working_directory: None,
                paths,
                exceptions,
                ..
            } if paths.is_none() && exceptions.is_empty() => None,
            Self {
                id: None,
                working_directory,
                paths,
                exceptions,
                ..
            } => Some(Cause::Details(CauseData {
                working_directory,
                paths,
                exceptions,
            })),
            _ => return Err(ConstraintError::CauseIdAndDetailsSet),
        };

        let Self {
            fault,
            error,
            throttle,
            ..
        } = self;

        if cause.is_some() && !(fault || error || throttle) {
            Err(ConstraintError::CauseWithoutError)
        } else {
            Ok(ErrorDetails {
                fault,
                error,
                throttle,
                cause,
            })
        }
    }
}

/// Exception information with stack trace and optional cause chain.
#[derive(Debug, Serialize)]
pub(crate) struct Exception<'a> {
    /// A 64-bit identifier for the exception, unique among segments in the same trace
    id: Id,

    /// The exception message describing what went wrong
    #[serde(skip_serializing_if = "MaybeSkip::skip")]
    message: Option<Cow<'a, str>>,

    /// The type or class name of the exception
    #[serde(rename = "type", skip_serializing_if = "MaybeSkip::skip")]
    exception_type: Option<Cow<'a, str>>,

    /// Boolean indicating that the exception was caused by an error returned by a downstream service
    #[serde(skip_serializing_if = "MaybeSkip::skip")]
    remote: bool,

    // /// Integer indicating the number of stack frames omitted from the stack trace
    // #[serde(skip_serializing_if = "Option::is_none")]
    // truncated: Option<i32>,

    // /// Integer indicating the number of exceptions skipped between this exception and its child
    // #[serde(skip_serializing_if = "Option::is_none")]
    // skipped: Option<i32>,

    // /// Exception ID of the exception's parent (the exception that caused this one)
    // #[serde(skip_serializing_if = "Option::is_none")]
    // cause: Option<Id>,
    /// Array of stack frame objects showing the call stack when the exception occurred
    #[serde(skip_serializing_if = "Vec::is_empty")]
    stack: Vec<StackFrame<'a>>,
}

/// Builder for constructing exception details.
#[derive(Debug, Default)]
pub(crate) struct ExceptionBuilder<'a> {
    id: Option<Id>,
    message: Option<Cow<'a, str>>,
    exception_type: Option<Cow<'a, str>>,
    remote: bool,
    // truncated: Option<i32>,
    // skipped: Option<i32>,
    // cause: Option<Id>,
    stack: Vec<StackFrame<'a>>,
}

impl<'a> ExceptionBuilder<'a> {
    // field_setter!(id:Id);
    field_setter!(message);
    field_setter!(exception_type);
    // field_setter!(truncated:i32);
    // field_setter!(skipped:i32);
    // field_setter!(cause:Id);

    flag_setter!(remote);

    /// Add a [StackFrame] to the stack trace.
    ///
    /// # Arguments
    ///
    /// * `stack_frame` - A frame object
    ///
    /// # Errors
    ///
    /// Returns `ConstraintError::EmptyStackFrame` if the `stack_frame` is empty.
    pub fn stack_frame(&mut self, stack_frame: StackFrame<'a>) -> Result<&mut Self> {
        if stack_frame.skip() {
            Err(ConstraintError::EmptyStackFrame)
        } else {
            self.stack.push(stack_frame);
            Ok(self)
        }
    }

    /// Builds the `Exception` instance.
    ///
    /// # Errors
    ///
    /// Returns `ConstraintError::MissingId` if the id field was not set.
    pub fn build(self) -> Result<Exception<'a>> {
        Ok(Exception {
            id: self.id.unwrap_or_else(Id::new),
            message: self.message,
            exception_type: self.exception_type,
            remote: self.remote,
            // truncated: self.truncated,
            // skipped: self.skipped,
            // cause: self.cause,
            stack: self.stack,
        })
    }
}

/// Single frame in an exception stack trace.
#[derive(Debug, Serialize)]
pub(crate) struct StackFrame<'a> {
    /// The relative path to the file where the function is defined
    #[serde(skip_serializing_if = "MaybeSkip::skip")]
    path: Option<Cow<'a, str>>,

    /// The line number in the file where the exception occurred
    #[serde(skip_serializing_if = "Option::is_none")]
    line: Option<i32>,

    /// The function or method name
    #[serde(skip_serializing_if = "MaybeSkip::skip")]
    label: Option<Cow<'a, str>>,
}

impl MaybeSkip for StackFrame<'_> {
    fn skip(&self) -> bool {
        self.path.skip() && self.line.is_none() && self.label.skip()
    }
}

/// Builder for constructing stack frame details.
#[derive(Debug, Default)]
pub(crate) struct StackFrameBuilder<'a> {
    path: Option<Cow<'a, str>>,
    line: Option<i32>,
    label: Option<Cow<'a, str>>,
}

impl<'a> StackFrameBuilder<'a> {
    field_setter!(path);
    field_setter!(line:i32);
    field_setter!(label);

    /// Builds the `StackFrame` instance.
    pub fn build(self) -> StackFrame<'a> {
        StackFrame {
            path: self.path,
            line: self.line,
            label: self.label,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::borrow::Cow;

    // Helper function to create a basic stack frame with at least one field
    fn create_stack_frame<'a>(
        path: Option<&'a str>,
        line: Option<i32>,
        label: Option<&'a str>,
    ) -> StackFrame<'a> {
        StackFrame {
            path: path.map(Cow::Borrowed),
            line,
            label: label.map(Cow::Borrowed),
        }
    }

    // Tests for ErrorDetailsBuilder::build

    #[test]
    fn error_details_builder_build_valid_with_cause_reference() {
        // Valid: cause reference (id only) with error flag
        let mut builder = ErrorDetailsBuilder::default();
        builder.error();
        builder.id = Some(Id::from(0x1234567890abcdef));
        let result = builder.build();
        assert!(result.is_ok());
        let error_details = result.unwrap();
        assert!(error_details.error);
        assert!(matches!(error_details.cause, Some(Cause::Reference(_))));
    }

    #[test]
    fn error_details_builder_build_valid_with_cause_details() {
        // Valid: cause details (exceptions) with fault flag
        let exception = ExceptionBuilder::default().build().unwrap();
        let mut builder = ErrorDetailsBuilder::default();
        builder.fault();
        builder.exception(exception);
        let result = builder.build();
        assert!(result.is_ok());
        let error_details = result.unwrap();
        assert!(error_details.fault);
        assert!(matches!(error_details.cause, Some(Cause::Details(_))));
    }

    #[test]
    fn error_details_builder_build_error_without_flags() {
        // Invalid: cause set without any error flag
        let builder = ErrorDetailsBuilder {
            id: Some(Id::from(0x1234567890abcdef)),
            ..Default::default()
        };
        let result = builder.build();
        assert!(matches!(result, Err(ConstraintError::CauseWithoutError)));
    }

    #[test]
    fn error_details_builder_build_both_id_and_details_set() {
        // Invalid: both id and details (exceptions) set
        let exception = ExceptionBuilder::default().build().unwrap();
        let mut builder = ErrorDetailsBuilder::default();
        builder.error();
        builder.id = Some(Id::from(0x1234567890abcdef));
        builder.exception(exception);
        let result = builder.build();
        assert!(matches!(result, Err(ConstraintError::CauseIdAndDetailsSet)));
    }

    // Tests for ExceptionBuilder::stack_frame

    #[test]
    fn exception_builder_stack_frame_valid() {
        // Valid: non-empty stack frame with path
        let stack_frame = create_stack_frame(Some("src/main.rs"), Some(42), None);
        let mut builder = ExceptionBuilder::default();
        let result = builder.stack_frame(stack_frame);
        assert!(result.is_ok());
        assert_eq!(builder.stack.len(), 1);

        // Valid: non-empty stack frame with label
        let stack_frame = create_stack_frame(None, None, Some("main"));
        let result = builder.stack_frame(stack_frame);
        assert!(result.is_ok());
        assert_eq!(builder.stack.len(), 2);

        // Valid: non-empty stack frame with line number
        let stack_frame = create_stack_frame(None, Some(100), None);
        let result = builder.stack_frame(stack_frame);
        assert!(result.is_ok());
        assert_eq!(builder.stack.len(), 3);
    }

    #[test]
    fn exception_builder_stack_frame_invalid() {
        // Invalid: empty stack frame (all fields None)
        let empty_frame = create_stack_frame(None, None, None);
        let mut builder = ExceptionBuilder::default();
        let result = builder.stack_frame(empty_frame);
        assert!(matches!(result, Err(ConstraintError::EmptyStackFrame)));
        assert_eq!(builder.stack.len(), 0);
    }

    // Tests for ExceptionBuilder::build

    #[test]
    fn exception_builder_build_complete() {
        // Complete exception with all fields
        let stack_frame1 = create_stack_frame(Some("src/lib.rs"), Some(10), Some("process"));
        let stack_frame2 = create_stack_frame(Some("src/main.rs"), Some(42), Some("main"));

        let mut builder = ExceptionBuilder::default();
        builder.message(Cow::Borrowed("Something went wrong"));
        builder.exception_type(Cow::Borrowed("RuntimeError"));
        builder.remote();
        builder.stack_frame(stack_frame1).unwrap();
        builder.stack_frame(stack_frame2).unwrap();

        let result = builder.build();
        assert!(result.is_ok());

        let exception = result.unwrap();
        assert_eq!(
            exception.message,
            Some(Cow::Borrowed("Something went wrong"))
        );
        assert_eq!(
            exception.exception_type,
            Some(Cow::Borrowed("RuntimeError"))
        );
        assert!(exception.remote);
        assert_eq!(exception.stack.len(), 2);
    }
}
