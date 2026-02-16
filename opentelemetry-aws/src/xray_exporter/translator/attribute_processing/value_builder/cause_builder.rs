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
