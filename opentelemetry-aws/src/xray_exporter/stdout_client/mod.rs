//! Stdout client for debugging X-Ray segment documents.
//!
//! This module provides [`StdoutClient`], which prints segment documents to stdout
//! in pretty-formatted JSON. This is useful for debugging and development purposes.
//!
//! # Examples
//!
//! ```no_run
//! use opentelemetry_aws::xray_exporter::{XrayExporter, stdout_client::StdoutClient};
//!
//! let exporter = XrayExporter::new(StdoutClient);
//! ```

use std::convert::Infallible;

use crate::xray_exporter::types::SegmentDocument;

use super::SegmentDocumentExporter;

/// A client that prints segment documents to stdout for debugging.
///
/// This client outputs each segment document as pretty-formatted JSON to stdout,
/// making it useful for development, testing, and debugging purposes. It does not
/// actually send data to AWS X-Ray.
///
/// # Examples
///
/// Basic usage:
///
/// ```no_run
/// use opentelemetry_aws::xray_exporter::{XrayExporter, stdout_client::StdoutClient};
/// use opentelemetry_sdk::trace::SdkTracerProvider;
///
/// let exporter = XrayExporter::new(StdoutClient);
///
/// let provider = SdkTracerProvider::builder()
///     .with_simple_exporter(exporter)
///     .build();
/// ```
///
/// The output will be formatted JSON like:
///
/// ```json
/// {
///   "name": "my-service",
///   "id": "0123456789abcdef",
///   "trace_id": "1-5f8a1234-abcdef0123456789abcdef01",
///   "start_time": 1602774000.123,
///   "end_time": 1602774000.456,
///   ...
/// }
/// ```
#[derive(Debug)]
pub struct StdoutClient;

impl SegmentDocumentExporter for StdoutClient {
    type Error = Infallible;

    async fn export_segment_documents(
        &self,
        batch: Vec<SegmentDocument<'_>>,
    ) -> Result<(), Self::Error> {
        for document in batch {
            println!("{}", document.to_string_pretty())
        }
        Ok(())
    }
}
