//! Context propagation over HTTP headers.
//!
//! Both directions use the process-wide text-map propagator configured through
//! [`opentelemetry::global::set_text_map_propagator`], so the behavior is
//! independent of whether tracing produces real or no-op spans.

use opentelemetry::global;
use opentelemetry::Context as OtelContext;
#[cfg(feature = "http-server")]
use opentelemetry_http::HeaderExtractor;
#[cfg(feature = "http-client")]
use opentelemetry_http::HeaderInjector;

/// Extracts a parent context from incoming request headers (server side).
#[cfg(feature = "http-server")]
pub(crate) fn extract(headers: &http::HeaderMap) -> OtelContext {
    global::get_text_map_propagator(|propagator| propagator.extract(&HeaderExtractor(headers)))
}

/// Injects the given context into outgoing request headers (client side).
#[cfg(feature = "http-client")]
pub(crate) fn inject(cx: &OtelContext, headers: &mut http::HeaderMap) {
    global::get_text_map_propagator(|propagator| {
        propagator.inject_context(cx, &mut HeaderInjector(headers))
    });
}
