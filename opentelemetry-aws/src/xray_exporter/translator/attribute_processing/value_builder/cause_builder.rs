use std::borrow::Cow;

use opentelemetry::{
    trace::{Event, Status},
    KeyValue, Value,
};

use crate::xray_exporter::{
    translator::{
        attribute_processing::{get_cow, get_integer, get_str, semconv, SpanAttributeProcessor},
        error::Result,
        translate_timestamp, AnyDocumentBuilder,
    },
    types::{ExceptionBuilder, StackFrameBuilder},
};

use super::ValueBuilder;

const EXCEPTION_EVENT_NAME: &str = "exception";
const HTTP_EVENT_NAME: &str = "HTTP request failure";

/// Builds error/fault cause information from span events and HTTP status codes.
#[derive(Debug)]
pub(in crate::xray_exporter::translator) struct CauseBuilder<'a> {
    events: &'a [Event],
    span_status: &'a Status,
    span_is_remote: bool,
    rpc_system_is_aws_api: bool,
    sdk_lang: Option<&'a str>,
    http_status_code: Option<u16>,
    http_response_status_code: Option<u16>,
    http_status_text: Option<Cow<'a, str>>,
    aws_http_error_message: Option<&'a str>,
    aws_http_error_event: Option<&'a str>,
}

impl<'a> CauseBuilder<'a> {
    pub fn new(events: &'a [Event], span_status: &'a Status, span_is_remote: bool) -> Self {
        Self {
            events,
            span_status,
            span_is_remote,
            rpc_system_is_aws_api: false,
            sdk_lang: None,
            http_status_code: None,
            http_response_status_code: None,
            http_status_text: None,
            aws_http_error_message: None,
            aws_http_error_event: None,
        }
    }

    fn rpc_system_is_aws_api(&mut self, value: &'a Value) -> bool {
        if value.as_str() == "aws-api" {
            self.rpc_system_is_aws_api = true;
        }
        false
    }

    fn sdk_lang(&mut self, value: &'a Value) -> bool {
        self.sdk_lang = get_str(value);
        self.sdk_lang.is_some()
    }

    fn http_status_code(&mut self, value: &'a Value) -> bool {
        self.http_status_code = get_integer(value).map(|code| code as u16);
        self.http_status_code.is_some()
    }

    fn http_response_status_code(&mut self, value: &'a Value) -> bool {
        self.http_response_status_code = get_integer(value).map(|code| code as u16);
        self.http_response_status_code.is_some()
    }

    fn http_status_text(&mut self, value: &'a Value) -> bool {
        self.http_status_text = Some(get_cow(value));
        true
    }

    fn aws_http_error_message(&mut self, value: &'a Value) -> bool {
        self.aws_http_error_message = get_str(value);
        self.aws_http_error_message.is_some()
    }

    fn aws_http_error_event(&mut self, value: &'a Value) -> bool {
        self.aws_http_error_event = get_str(value);
        self.aws_http_error_event.is_some()
    }
}

impl<'value> ValueBuilder<'value> for CauseBuilder<'value> {
    fn resolve(self, segment_builder: &mut AnyDocumentBuilder<'value>) -> Result<()> {
        let span_in_error = matches!(self.span_status, Status::Error { .. });
        let has_exceptions = self.events.iter().any(|e| {
            let event_name = e.name.as_ref();
            event_name == EXCEPTION_EVENT_NAME
                || self.rpc_system_is_aws_api && event_name == HTTP_EVENT_NAME
        });
        // If the span is not in error, just return
        if !(span_in_error || has_exceptions) {
            return Ok(());
        }

        // Else set appropriate fields on the error_details

        let error_details = match segment_builder {
            AnyDocumentBuilder::Segment(document_builder) => document_builder.error_details(),
            AnyDocumentBuilder::Subsegment(document_builder) => document_builder.error_details(),
        };

        // Interprete the HTTP code, if present
        let code = self.http_status_code.or(self.http_response_status_code);
        match code {
            Some(429) => {
                error_details.throttle().error();
            }
            Some(400..=499) => {
                error_details.error();
            }
            Some(500..=599) => {
                error_details.fault();
            }
            _ if span_in_error => {
                error_details.fault();
            }
            _ => (),
        }

        // Search the associated Events to find information
        for event in self.events {
            // An event is interesting if:
            // - Its name is "exception" (EXCEPTION_EVENT_NAME) => inherited from the Go Xray exporter
            // - Its name is "HTTP request failure" (HTTP_EVENT_NAME) for AWS spans => inherited from the Go Xray exporter
            // - Contains exception.type or exception.message SemConv attributes
            //
            // If an event is interesting, we extract has much as we can to include in an "Exception" object
            let mut exception = ExceptionBuilder::default();
            let mut stack_frame_builder = StackFrameBuilder::default();
            let mut include_exception = false;
            let mut exception_message_set = false;
            if self.span_is_remote {
                exception.remote();
            }
            match event.name.as_ref() {
                EXCEPTION_EVENT_NAME => include_exception = true,
                // All the HTTP_EVENT_NAME related stuff is more or less copied from
                // the Go Xray exporter.
                // Seems completely arbitrary and taken out of a hat, but I guess it does not really hurt(???)
                HTTP_EVENT_NAME if self.rpc_system_is_aws_api => {
                    include_exception = true;
                    exception_message_set = true;
                    let http_response_status_code = self
                        .http_response_status_code
                        .map(|code| code.to_string())
                        .unwrap_or_default();
                    let timestamp = translate_timestamp(event.timestamp);
                    let aws_http_error_message = self.aws_http_error_message.unwrap_or_default();
                    if let Some(aws_http_error_event) = self.aws_http_error_event {
                        exception.exception_type(Cow::Borrowed(aws_http_error_event));
                    }
                    exception.remote().message(Cow::Owned(format!(
                        "{http_response_status_code}@{timestamp}@{aws_http_error_message}"
                    )));
                }
                _ => (),
            }

            for KeyValue { key, value, .. } in event.attributes.iter() {
                match key.as_str() {
                    semconv::CODE_FILE_PATH => {
                        stack_frame_builder.path(get_cow(value));
                    }
                    semconv::CODE_LINE_NUMBER => {
                        if let Some(l) = get_integer(value) {
                            stack_frame_builder.line(l as i32);
                        }
                    }
                    semconv::CODE_MODULE_NAME => {
                        stack_frame_builder.label(get_cow(value));
                    }
                    semconv::EXCEPTION_TYPE => {
                        include_exception = true;
                        exception.exception_type(get_cow(value));
                    }
                    semconv::EXCEPTION_MESSAGE => {
                        include_exception = true;
                        exception_message_set = true;
                        exception.message(get_cow(value));
                    }
                    semconv::EXCEPTION_STACKTRACE => {
                        if let Some(stack_trace) = get_str(value) {
                            #[allow(clippy::single_match)]
                            match self.sdk_lang {
                                Some("rust") => {
                                    use regex::Regex;
                                    thread_local! {static RE: Regex = Regex::new(r"\d+:\s+((?<ip>0x[[:xdigit:]]+) - )?(?<label>.+)\n(\s*at (?<path>.+):(?<line>\d+):(?<column>\d+)\n)?").expect("valid regex")}
                                    RE.with(|r| {
                                        for c in r.captures_iter(stack_trace) {
                                            let mut sfb = StackFrameBuilder::default();
                                            sfb.label(
                                                c.name("label")
                                                    .expect("always present")
                                                    .as_str()
                                                    .into(),
                                            );
                                            if let Some(path) = c.name("path") {
                                                sfb.path(path.as_str().into());
                                            }
                                            if let Some(line) = c.name("line") {
                                                sfb.line(
                                                    line.as_str().parse().expect("are digits"),
                                                );
                                            }
                                            exception
                                                .stack_frame(sfb.build())
                                                .expect("StackFrame cannot be empty");
                                        }
                                    })
                                }
                                _ => (),
                            };
                        }
                    }
                    _ => (),
                }
            }

            if include_exception {
                // At this point, we extracted all we could from the event and we wish to include the resulting Exception
                if !exception_message_set {
                    let message = match self.span_status {
                        Status::Error { description } if !description.is_empty() => {
                            Some(description.clone())
                        }
                        _ => None,
                    };

                    let message = message.or_else(|| match self.http_status_text.as_ref() {
                        Some(http_status_text) if !http_status_text.is_empty() => {
                            Some(http_status_text.clone())
                        }
                        _ => None,
                    });

                    if let Some(message) = message {
                        exception.message(message);
                    }
                }

                // We don't care if it fails because the frame is empty
                // Just try to insert what we got, if we got anything
                let _ = exception.stack_frame(stack_frame_builder.build()).ok();
                error_details.exception(exception.build()?);
            }
        }

        Ok(())
    }
}

impl<'v> SpanAttributeProcessor<'v, 7> for CauseBuilder<'v> {
    const HANDLERS: [(&'static str, fn(&mut Self, &'v Value) -> bool); 7] = [
        (semconv::RPC_SYSTEM, Self::rpc_system_is_aws_api),
        (semconv::TELEMETRY_SDK_LANGUAGE, Self::sdk_lang),
        (semconv::HTTP_STATUS_TEXT, Self::http_status_text),
        (
            #[allow(deprecated)]
            semconv::HTTP_STATUS_CODE,
            Self::http_status_code,
        ),
        (
            semconv::HTTP_RESPONSE_STATUS_CODE,
            Self::http_response_status_code,
        ),
        (
            semconv::AWS_HTTP_ERROR_MESSAGE,
            Self::aws_http_error_message,
        ),
        (semconv::AWS_HTTP_ERROR_EVENT, Self::aws_http_error_event),
    ];
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xray_exporter::types::{Id, SubsegmentDocumentBuilder, TraceId};
    use opentelemetry::trace::Status;
    use opentelemetry::{KeyValue, Value};
    use std::time::SystemTime;

    /// Helper: create an Event with the given name and attributes.
    fn make_event(name: &'static str, attrs: Vec<KeyValue>) -> Event {
        Event::new(name, SystemTime::now(), attrs, 0)
    }

    /// Helper: finalize a subsegment builder, build it, and return JSON string.
    fn build_json(builder: AnyDocumentBuilder<'_>) -> String {
        match builder {
            AnyDocumentBuilder::Subsegment(mut b) => {
                b.name("test").unwrap();
                b.id(Id::from(0xABCDu64));
                b.parent_id(Id::from(0x1234u64));
                b.start_time(1_000_000.0);
                b.trace_id(TraceId::new(), true).unwrap();
                b.build().unwrap().to_string()
            }
            AnyDocumentBuilder::Segment(mut b) => {
                b.name("test").unwrap();
                b.id(Id::from(0xABCDu64));
                b.start_time(1_000_000.0);
                b.trace_id(TraceId::new(), true).unwrap();
                b.build().unwrap().to_string()
            }
        }
    }

    /// Helper: parse JSON string into serde_json::Value for field inspection.
    fn parse_json(json: &str) -> serde_json::Value {
        serde_json::from_str(json).unwrap()
    }

    // ---------------------------------------------------------------
    // Test 1: No error, no exceptions → early return, no flags set
    // ---------------------------------------------------------------

    #[test]
    fn test_resolve_no_error_no_exceptions() {
        let events = [];
        let status = Status::Ok;
        let builder = CauseBuilder::new(&events, &status, false);

        let mut doc = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder.resolve(&mut doc).unwrap();

        let json = build_json(doc);
        let parsed = parse_json(&json);
        let obj = parsed.as_object().unwrap();

        // No fault/error/throttle/cause fields should be present
        assert!(!obj.contains_key("fault"), "fault should be absent");
        assert!(!obj.contains_key("error"), "error should be absent");
        assert!(!obj.contains_key("throttle"), "throttle should be absent");
        assert!(!obj.contains_key("cause"), "cause should be absent");
    }

    // ---------------------------------------------------------------
    // Test 2: HTTP 429 → throttle=true, error=true, fault=false
    // ---------------------------------------------------------------

    #[test]
    fn test_resolve_http_429_throttle() {
        let events = [];
        let status = Status::Error {
            description: "".into(),
        };
        let val_429 = Value::I64(429);
        let mut builder = CauseBuilder::new(&events, &status, false);
        builder.http_response_status_code(&val_429);

        let mut doc = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder.resolve(&mut doc).unwrap();

        let json = build_json(doc);
        let parsed = parse_json(&json);

        assert_eq!(parsed["throttle"], true, "throttle should be true for 429");
        assert_eq!(parsed["error"], true, "error should be true for 429");
        assert!(
            !parsed.as_object().unwrap().contains_key("fault") || parsed["fault"] == false,
            "fault should be false/absent for 429"
        );
    }

    // ---------------------------------------------------------------
    // Test 3: HTTP 4xx (not 429) → error=true, fault=false, throttle=false
    // ---------------------------------------------------------------

    #[test]
    fn test_resolve_http_404_error() {
        let events = [];
        let status = Status::Error {
            description: "".into(),
        };
        let val_404 = Value::I64(404);
        let mut builder = CauseBuilder::new(&events, &status, false);
        builder.http_response_status_code(&val_404);

        let mut doc = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder.resolve(&mut doc).unwrap();

        let json = build_json(doc);
        let parsed = parse_json(&json);

        assert_eq!(parsed["error"], true, "error should be true for 404");
        assert!(
            !parsed.as_object().unwrap().contains_key("fault") || parsed["fault"] == false,
            "fault should be false/absent for 404"
        );
        assert!(
            !parsed.as_object().unwrap().contains_key("throttle") || parsed["throttle"] == false,
            "throttle should be false/absent for 404"
        );
    }

    // ---------------------------------------------------------------
    // Test 4: HTTP 5xx → fault=true, error=false
    // ---------------------------------------------------------------

    #[test]
    fn test_resolve_http_500_fault() {
        let events = [];
        let status = Status::Error {
            description: "".into(),
        };
        let val_500 = Value::I64(500);
        let mut builder = CauseBuilder::new(&events, &status, false);
        builder.http_response_status_code(&val_500);

        let mut doc = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder.resolve(&mut doc).unwrap();

        let json = build_json(doc);
        let parsed = parse_json(&json);

        assert_eq!(parsed["fault"], true, "fault should be true for 500");
        assert!(
            !parsed.as_object().unwrap().contains_key("error") || parsed["error"] == false,
            "error should be false/absent for 500"
        );
    }

    // ---------------------------------------------------------------
    // Test 5: span_in_error with no HTTP code → fault=true (default)
    // ---------------------------------------------------------------

    #[test]
    fn test_resolve_span_error_no_http_code_defaults_to_fault() {
        let events = [];
        let status = Status::Error {
            description: "something failed".into(),
        };
        let builder = CauseBuilder::new(&events, &status, false);

        let mut doc = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder.resolve(&mut doc).unwrap();

        let json = build_json(doc);
        let parsed = parse_json(&json);

        assert_eq!(
            parsed["fault"], true,
            "fault should be true when span is in error with no HTTP code"
        );
    }

    // ---------------------------------------------------------------
    // Test 6: Exception event with type and message attributes
    // ---------------------------------------------------------------

    #[test]
    fn test_resolve_exception_event_with_type_and_message() {
        let events = vec![make_event(
            "exception",
            vec![
                KeyValue::new("exception.type", "RuntimeError"),
                KeyValue::new("exception.message", "null pointer"),
            ],
        )];
        let status = Status::Error {
            description: "".into(),
        };
        let builder = CauseBuilder::new(&events, &status, false);

        let mut doc = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder.resolve(&mut doc).unwrap();

        let json = build_json(doc);
        let parsed = parse_json(&json);

        // Should have cause with exceptions
        let cause = &parsed["cause"];
        assert!(cause.is_object(), "cause should be present as object");
        let exceptions = cause["exceptions"].as_array().unwrap();
        assert_eq!(exceptions.len(), 1);
        assert_eq!(exceptions[0]["type"], "RuntimeError");
        assert_eq!(exceptions[0]["message"], "null pointer");
        // remote should be absent (false is skipped)
        assert!(
            !exceptions[0].as_object().unwrap().contains_key("remote")
                || exceptions[0]["remote"] == false,
            "remote should be false/absent for non-remote span"
        );
    }

    // ---------------------------------------------------------------
    // Test 7: Exception event with remote span → remote=true
    // ---------------------------------------------------------------

    #[test]
    fn test_resolve_exception_event_remote_span() {
        let events = vec![make_event(
            "exception",
            vec![KeyValue::new("exception.type", "TimeoutError")],
        )];
        let status = Status::Error {
            description: "timeout".into(),
        };
        // span_is_remote = true
        let builder = CauseBuilder::new(&events, &status, true);

        let mut doc = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder.resolve(&mut doc).unwrap();

        let json = build_json(doc);
        let parsed = parse_json(&json);

        let exceptions = parsed["cause"]["exceptions"].as_array().unwrap();
        assert_eq!(exceptions.len(), 1);
        assert_eq!(
            exceptions[0]["remote"], true,
            "remote should be true for remote span"
        );
    }

    // ---------------------------------------------------------------
    // Test 8: HTTP_EVENT_NAME with rpc.system=aws-api → special message
    // ---------------------------------------------------------------

    #[test]
    fn test_resolve_http_event_with_aws_api_rpc_system() {
        let events = vec![make_event("HTTP request failure", vec![])];
        let status = Status::Error {
            description: "".into(),
        };
        // Bind all Value references to local variables to satisfy lifetimes
        let val_aws_api = Value::String("aws-api".into());
        let val_503 = Value::I64(503);
        let val_err_msg = Value::String("Service Unavailable".into());
        let val_err_event = Value::String("ServiceUnavailableException".into());

        let mut builder = CauseBuilder::new(&events, &status, false);
        builder.rpc_system_is_aws_api(&val_aws_api);
        builder.http_response_status_code(&val_503);
        builder.aws_http_error_message(&val_err_msg);
        builder.aws_http_error_event(&val_err_event);

        let mut doc = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder.resolve(&mut doc).unwrap();

        let json = build_json(doc);
        let parsed = parse_json(&json);

        let exceptions = parsed["cause"]["exceptions"].as_array().unwrap();
        assert_eq!(exceptions.len(), 1);

        // The message format is: "{http_response_status_code}@{timestamp}@{aws_http_error_message}"
        let message = exceptions[0]["message"].as_str().unwrap();
        assert!(
            message.starts_with("503@"),
            "message should start with status code: {message}"
        );
        assert!(
            message.ends_with("@Service Unavailable"),
            "message should end with error message: {message}"
        );

        // exception_type should be the aws_http_error_event
        assert_eq!(exceptions[0]["type"], "ServiceUnavailableException");

        // remote should be true (set explicitly in the HTTP_EVENT_NAME branch)
        assert_eq!(exceptions[0]["remote"], true);
    }

    // ---------------------------------------------------------------
    // Test 9: Exception without message → falls back to span status description
    // ---------------------------------------------------------------

    #[test]
    fn test_resolve_exception_without_message_falls_back_to_status_description() {
        let events = vec![make_event(
            "exception",
            vec![KeyValue::new("exception.type", "IOError")],
        )];
        let status = Status::Error {
            description: "disk full".into(),
        };
        let builder = CauseBuilder::new(&events, &status, false);

        let mut doc = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder.resolve(&mut doc).unwrap();

        let json = build_json(doc);
        let parsed = parse_json(&json);

        let exceptions = parsed["cause"]["exceptions"].as_array().unwrap();
        assert_eq!(exceptions.len(), 1);
        assert_eq!(exceptions[0]["type"], "IOError");
        assert_eq!(
            exceptions[0]["message"], "disk full",
            "should fall back to span status description"
        );
    }

    // ---------------------------------------------------------------
    // Test 10: Exception without message, no status description → falls back to http_status_text
    // ---------------------------------------------------------------

    #[test]
    fn test_resolve_exception_without_message_falls_back_to_http_status_text() {
        let events = vec![make_event(
            "exception",
            vec![KeyValue::new("exception.type", "HttpError")],
        )];
        // Empty description so it won't be used
        let status = Status::Error {
            description: "".into(),
        };
        let val_not_found = Value::String("Not Found".into());
        let mut builder = CauseBuilder::new(&events, &status, false);
        builder.http_status_text(&val_not_found);

        let mut doc = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder.resolve(&mut doc).unwrap();

        let json = build_json(doc);
        let parsed = parse_json(&json);

        let exceptions = parsed["cause"]["exceptions"].as_array().unwrap();
        assert_eq!(exceptions.len(), 1);
        assert_eq!(
            exceptions[0]["message"], "Not Found",
            "should fall back to http_status_text"
        );
    }

    // ---------------------------------------------------------------
    // Test 11: Rust stack trace parsing
    // ---------------------------------------------------------------

    #[test]
    fn test_resolve_rust_stack_trace_parsing() {
        // Simulate a Rust-style backtrace
        let stacktrace = "\
   0: 0x55a1b2c3d4e5 - std::backtrace::Backtrace::create\n\
             at /rustc/abc123/library/std/src/backtrace.rs:300:13\n\
   1: myapp::handler::process_request\n\
             at src/handler.rs:42:5\n\
   2: 0x55a1b2c3d4e6 - core::ops::function::FnOnce::call_once\n";

        let events = vec![make_event(
            "exception",
            vec![
                KeyValue::new("exception.type", "PanicError"),
                KeyValue::new("exception.stacktrace", stacktrace),
            ],
        )];
        let status = Status::Error {
            description: "panicked".into(),
        };
        let val_rust = Value::String("rust".into());
        let mut builder = CauseBuilder::new(&events, &status, false);
        builder.sdk_lang(&val_rust);

        let mut doc = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder.resolve(&mut doc).unwrap();

        let json = build_json(doc);
        let parsed = parse_json(&json);

        let exceptions = parsed["cause"]["exceptions"].as_array().unwrap();
        assert_eq!(exceptions.len(), 1);

        let stack = exceptions[0]["stack"].as_array().unwrap();
        // Frame 0: has IP, label, path, line
        assert_eq!(stack[0]["label"], "std::backtrace::Backtrace::create");
        assert_eq!(
            stack[0]["path"],
            "/rustc/abc123/library/std/src/backtrace.rs"
        );
        assert_eq!(stack[0]["line"], 300);

        // Frame 1: no IP, has label, path, line
        assert_eq!(stack[1]["label"], "myapp::handler::process_request");
        assert_eq!(stack[1]["path"], "src/handler.rs");
        assert_eq!(stack[1]["line"], 42);

        // Frame 2: has IP, label, but no path/line (no "at" line follows)
        assert_eq!(stack[2]["label"], "core::ops::function::FnOnce::call_once");
        assert!(
            !stack[2].as_object().unwrap().contains_key("path"),
            "frame 2 should have no path"
        );
    }

    // ---------------------------------------------------------------
    // Test: Non-rust sdk_lang does NOT parse stacktrace
    // ---------------------------------------------------------------

    #[test]
    fn test_resolve_non_rust_sdk_lang_does_not_parse_stacktrace() {
        let stacktrace = "\
   0: 0x55a1b2c3d4e5 - std::backtrace::Backtrace::create\n\
             at /rustc/abc123/library/std/src/backtrace.rs:300:13\n";

        let events = vec![make_event(
            "exception",
            vec![
                KeyValue::new("exception.type", "Error"),
                KeyValue::new("exception.stacktrace", stacktrace),
            ],
        )];
        let status = Status::Error {
            description: "err".into(),
        };
        let val_python = Value::String("python".into());
        let mut builder = CauseBuilder::new(&events, &status, false);
        builder.sdk_lang(&val_python);

        let mut doc = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder.resolve(&mut doc).unwrap();

        let json = build_json(doc);
        let parsed = parse_json(&json);

        let exceptions = parsed["cause"]["exceptions"].as_array().unwrap();
        assert_eq!(exceptions.len(), 1);
        // Stack should be empty — non-rust sdk_lang doesn't parse stacktrace
        assert!(
            !exceptions[0].as_object().unwrap().contains_key("stack")
                || exceptions[0]["stack"].as_array().unwrap().is_empty(),
            "stack should be empty for non-rust sdk_lang"
        );
    }

    // ---------------------------------------------------------------
    // Test: http_status_code (deprecated) is also recognized
    // ---------------------------------------------------------------

    #[test]
    fn test_resolve_deprecated_http_status_code() {
        let events = [];
        let status = Status::Error {
            description: "".into(),
        };
        let val_502 = Value::I64(502);
        let mut builder = CauseBuilder::new(&events, &status, false);
        builder.http_status_code(&val_502);

        let mut doc = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder.resolve(&mut doc).unwrap();

        let json = build_json(doc);
        let parsed = parse_json(&json);

        assert_eq!(parsed["fault"], true, "502 should set fault=true");
    }

    // ---------------------------------------------------------------
    // Test: http_status_code takes precedence over http_response_status_code
    // ---------------------------------------------------------------

    #[test]
    fn test_resolve_http_status_code_precedence() {
        // The code uses: self.http_status_code.or(self.http_response_status_code)
        // So http_status_code takes precedence.
        let events = [];
        let status = Status::Error {
            description: "".into(),
        };
        let val_429 = Value::I64(429);
        let val_500 = Value::I64(500);
        let mut builder = CauseBuilder::new(&events, &status, false);
        builder.http_status_code(&val_429); // throttle
        builder.http_response_status_code(&val_500); // would be fault

        let mut doc = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder.resolve(&mut doc).unwrap();

        let json = build_json(doc);
        let parsed = parse_json(&json);

        // http_status_code (429) should win → throttle + error
        assert_eq!(
            parsed["throttle"], true,
            "http_status_code should take precedence"
        );
        assert_eq!(parsed["error"], true);
    }

    // ---------------------------------------------------------------
    // Test: Multiple exception events produce multiple exceptions
    // ---------------------------------------------------------------

    #[test]
    fn test_resolve_multiple_exception_events() {
        let events = vec![
            make_event(
                "exception",
                vec![
                    KeyValue::new("exception.type", "FirstError"),
                    KeyValue::new("exception.message", "first"),
                ],
            ),
            make_event(
                "exception",
                vec![
                    KeyValue::new("exception.type", "SecondError"),
                    KeyValue::new("exception.message", "second"),
                ],
            ),
        ];
        let status = Status::Error {
            description: "".into(),
        };
        let builder = CauseBuilder::new(&events, &status, false);

        let mut doc = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder.resolve(&mut doc).unwrap();

        let json = build_json(doc);
        let parsed = parse_json(&json);

        let exceptions = parsed["cause"]["exceptions"].as_array().unwrap();
        assert_eq!(exceptions.len(), 2);
        assert_eq!(exceptions[0]["type"], "FirstError");
        assert_eq!(exceptions[0]["message"], "first");
        assert_eq!(exceptions[1]["type"], "SecondError");
        assert_eq!(exceptions[1]["message"], "second");
    }

    // ---------------------------------------------------------------
    // Test: Exception event detected via attributes alone (not name)
    // triggers include_exception via exception.type/message attributes
    // ---------------------------------------------------------------

    #[test]
    fn test_resolve_exception_detected_by_attributes_not_name() {
        // Event name is NOT "exception", but has exception.type attribute
        // which sets include_exception = true
        let events = vec![make_event(
            "some_other_event",
            vec![KeyValue::new("exception.type", "CustomError")],
        )];
        let status = Status::Error {
            description: "custom".into(),
        };
        let builder = CauseBuilder::new(&events, &status, false);

        let mut doc = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder.resolve(&mut doc).unwrap();

        let json = build_json(doc);
        let parsed = parse_json(&json);

        let exceptions = parsed["cause"]["exceptions"].as_array().unwrap();
        assert_eq!(exceptions.len(), 1);
        assert_eq!(exceptions[0]["type"], "CustomError");
        // Message should fall back to status description since exception.message not set
        assert_eq!(exceptions[0]["message"], "custom");
    }

    // ---------------------------------------------------------------
    // Test: has_exceptions=true but span NOT in error → resolve succeeds,
    // but no error flags set (build would fail with CauseWithoutError)
    // ---------------------------------------------------------------

    #[test]
    fn test_resolve_exception_event_without_span_error() {
        let events = vec![make_event(
            "exception",
            vec![
                KeyValue::new("exception.type", "Warning"),
                KeyValue::new("exception.message", "something odd"),
            ],
        )];
        // Status::Ok — span is NOT in error
        let status = Status::Ok;
        let builder = CauseBuilder::new(&events, &status, false);

        let mut doc = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        // resolve() itself succeeds — it just sets fields on the builder
        assert!(builder.resolve(&mut doc).is_ok());

        // However, building the document will fail because cause is set
        // without any error flag (fault/error/throttle)
        match doc {
            AnyDocumentBuilder::Subsegment(mut b) => {
                b.name("test").unwrap();
                b.id(Id::from(0xABCDu64));
                b.parent_id(Id::from(0x1234u64));
                b.start_time(1_000_000.0);
                b.trace_id(TraceId::new(), true).unwrap();
                assert!(
                    b.build().is_err(),
                    "build should fail with CauseWithoutError"
                );
            }
            _ => panic!("expected Subsegment"),
        }
    }

    // ---------------------------------------------------------------
    // Test: Exception with no message AND empty status description AND empty http_status_text
    // → no message field on exception
    // ---------------------------------------------------------------

    #[test]
    fn test_resolve_exception_no_message_no_fallbacks() {
        let events = vec![make_event(
            "exception",
            vec![KeyValue::new("exception.type", "UnknownError")],
        )];
        let status = Status::Error {
            description: "".into(),
        };
        let val_empty = Value::String("".into());
        let mut builder = CauseBuilder::new(&events, &status, false);
        builder.http_status_text(&val_empty);

        let mut doc = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder.resolve(&mut doc).unwrap();

        let json = build_json(doc);
        let parsed = parse_json(&json);

        let exceptions = parsed["cause"]["exceptions"].as_array().unwrap();
        assert_eq!(exceptions.len(), 1);
        assert_eq!(exceptions[0]["type"], "UnknownError");
        // No message should be set (empty description and empty http_status_text are skipped)
        assert!(
            !exceptions[0].as_object().unwrap().contains_key("message"),
            "message should be absent when all fallbacks are empty"
        );
    }

    // ---------------------------------------------------------------
    // Test: code.file.path, code.line.number, code.module.name on event
    // ---------------------------------------------------------------

    #[test]
    fn test_resolve_exception_with_code_attributes() {
        let events = vec![make_event(
            "exception",
            vec![
                KeyValue::new("exception.type", "CodeError"),
                KeyValue::new("exception.message", "bad code"),
                KeyValue::new("code.file.path", "src/main.rs"),
                KeyValue::new("code.line.number", 42i64),
                KeyValue::new("code.module.name", "myapp::main"),
            ],
        )];
        let status = Status::Error {
            description: "".into(),
        };
        let builder = CauseBuilder::new(&events, &status, false);

        let mut doc = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder.resolve(&mut doc).unwrap();

        let json = build_json(doc);
        let parsed = parse_json(&json);

        let exceptions = parsed["cause"]["exceptions"].as_array().unwrap();
        assert_eq!(exceptions.len(), 1);

        // The code attributes should produce a stack frame
        let stack = exceptions[0]["stack"].as_array().unwrap();
        assert_eq!(stack.len(), 1);
        assert_eq!(stack[0]["path"], "src/main.rs");
        assert_eq!(stack[0]["line"], 42);
        assert_eq!(stack[0]["label"], "myapp::main");
    }

    // ---------------------------------------------------------------
    // Test: HTTP_EVENT_NAME without rpc_system=aws-api is ignored
    // ---------------------------------------------------------------

    #[test]
    fn test_resolve_http_event_without_aws_api_ignored() {
        let events = vec![make_event("HTTP request failure", vec![])];
        let status = Status::Error {
            description: "".into(),
        };
        // rpc_system_is_aws_api is false by default
        let builder = CauseBuilder::new(&events, &status, false);

        let mut doc = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder.resolve(&mut doc).unwrap();

        let json = build_json(doc);
        let parsed = parse_json(&json);

        // The HTTP event should NOT be treated as an exception
        // Only fault should be set (span_in_error with no HTTP code)
        assert_eq!(parsed["fault"], true);
        assert!(
            !parsed.as_object().unwrap().contains_key("cause"),
            "cause should be absent when HTTP event is not recognized"
        );
    }

    // ---------------------------------------------------------------
    // Test: Non-exception, non-HTTP events are skipped
    // ---------------------------------------------------------------

    #[test]
    fn test_resolve_irrelevant_events_skipped() {
        let events = vec![
            make_event("some_log", vec![KeyValue::new("key", "value")]),
            make_event("another_event", vec![]),
        ];
        let status = Status::Error {
            description: "".into(),
        };
        let builder = CauseBuilder::new(&events, &status, false);

        let mut doc = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder.resolve(&mut doc).unwrap();

        let json = build_json(doc);
        let parsed = parse_json(&json);

        // fault should be set (span_in_error, no HTTP code)
        assert_eq!(parsed["fault"], true);
        // No cause since no exception events
        assert!(
            !parsed.as_object().unwrap().contains_key("cause"),
            "cause should be absent for irrelevant events"
        );
    }
}
