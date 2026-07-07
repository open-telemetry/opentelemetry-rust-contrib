//! Common HTTP attribute constants and helpers shared by the server and client
//! layers.

use opentelemetry::KeyValue;
use opentelemetry_semantic_conventions as semconv;

pub(crate) const NETWORK_PROTOCOL_NAME_LABEL: &str = semconv::attribute::NETWORK_PROTOCOL_NAME;
pub(crate) const NETWORK_PROTOCOL_VERSION_LABEL: &str =
    semconv::attribute::NETWORK_PROTOCOL_VERSION;
pub(crate) const URL_SCHEME_LABEL: &str = semconv::attribute::URL_SCHEME;
pub(crate) const HTTP_REQUEST_METHOD_LABEL: &str = semconv::attribute::HTTP_REQUEST_METHOD;
pub(crate) const HTTP_ROUTE_LABEL: &str = semconv::attribute::HTTP_ROUTE;
pub(crate) const HTTP_RESPONSE_STATUS_CODE_LABEL: &str =
    semconv::attribute::HTTP_RESPONSE_STATUS_CODE;
#[cfg(feature = "http-client")]
pub(crate) const SERVER_ADDRESS_LABEL: &str = semconv::attribute::SERVER_ADDRESS;
#[cfg(feature = "http-client")]
pub(crate) const SERVER_PORT_LABEL: &str = semconv::attribute::SERVER_PORT;

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
pub(crate) fn method_kv(method: &http::Method) -> KeyValue {
    match method_as_static(method) {
        Some(s) => KeyValue::new(HTTP_REQUEST_METHOD_LABEL, s),
        None => KeyValue::new(HTTP_REQUEST_METHOD_LABEL, method.as_str().to_owned()),
    }
}

/// Builds the `url.scheme` [`KeyValue`], promoting the common `http`/`https`
/// schemes to a `&'static str`.
pub(crate) fn url_scheme_kv(uri: &http::Uri) -> KeyValue {
    match uri.scheme_str() {
        Some("http") => KeyValue::new(URL_SCHEME_LABEL, "http"),
        Some("https") => KeyValue::new(URL_SCHEME_LABEL, "https"),
        Some(other) => KeyValue::new(URL_SCHEME_LABEL, other.to_owned()),
        None => KeyValue::new(URL_SCHEME_LABEL, ""),
    }
}

/// Splits an HTTP version into its `network.protocol.name` and
/// `network.protocol.version` values.
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
