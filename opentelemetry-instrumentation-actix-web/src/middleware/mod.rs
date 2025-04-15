use opentelemetry::InstrumentationScope;

#[cfg(feature = "metrics")]
#[cfg_attr(docsrs, doc(cfg(feature = "metrics")))]
pub(crate) mod metrics;
pub(crate) mod route_formatter;
pub(crate) mod trace;

pub(crate) fn get_scope() -> InstrumentationScope {
    InstrumentationScope::builder("opentelemetry-instrumentation-actix-web")
        .with_version(env!("CARGO_PKG_VERSION"))
        .with_schema_url(opentelemetry_semantic_conventions::SCHEMA_URL)
        .build()
}
