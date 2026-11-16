//! Pluggable extractors for the HTTP server layer.
//!
//! - [`RouteExtractor`] decides how the `http.route` attribute (and span name) is
//!   produced.
//! - [`RequestAttributeExtractor`] / [`ResponseAttributeExtractor`] let you attach
//!   additional attributes to spans and metrics.
//! - [`ClientAddressExtractor`] decides where the `client.address` and
//!   `client.port` attributes come from.

use http::header::{HeaderName, FORWARDED};
use opentelemetry::KeyValue;

#[cfg(feature = "axum-connect-info")]
use axum::extract::connect_info::ConnectInfo;
#[cfg(feature = "axum")]
use axum::extract::MatchedPath;
#[cfg(feature = "axum-connect-info")]
use std::net::SocketAddr;

use crate::common::attributes::forwarded_client_node;

/// Header name for the de-facto standard forwarded-client header. The `http`
/// crate has no constant for it because it is not a standard header.
const X_FORWARDED_FOR: HeaderName = HeaderName::from_static("x-forwarded-for");

/// Trait for extracting the route/target from HTTP requests.
///
/// Implementations return an optional route string. When present, this is used for:
/// - Span names: `"{method} {route}"` (e.g., `"GET /users/:id"`)
/// - The `http.route` metric attribute
///
/// When `None` is returned, span names use method-only (e.g., `"GET"`) and no
/// `http.route` attribute is set.
///
/// # Cardinality Considerations
///
/// The route should be a low-cardinality value (e.g., a route template like `/users/:id`)
/// rather than the actual path (e.g., `/users/123`). High-cardinality routes can overwhelm
/// the OpenTelemetry SDK's built-in cardinality limits and downstream backends.
pub trait RouteExtractor<B>: Clone + Send + Sync + 'static {
    /// Extracts the route from the request, if available.
    ///
    /// Returns `None` to use method-only span names and skip the `http.route` attribute.
    fn extract_route(&self, req: &http::Request<B>) -> Option<String>;
}

/// Route extractor that returns no route (method-only span names).
///
/// This is the safest option as it avoids cardinality explosion from dynamic
/// path segments. Span names will be just the HTTP method (e.g., `"GET"`).
///
/// The `http.route` attribute will not be set when using this extractor.
#[derive(Clone, Default)]
pub struct NoRouteExtractor;

impl<B> RouteExtractor<B> for NoRouteExtractor {
    fn extract_route(&self, _req: &http::Request<B>) -> Option<String> {
        None
    }
}

/// Route extractor that uses Axum's `MatchedPath` for low-cardinality routes.
///
/// This extractor uses the route template (e.g., `/users/:id`) instead of the actual
/// path (e.g., `/users/123`), providing low-cardinality span names and route attributes
/// that are safe for production use.
///
/// # When `MatchedPath` is unavailable
///
/// Returns `None` (falling back to method-only span names) when `MatchedPath` is not
/// present in the request extensions. This can happen when:
///
/// - The OpenTelemetry layer is placed *before* Axum's router in the middleware stack.
///   The layer must be placed *after* the router to access route information.
/// - The request does not match any defined route (404 responses).
/// - Using Axum's `fallback` handler, which does not set `MatchedPath`.
///
/// For correct route extraction, ensure the middleware order is:
///
/// ```ignore
/// let app = Router::new()
///     .route("/users/:id", get(handler))
///     .layer(otel_layer);  // Layer applied after routes
/// ```
///
/// See the [Axum documentation on middleware ordering](https://docs.rs/axum/latest/axum/middleware/index.html#ordering)
/// for more details.
#[cfg(feature = "axum")]
#[derive(Clone, Default)]
pub struct AxumMatchedPathExtractor;

#[cfg(feature = "axum")]
impl<B> RouteExtractor<B> for AxumMatchedPathExtractor {
    fn extract_route(&self, req: &http::Request<B>) -> Option<String> {
        req.extensions()
            .get::<MatchedPath>()
            .map(|matched_path| matched_path.as_str().to_owned())
    }
}

/// Route extractor that uses the URL path (without query parameters).
///
/// # Warning: Cardinality
///
/// Using this extractor can cause **high cardinality** issues if your routes contain
/// dynamic path segments (e.g., `/users/{id}`, `/orders/{order_id}/items/{item_id}`).
/// Each unique path will create a unique span name and route attribute, potentially
/// overwhelming your tracing and metrics backends with millions of unique series.
///
/// **Only use this if**:
/// - Your routes are static (no path parameters)
/// - You understand and accept the cardinality implications
///
/// Consider using a custom [`FnRouteExtractor`] with path normalization instead.
#[derive(Clone, Default)]
pub struct PathExtractor;

impl<B> RouteExtractor<B> for PathExtractor {
    fn extract_route(&self, req: &http::Request<B>) -> Option<String> {
        Some(req.uri().path().to_owned())
    }
}

/// A function-based route extractor.
#[derive(Clone)]
pub struct FnRouteExtractor<F> {
    extractor: F,
}

impl<F> FnRouteExtractor<F> {
    pub fn new(extractor: F) -> Self {
        Self { extractor }
    }
}

impl<F, B> RouteExtractor<B> for FnRouteExtractor<F>
where
    F: Fn(&http::Request<B>) -> Option<String> + Clone + Send + Sync + 'static,
{
    fn extract_route(&self, req: &http::Request<B>) -> Option<String> {
        (self.extractor)(req)
    }
}

#[cfg(feature = "axum")]
pub(crate) type DefaultRouteExtractor = AxumMatchedPathExtractor;

#[cfg(not(feature = "axum"))]
pub(crate) type DefaultRouteExtractor = NoRouteExtractor;

/// Trait for extracting custom attributes from HTTP requests.
pub trait RequestAttributeExtractor<B>: Clone + Send + Sync + 'static {
    fn extract_attributes(&self, req: &http::Request<B>) -> Vec<KeyValue>;
}

/// Trait for extracting custom attributes from HTTP responses.
pub trait ResponseAttributeExtractor<B>: Clone + Send + Sync + 'static {
    fn extract_attributes(&self, res: &http::Response<B>) -> Vec<KeyValue>;
}

/// Default implementation that extracts no attributes.
#[derive(Clone)]
pub struct NoOpExtractor;

impl<B> RequestAttributeExtractor<B> for NoOpExtractor {
    fn extract_attributes(&self, _req: &http::Request<B>) -> Vec<KeyValue> {
        vec![]
    }
}

impl<B> ResponseAttributeExtractor<B> for NoOpExtractor {
    fn extract_attributes(&self, _res: &http::Response<B>) -> Vec<KeyValue> {
        vec![]
    }
}

/// A function-based request attribute extractor.
#[derive(Clone)]
pub struct FnRequestExtractor<F> {
    extractor: F,
}

impl<F> FnRequestExtractor<F> {
    pub fn new(extractor: F) -> Self {
        Self { extractor }
    }
}

impl<F, B> RequestAttributeExtractor<B> for FnRequestExtractor<F>
where
    F: Fn(&http::Request<B>) -> Vec<KeyValue> + Clone + Send + Sync + 'static,
{
    fn extract_attributes(&self, req: &http::Request<B>) -> Vec<KeyValue> {
        (self.extractor)(req)
    }
}

/// A function-based response attribute extractor.
#[derive(Clone)]
pub struct FnResponseExtractor<F> {
    extractor: F,
}

impl<F> FnResponseExtractor<F> {
    pub fn new(extractor: F) -> Self {
        Self { extractor }
    }
}

impl<F, B> ResponseAttributeExtractor<B> for FnResponseExtractor<F>
where
    F: Fn(&http::Response<B>) -> Vec<KeyValue> + Clone + Send + Sync + 'static,
{
    fn extract_attributes(&self, res: &http::Response<B>) -> Vec<KeyValue> {
        (self.extractor)(res)
    }
}

/// The client of a request, as the `client.address` and `client.port`
/// attributes report it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClientAddress {
    /// The domain name, IP address or Unix domain socket name of the client.
    pub address: String,
    /// The port of the client in `address`. The layer reports it only when
    /// `LayerBuilder::with_client_port` enables it.
    pub port: Option<u16>,
}

/// Trait for finding the client of a request.
///
/// The conventions ask for the original client behind all proxies, if it is
/// known, and for the immediate peer otherwise. Only the application knows
/// which proxies it can trust, so the application picks the extractor:
///
/// - [`SocketAddressExtractor`] is the default. It reports the immediate peer,
///   which a client cannot spoof.
/// - [`ForwardedHeadersExtractor`] reads the `X-Forwarded-For` and `Forwarded`
///   headers. Use it only when a trusted proxy sets or strips those headers.
/// - [`FnClientAddressExtractor`] runs your own function, for example one that
///   applies your trusted-proxy policy.
///
/// When the extractor returns `None`, the span carries no `client.address`.
pub trait ClientAddressExtractor<B>: Clone + Send + Sync + 'static {
    /// Finds the client of the request, if one is known.
    fn extract_client_address(&self, req: &http::Request<B>) -> Option<ClientAddress>;
}

/// Client address extractor that reports the socket address of the immediate
/// peer.
///
/// A Tower `Service` does not see the socket, so the service in front must put
/// the peer address in the request extensions. With Axum, build the router with
/// `Router::into_make_service_with_connect_info::<SocketAddr>()` and enable the
/// `axum-connect-info` feature of this crate. Without the feature, or without a
/// socket address in the request extensions, this extractor returns `None`.
#[derive(Clone, Default)]
pub struct SocketAddressExtractor;

impl<B> ClientAddressExtractor<B> for SocketAddressExtractor {
    fn extract_client_address(&self, req: &http::Request<B>) -> Option<ClientAddress> {
        socket_address(req)
    }
}

#[cfg(feature = "axum-connect-info")]
fn socket_address<B>(req: &http::Request<B>) -> Option<ClientAddress> {
    req.extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|info| ClientAddress {
            address: info.0.ip().to_string(),
            port: Some(info.0.port()),
        })
}

#[cfg(not(feature = "axum-connect-info"))]
fn socket_address<B>(_req: &http::Request<B>) -> Option<ClientAddress> {
    None
}

/// Client address extractor that reads the forwarded headers, and falls back
/// to the socket address of the immediate peer.
///
/// The extractor reports the first client it finds in this order:
///
/// - The first entry of the `X-Forwarded-For` header. A chain of proxies
///   appends one address per hop, so the first entry is the original client.
/// - The node name of the first `for` directive in the `Forwarded` header.
/// - The socket address of the immediate peer, as [`SocketAddressExtractor`]
///   reports it.
///
/// # Warning: Untrusted input
///
/// A client can send its own `X-Forwarded-For` and `Forwarded` values, and this
/// extractor cannot tell a proxy-added entry from a spoofed one. Use it only
/// when a proxy in front of your service sets or strips these headers.
/// Otherwise, use [`FnClientAddressExtractor`] with your own trusted-proxy
/// policy.
#[derive(Clone, Default)]
pub struct ForwardedHeadersExtractor;

impl<B> ClientAddressExtractor<B> for ForwardedHeadersExtractor {
    fn extract_client_address(&self, req: &http::Request<B>) -> Option<ClientAddress> {
        let forwarded_for = req
            .headers()
            .get(&X_FORWARDED_FOR)
            .and_then(|value| value.to_str().ok());
        let forwarded = req
            .headers()
            .get(&FORWARDED)
            .and_then(|value| value.to_str().ok());
        forwarded_client_node(forwarded_for, forwarded)
            .map(|(address, port)| ClientAddress {
                address: address.to_owned(),
                port,
            })
            .or_else(|| socket_address(req))
    }
}

/// A function-based client address extractor.
#[derive(Clone)]
pub struct FnClientAddressExtractor<F> {
    extractor: F,
}

impl<F> FnClientAddressExtractor<F> {
    pub fn new(extractor: F) -> Self {
        Self { extractor }
    }
}

impl<F, B> ClientAddressExtractor<B> for FnClientAddressExtractor<F>
where
    F: Fn(&http::Request<B>) -> Option<ClientAddress> + Clone + Send + Sync + 'static,
{
    fn extract_client_address(&self, req: &http::Request<B>) -> Option<ClientAddress> {
        (self.extractor)(req)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_address_extractors_pick_their_source() {
        struct Header {
            name: &'static str,
            value: &'static str,
        }

        struct TestCase {
            name: &'static str,
            extract: fn(&http::Request<()>) -> Option<ClientAddress>,
            headers: &'static [Header],
            expected: Option<ClientAddress>,
        }

        const X_FORWARDED_FOR_HEADER: Header = Header {
            name: "X-Forwarded-For",
            value: "203.0.113.195:4711, 70.41.3.18",
        };

        let test_cases = [
            TestCase {
                name: "the socket address extractor ignores forwarded headers",
                extract: |req| SocketAddressExtractor.extract_client_address(req),
                headers: &[X_FORWARDED_FOR_HEADER],
                expected: None,
            },
            TestCase {
                name: "the forwarded headers extractor reads X-Forwarded-For",
                extract: |req| ForwardedHeadersExtractor.extract_client_address(req),
                headers: &[X_FORWARDED_FOR_HEADER],
                expected: Some(ClientAddress {
                    address: String::from("203.0.113.195"),
                    port: Some(4711),
                }),
            },
            TestCase {
                name: "the forwarded headers extractor reads Forwarded",
                extract: |req| ForwardedHeadersExtractor.extract_client_address(req),
                headers: &[Header {
                    name: "Forwarded",
                    value: "for=192.0.2.43;by=203.0.113.60",
                }],
                expected: Some(ClientAddress {
                    address: String::from("192.0.2.43"),
                    port: None,
                }),
            },
            TestCase {
                name: "X-Forwarded-For wins over Forwarded",
                extract: |req| ForwardedHeadersExtractor.extract_client_address(req),
                headers: &[
                    Header {
                        name: "Forwarded",
                        value: "for=192.0.2.43",
                    },
                    X_FORWARDED_FOR_HEADER,
                ],
                expected: Some(ClientAddress {
                    address: String::from("203.0.113.195"),
                    port: Some(4711),
                }),
            },
            TestCase {
                name: "the forwarded headers extractor without headers or socket",
                extract: |req| ForwardedHeadersExtractor.extract_client_address(req),
                headers: &[],
                expected: None,
            },
            TestCase {
                name: "a function extractor returns what the function returns",
                extract: |req| {
                    FnClientAddressExtractor::new(|req: &http::Request<()>| {
                        req.headers()
                            .get("X-Real-IP")
                            .and_then(|value| value.to_str().ok())
                            .map(|address| ClientAddress {
                                address: address.to_owned(),
                                port: None,
                            })
                    })
                    .extract_client_address(req)
                },
                headers: &[
                    X_FORWARDED_FOR_HEADER,
                    Header {
                        name: "X-Real-IP",
                        value: "198.51.100.17",
                    },
                ],
                expected: Some(ClientAddress {
                    address: String::from("198.51.100.17"),
                    port: None,
                }),
            },
        ];

        for test_case in test_cases {
            let mut request = http::Request::builder();
            for header in test_case.headers {
                request = request.header(header.name, header.value);
            }
            let request = request.body(()).unwrap();

            let result = (test_case.extract)(&request);

            assert_eq!(result, test_case.expected, "{}", test_case.name);
        }
    }
}
