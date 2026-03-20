//! AWS X-Ray segment document data structures.
//!
//! This module provides the [`SegmentDocument`] type, which represents an X-Ray segment
//! or subsegment document ready for serialization and transmission to the X-Ray service.
//!
//! Segment documents are created by the `SegmentTranslator` when converting OpenTelemetry
//! spans to X-Ray format. The structure conforms to the official
//! [X-Ray segment document specification](https://docs.aws.amazon.com/xray/latest/devguide/xray-api-segmentdocuments.html#api-segmentdocuments-fields)
//! and [schema](https://docs.aws.amazon.com/xray/latest/devguide/samples/xray-segmentdocument-schema-v1.0.0.zip).

mod aws;
mod cause;
pub(crate) mod error;
mod http;
mod id;
mod segment_document;
mod service;
mod sql;
mod utils;
mod value;

pub(crate) use cause::{ExceptionBuilder, StackFrameBuilder};
pub(crate) use segment_document::{
    builder_type::DocumentBuilderType, DocumentBuilder, SegmentDocumentBuilder,
    SubsegmentDocumentBuilder,
};

pub use id::{Id, TraceId};

#[cfg(feature = "subsegment-nesting")]
pub(crate) use segment_document::DocumentBuilderHeader;

pub(crate) use value::{AnnotationValue, AnySlice, AnyValue, Namespace, Origin, StrList};

pub use segment_document::SegmentDocument;
