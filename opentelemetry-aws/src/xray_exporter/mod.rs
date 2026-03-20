//! AWS X-Ray exporter for OpenTelemetry.
//!
//! This module provides functionality to export OpenTelemetry spans to AWS X-Ray,
//! converting them into X-Ray segment documents and transmitting them to the X-Ray service.
//!
//! # Architecture
//!
//! The exporter consists of three main components:
//!
//! - **[`XrayExporter`]**: The main exporter that implements [`SpanExporter`] and coordinates
//!   the translation and export process
//! - **[`SegmentTranslator`]**: Converts OpenTelemetry spans into X-Ray segment documents
//! - **Client implementations**: Handle the actual transmission of segment documents to X-Ray
//!   (e.g., `XrayDaemonClient`, `StdoutClient`)
//!
//! # Usage
//!
//! Basic setup with the X-Ray daemon client:
//!
//! **Note**: This example requires the `xray-daemon-client` feature.
//!
//! ```no_run
//! use opentelemetry_aws::{xray_exporter::{XrayExporter, daemon_client::XrayDaemonClient}, trace::XrayIdGenerator};
//! use opentelemetry_sdk::trace::SdkTracerProvider;
//! use opentelemetry::global;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! // Create a client that sends to the X-Ray daemon on localhost:2000(udp)
//! let client = XrayDaemonClient::default();
//!
//! // Create the exporter
//! let exporter = XrayExporter::new(client);
//!
//! // Use with a tracer provider
//! let provider = SdkTracerProvider::builder()
//!     .with_id_generator(XrayIdGenerator::default())
//!     .with_batch_exporter(exporter)
//!     .build();
//!
//! // Set it as the global provider
//! global::set_tracer_provider(provider);
//! # Ok(())
//! # }
//! ```
//!
//! With custom translator configuration:
//!
//! **Note**: This example requires the `xray-daemon-client` feature.
//!
//! ```no_run
//! use opentelemetry_aws::xray_exporter::{
//!     XrayExporter,
//!     daemon_client::XrayDaemonClient,
//!     SegmentTranslator,
//! };
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let client = XrayDaemonClient::default();
//!
//! // Configure the translator
//! let translator = SegmentTranslator::new()
//!     .with_indexed_attr("service.name".to_string())
//!     .with_indexed_attr("http.method".to_string())
//!     .with_log_group_name("/aws/lambda/my-function".to_string());
//!
//! let exporter = XrayExporter::new(client)
//!     .with_translator(translator);
//! # Ok(())
//! # }
//! ```
//!
//! # Feature Flags
//!
//! This module requires the `xray-exporter` feature, which is enabled by default.
//! Several sub-modules and capabilities are gated behind additional feature flags:
//!
//! | Feature | Description |
//! |---------|-------------|
//! | `xray-exporter` | Enables this module (enabled by default) |
//! | `xray-daemon-client` | Enables the [`daemon_client`] sub-module with [`XrayDaemonClient`] for sending segments to the X-Ray daemon over UDP |
//! | `xray-stdout-client` | Enables the [`stdout_client`] sub-module with [`StdoutClient`] for writing segments to stdout (useful for debugging) |
//! | `subsegment-nesting` | Enables the use of subsegment nesting via [`SegmentTranslator::always_nest_subsegments`] |
//! | `internal-logs` | Enables internal `tracing` instrumentation throughout the module |
//!
//! [`XrayDaemonClient`]: daemon_client::XrayDaemonClient
//! [`StdoutClient`]: stdout_client::StdoutClient
//! [`SpanExporter`]: opentelemetry_sdk::trace::SpanExporter

use core::{error::Error, fmt, future::Future};
use opentelemetry_sdk::{
    error::{OTelSdkError, OTelSdkResult},
    trace::SpanExporter,
};

mod translator;
mod types;
mod utils;

#[cfg(feature = "xray-daemon-client")]
pub mod daemon_client;

#[cfg(feature = "xray-stdout-client")]
pub mod stdout_client;

pub use translator::SegmentTranslator;
pub use types::{Id, SegmentDocument, TraceId};

pub mod error {
    //! Error types for the AWS X-Ray exporter.
    //!
    //! This module exposes the error types that can occur during the translation
    //! and construction of AWS X-Ray segment documents from OpenTelemetry spans.
    //!
    //! Two error types are provided:
    //!
    //! - [`ConstraintError`] — validation errors raised when a segment document
    //!   field violates X-Ray requirements (missing identifiers, invalid names,
    //!   annotation limits, etc.).
    //! - [`TranslationError`] — errors that occur during span-to-segment
    //!   translation. This includes [`TranslationError::MissingSpanId`] for spans
    //!   without a valid span ID, and [`TranslationError::ConstraintError`] which
    //!   wraps a [`ConstraintError`] when a constraint check fails during
    //!   translation.
    //!
    //! # Examples
    //!
    //! Matching on translation errors:
    //!
    //! ```
    //! use opentelemetry_aws::xray_exporter::error::{TranslationError, ConstraintError};
    //!
    //! fn handle_error(err: TranslationError) {
    //!     match err {
    //!         TranslationError::MissingSpanId => {
    //!             eprintln!("span is missing a valid span ID");
    //!         }
    //!         TranslationError::ConstraintError(constraint) => {
    //!             eprintln!("segment constraint violated: {constraint}");
    //!         }
    //!     }
    //! }
    //! ```

    pub use super::translator::error::TranslationError;
    pub use super::types::error::ConstraintError;
}

/// Trait for exporting X-Ray segment documents to a backend.
///
/// This trait abstracts the mechanism for transmitting segment documents to AWS X-Ray,
/// allowing different implementations such as UDP transmission to the X-Ray daemon,
/// stdout for debugging, or custom backends.
///
/// # Examples
///
/// Implementing a custom exporter:
///
/// ```
/// use opentelemetry_aws::xray_exporter::{SegmentDocumentExporter, SegmentDocument};
///
/// #[derive(Debug)]
/// struct CustomExporter;
/// # use core::{error, fmt};
/// # #[derive(Debug)]
/// # struct CustomError;
/// # impl error::Error for CustomError {}
/// # impl fmt::Display for CustomError {
/// #     fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
/// #         write!(f, "Oh no, something bad went down")
/// #     }
/// # }
///
/// impl SegmentDocumentExporter for CustomExporter {
///     type Error = CustomError; // Impl Error
///     async fn export_segment_documents(&self, batch: Vec<SegmentDocument<'_>>) -> Result<(), Self::Error> {
///         for document in batch {
///             // Custom export logic
///             println!("Exporting: {}", document.to_string());
///         }
///         Ok(())
///     }
/// }
/// ```
pub trait SegmentDocumentExporter {
    type Error: Error;
    /// Exports a batch of segment documents.
    ///
    /// # Errors
    ///
    /// Returns an error string if the export operation fails. The specific error
    /// conditions depend on the implementation (e.g., network errors, serialization
    /// failures, etc.).
    fn export_segment_documents(
        &self,
        batch: Vec<SegmentDocument<'_>>,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;
}

/// AWS X-Ray exporter for OpenTelemetry spans.
///
/// This exporter converts OpenTelemetry spans into AWS X-Ray segment documents
/// and transmits them using the provided client implementation. It implements
/// the [`SpanExporter`] trait from the OpenTelemetry SDK.
///
/// The exporter uses a [`SegmentTranslator`] to perform the conversion from
/// OpenTelemetry's data model to X-Ray's segment format, handling various
/// AWS-specific metadata and maintaining compatibility with X-Ray's requirements.
///
/// # Type Parameters
///
/// * `Client` - The client implementation used to transmit segment documents.
///   Must implement [`SegmentDocumentExporter`].
///
/// # Examples
///
/// Basic usage with the X-Ray daemon client:
///
/// ```no_run
/// use opentelemetry_aws::xray_exporter::{XrayExporter, daemon_client::XrayDaemonClient};
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// // Default XRay daemon client export to localhost:2000(udp)
/// let exporter = XrayExporter::new(XrayDaemonClient::default());
/// # Ok(())
/// # }
/// ```
///
/// With customized translator:
///
/// ```no_run
/// use opentelemetry_aws::xray_exporter::{
///     XrayExporter,
///     daemon_client::XrayDaemonClient,
///     SegmentTranslator,
/// };
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let client = XrayDaemonClient::default();
///
/// let translator = SegmentTranslator::new()
///     .index_all_attrs();
///
/// let exporter = XrayExporter::new(client)
///     .with_translator(translator);
/// # Ok(())
/// # }
/// ```
///
/// [`SpanExporter`]: opentelemetry_sdk::trace::SpanExporter
#[derive(Debug)]
pub struct XrayExporter<Client: SegmentDocumentExporter + Send + Sync + fmt::Debug> {
    client: Client,
    translator: SegmentTranslator,
}

impl<Client> XrayExporter<Client>
where
    Client: SegmentDocumentExporter + Send + Sync + fmt::Debug,
{
    /// Creates a new X-Ray exporter with the given client.
    ///
    /// The exporter is initialized with a default [`SegmentTranslator`]. Use
    /// [`with_translator`] to set a customized translation behavior.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use opentelemetry_aws::xray_exporter::{XrayExporter, daemon_client::XrayDaemonClient};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// // Default XRay daemon client export to localhost:2000(udp)
    /// let exporter = XrayExporter::new(XrayDaemonClient::default());
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// [`with_translator`]: XrayExporter::with_translator
    pub fn new(client: Client) -> Self {
        Self {
            client,
            translator: SegmentTranslator::default(),
        }
    }

    /// Sets a custom translator for this exporter.
    ///
    /// Configure how OpenTelemetry spans are translated into X-Ray segment documents,
    /// including which attributes to index, log group names, and other translation options.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use opentelemetry_aws::xray_exporter::{
    ///     XrayExporter,
    ///     daemon_client::XrayDaemonClient,
    ///     SegmentTranslator,
    /// };
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let translator = SegmentTranslator::new()
    ///     .with_indexed_attr("service.name".to_string())
    ///     .with_log_group_name("/aws/lambda/my-function".to_string());
    ///
    /// // Default XRay daemon client exports to localhost:2000(udp)
    /// let exporter = XrayExporter::new(XrayDaemonClient::default())
    ///     .with_translator(translator);
    /// # Ok(())
    /// # }
    /// ```
    pub fn with_translator(mut self, translator: SegmentTranslator) -> Self {
        self.translator = translator;
        self
    }
}

impl<Client: SegmentDocumentExporter + Send + Sync + fmt::Debug> SpanExporter
    for XrayExporter<Client>
{
    async fn export(&self, batch: Vec<opentelemetry_sdk::trace::SpanData>) -> OTelSdkResult {
        let doc_batch = self.translator.translate_spans(&batch);
        if doc_batch.len() < batch.len() {
            if doc_batch.is_empty() {
                return Err(OTelSdkError::InternalFailure(
                    "All spans in batch failed translation".to_string(),
                ));
            } else {
                #[cfg(feature = "internal-logs")]
                tracing::warn!(
                    message = "Some spans failed translation",
                    dropped = batch.len() - doc_batch.len()
                );
            }
        }

        self.client
            .export_segment_documents(doc_batch)
            .await
            .map_err(|e| OTelSdkError::InternalFailure(e.to_string()))?;
        Ok(())
    }

    fn set_resource(&mut self, resource: &opentelemetry_sdk::Resource) {
        self.translator.set_resource(resource);
    }
}
