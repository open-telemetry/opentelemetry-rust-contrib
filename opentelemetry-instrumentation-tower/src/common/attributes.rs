//! Common HTTP attribute helpers shared by the server and client layers.

use opentelemetry::KeyValue;
use opentelemetry_semantic_conventions as semconv;

/// Maps common HTTP methods to a `&'static str` so the resulting `KeyValue`
/// stores the method as a static string (no heap allocation, allocation-free
/// `KeyValue::clone()`). Returns `None` for custom/extension methods, which
/// fall back to an owned `String`.
#[inline]
pub(crate) fn method_as_static(m: &http::Method) -> Option<&'static str> {
    match *m {
        http::Method::GET => Some("GET"),
        http::Method::POST => Some("POST"),
        http::Method::PUT => Some("PUT"),
        http::Method::DELETE => Some("DELETE"),
        http::Method::HEAD => Some("HEAD"),
        http::Method::OPTIONS => Some("OPTIONS"),
        http::Method::PATCH => Some("PATCH"),
        http::Method::CONNECT => Some("CONNECT"),
        http::Method::TRACE => Some("TRACE"),
        _ => None,
    }
}

/// Builds the `http.request.method` [`KeyValue`], promoting well-known methods
/// to a `&'static str` for an allocation-free clone in the hot path.
#[inline]
pub(crate) fn method_kv(method: &http::Method) -> KeyValue {
    match method_as_static(method) {
        Some(s) => KeyValue::new(semconv::attribute::HTTP_REQUEST_METHOD, s),
        None => KeyValue::new(
            semconv::attribute::HTTP_REQUEST_METHOD,
            method.as_str().to_owned(),
        ),
    }
}

/// Builds the `url.scheme` [`KeyValue`], promoting the common `http`/`https`
/// schemes to a `&'static str`.
#[inline]
pub(crate) fn url_scheme_kv(uri: &http::Uri) -> KeyValue {
    match uri.scheme_str() {
        Some("http") => KeyValue::new(semconv::attribute::URL_SCHEME, "http"),
        Some("https") => KeyValue::new(semconv::attribute::URL_SCHEME, "https"),
        Some(other) => KeyValue::new(semconv::attribute::URL_SCHEME, other.to_owned()),
        None => KeyValue::new(semconv::attribute::URL_SCHEME, ""),
    }
}

/// Splits an HTTP version into its `network.protocol.name` and
/// `network.protocol.version` values.
#[inline]
pub(crate) fn split_and_format_protocol_version(
    http_version: http::Version,
) -> (&'static str, &'static str) {
    let version_str = match http_version {
        http::Version::HTTP_09 => "0.9",
        http::Version::HTTP_10 => "1.0",
        http::Version::HTTP_11 => "1.1",
        http::Version::HTTP_2 => "2.0",
        http::Version::HTTP_3 => "3.0",
        _ => "",
    };
    ("http", version_str)
}
