//! Building blocks shared by the HTTP layers.

pub(crate) mod attributes;

use opentelemetry::InstrumentationScope;

/// The instrumentation scope of the server and the client layers.
pub(crate) fn instrumentation_scope() -> InstrumentationScope {
    InstrumentationScope::builder(crate::INSTRUMENTATION_NAME)
        .with_version(env!("CARGO_PKG_VERSION"))
        .with_schema_url(opentelemetry_semantic_conventions::SCHEMA_URL)
        .build()
}
