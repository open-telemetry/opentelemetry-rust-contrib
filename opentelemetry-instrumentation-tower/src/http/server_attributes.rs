//! Server-side resolution of the HTTP semantic convention attributes that
//! describe the client, this server, and the request target.
//!
//! An HTTP server can sit behind one or more reverse proxies, which rewrite the
//! connection and the `Host` header. The conventions therefore prefer the
//! forwarding headers over the values of the immediate connection:
//! <https://opentelemetry.io/docs/specs/semconv/http/http-spans/#setting-serveraddress-and-serverport-attributes>

use std::net::SocketAddr;

use opentelemetry::KeyValue;
use opentelemetry_semantic_conventions as semconv;

use crate::common::attributes::{
    first_forwarded_value, forwarded_directive, split_host_port, url_scheme_kv,
};

/// Value that RFC 7239 reserves for a client the proxy does not disclose.
const FORWARDED_UNKNOWN: &str = "unknown";

/// Attributes of an incoming request that describe the client, this server, and
/// the request target.
pub(crate) struct ServerRequestAttributes {
    /// `url.scheme`, which the server layer also records as a metric attribute.
    pub(crate) url_scheme_kv: KeyValue,
    /// `url.query`, set when the request target carries a query component.
    pub(crate) url_query_kv_opt: Option<KeyValue>,
    /// `client.address` of the original client, behind all proxies.
    pub(crate) client_address_kv_opt: Option<KeyValue>,
    /// `network.peer.address` and `network.peer.port` of the immediate peer,
    /// which is the closest proxy when the request passed through one.
    pub(crate) network_peer_kv_opt: Option<(KeyValue, KeyValue)>,
    /// `server.address` and `server.port` that the client addressed.
    pub(crate) server_address_kv_opt: Option<KeyValue>,
    pub(crate) server_port_kv_opt: Option<KeyValue>,
}

/// Resolves the attributes of an incoming request.
pub(crate) fn server_request_attributes<B>(req: &http::Request<B>) -> ServerRequestAttributes {
    let headers = req.headers();
    let uri = req.uri();
    let peer = peer_socket_addr(req);

    let (server_address, server_port) = match server_authority(req) {
        Some((address, port)) => (
            Some(KeyValue::new(
                semconv::attribute::SERVER_ADDRESS,
                address.to_owned(),
            )),
            port.map(|port| KeyValue::new(semconv::attribute::SERVER_PORT, i64::from(port))),
        ),
        None => (None, None),
    };

    ServerRequestAttributes {
        url_scheme_kv: url_scheme_kv(scheme(headers, uri)),
        url_query_kv_opt: uri
            .query()
            .map(|query| KeyValue::new(semconv::attribute::URL_QUERY, query.to_owned())),
        client_address_kv_opt: client_address(headers, peer)
            .map(|address| KeyValue::new(semconv::attribute::CLIENT_ADDRESS, address)),
        network_peer_kv_opt: peer.map(|peer| {
            (
                KeyValue::new(
                    semconv::attribute::NETWORK_PEER_ADDRESS,
                    peer.ip().to_string(),
                ),
                KeyValue::new(
                    semconv::attribute::NETWORK_PEER_PORT,
                    i64::from(peer.port()),
                ),
            )
        }),
        server_address_kv_opt: server_address,
        server_port_kv_opt: server_port,
    }
}

/// Reads the scheme of the original client request, and falls back to `http`
/// when neither a forwarding header nor the request target carries one.
///
/// A server receives a request target in origin form, which has no scheme, so
/// the fallback applies to most requests. A server that terminates TLS itself
/// therefore reports `http` unless a proxy sends the scheme.
fn scheme<'h>(headers: &'h http::HeaderMap, uri: &'h http::Uri) -> &'h str {
    forwarded_directive(headers, "proto")
        .or_else(|| first_forwarded_value(headers, "x-forwarded-proto"))
        .or_else(|| uri.scheme_str())
        .unwrap_or("http")
}

/// Reads the address of the original client, behind all proxies, and falls back
/// to the address of the immediate peer.
fn client_address(headers: &http::HeaderMap, peer: Option<SocketAddr>) -> Option<String> {
    let forwarded = forwarded_directive(headers, "for")
        .or_else(|| first_forwarded_value(headers, "x-forwarded-for"))
        .filter(|value| !value.eq_ignore_ascii_case(FORWARDED_UNKNOWN))
        // RFC 7239 lets a proxy send an obfuscated identifier instead of an
        // address. Such a value is no address, so it is of no use here.
        .filter(|value| !value.starts_with('_'))
        .and_then(|value| split_host_port(value).map(|(host, _)| host.to_owned()));

    forwarded.or_else(|| peer.map(|peer| peer.ip().to_string()))
}

/// Reads the address and port of the server that the client addressed.
fn server_authority<B>(req: &http::Request<B>) -> Option<(&str, Option<u16>)> {
    let headers = req.headers();

    let authority = forwarded_directive(headers, "host")
        .or_else(|| first_forwarded_value(headers, "x-forwarded-host"))
        // The `:authority` pseudo-header of HTTP/2 and HTTP/3, and the
        // absolute-form request target of HTTP/1.1, both land here.
        .or_else(|| req.uri().authority().map(|authority| authority.as_str()))
        .or_else(|| headers.get(http::header::HOST)?.to_str().ok())?;

    split_host_port(authority)
}

/// Reads the socket address of the immediate peer, which Axum records in the
/// request extensions when the application serves with
/// `Router::into_make_service_with_connect_info`.
#[cfg(feature = "axum")]
fn peer_socket_addr<B>(req: &http::Request<B>) -> Option<SocketAddr> {
    req.extensions()
        .get::<axum::extract::ConnectInfo<SocketAddr>>()
        .map(|connect_info| connect_info.0)
}

/// A Tower service reads the request only, which carries no connection. Without
/// the `axum` feature there is no peer address to report.
#[cfg(not(feature = "axum"))]
fn peer_socket_addr<B>(_req: &http::Request<B>) -> Option<SocketAddr> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a request the way a server receives it: an origin-form target,
    /// and the addressed authority only in the `Host` header.
    fn request(target: &str, headers: &[(&str, &str)]) -> http::Request<()> {
        let mut builder = http::Request::builder().uri(target);
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        builder.body(()).unwrap()
    }

    #[test]
    fn server_request_attributes_follow_the_forwarding_headers() {
        struct TestCase {
            name: &'static str,
            target: &'static str,
            headers: &'static [(&'static str, &'static str)],
            expected: Vec<KeyValue>,
        }

        let test_cases = [
            TestCase {
                name: "host header only",
                target: "/users/456",
                headers: &[("host", "example.com:8443")],
                expected: vec![
                    KeyValue::new(semconv::attribute::URL_SCHEME, "http"),
                    KeyValue::new(semconv::attribute::SERVER_ADDRESS, "example.com"),
                    KeyValue::new(semconv::attribute::SERVER_PORT, 8443),
                ],
            },
            TestCase {
                name: "query component",
                target: "/users/456?fields=name&verbose=true",
                headers: &[("host", "example.com")],
                expected: vec![
                    KeyValue::new(semconv::attribute::URL_SCHEME, "http"),
                    KeyValue::new(semconv::attribute::URL_QUERY, "fields=name&verbose=true"),
                    KeyValue::new(semconv::attribute::SERVER_ADDRESS, "example.com"),
                ],
            },
            TestCase {
                name: "empty query component",
                target: "/users/456?",
                headers: &[("host", "example.com")],
                expected: vec![
                    KeyValue::new(semconv::attribute::URL_SCHEME, "http"),
                    KeyValue::new(semconv::attribute::URL_QUERY, ""),
                    KeyValue::new(semconv::attribute::SERVER_ADDRESS, "example.com"),
                ],
            },
            TestCase {
                name: "rfc 7239 forwarded header",
                target: "/users/456",
                headers: &[
                    ("host", "backend.internal:5000"),
                    (
                        "forwarded",
                        "for=203.0.113.7;host=public.example.com:8443;proto=https",
                    ),
                ],
                expected: vec![
                    KeyValue::new(semconv::attribute::URL_SCHEME, "https"),
                    KeyValue::new(semconv::attribute::CLIENT_ADDRESS, "203.0.113.7"),
                    KeyValue::new(semconv::attribute::SERVER_ADDRESS, "public.example.com"),
                    KeyValue::new(semconv::attribute::SERVER_PORT, 8443),
                ],
            },
            TestCase {
                name: "x-forwarded headers",
                target: "/users/456",
                headers: &[
                    ("host", "backend.internal:5000"),
                    ("x-forwarded-for", "203.0.113.7, 198.51.100.17"),
                    ("x-forwarded-host", "public.example.com"),
                    ("x-forwarded-proto", "https"),
                ],
                expected: vec![
                    KeyValue::new(semconv::attribute::URL_SCHEME, "https"),
                    KeyValue::new(semconv::attribute::CLIENT_ADDRESS, "203.0.113.7"),
                    KeyValue::new(semconv::attribute::SERVER_ADDRESS, "public.example.com"),
                ],
            },
            TestCase {
                name: "forwarded header wins over x-forwarded headers",
                target: "/users/456",
                headers: &[
                    ("host", "backend.internal:5000"),
                    ("forwarded", "for=203.0.113.7;host=first.example.com"),
                    ("x-forwarded-for", "198.51.100.17"),
                    ("x-forwarded-host", "second.example.com"),
                ],
                expected: vec![
                    KeyValue::new(semconv::attribute::URL_SCHEME, "http"),
                    KeyValue::new(semconv::attribute::CLIENT_ADDRESS, "203.0.113.7"),
                    KeyValue::new(semconv::attribute::SERVER_ADDRESS, "first.example.com"),
                ],
            },
            TestCase {
                name: "forwarded client with port and ipv6 literal",
                target: "/users/456",
                headers: &[
                    ("host", "example.com"),
                    ("forwarded", r#"for="[2001:db8::17]:4711""#),
                ],
                expected: vec![
                    KeyValue::new(semconv::attribute::URL_SCHEME, "http"),
                    KeyValue::new(semconv::attribute::CLIENT_ADDRESS, "2001:db8::17"),
                    KeyValue::new(semconv::attribute::SERVER_ADDRESS, "example.com"),
                ],
            },
            TestCase {
                name: "undisclosed forwarded client",
                target: "/users/456",
                headers: &[("host", "example.com"), ("forwarded", "for=unknown")],
                expected: vec![
                    KeyValue::new(semconv::attribute::URL_SCHEME, "http"),
                    KeyValue::new(semconv::attribute::SERVER_ADDRESS, "example.com"),
                ],
            },
            TestCase {
                name: "obfuscated forwarded client",
                target: "/users/456",
                headers: &[("host", "example.com"), ("forwarded", "for=_hidden")],
                expected: vec![
                    KeyValue::new(semconv::attribute::URL_SCHEME, "http"),
                    KeyValue::new(semconv::attribute::SERVER_ADDRESS, "example.com"),
                ],
            },
            TestCase {
                name: "no host information at all",
                target: "/users/456",
                headers: &[],
                expected: vec![KeyValue::new(semconv::attribute::URL_SCHEME, "http")],
            },
        ];

        for test_case in test_cases {
            let req = request(test_case.target, test_case.headers);

            let result = flatten(server_request_attributes(&req));

            assert_eq!(result, test_case.expected, "{}", test_case.name);
        }
    }

    #[test]
    fn server_request_attributes_read_the_absolute_form_target() {
        let req = http::Request::builder()
            .uri("http://example.com:8080/users/456?fields=name")
            .body(())
            .unwrap();

        let result = flatten(server_request_attributes(&req));

        assert_eq!(
            result,
            vec![
                KeyValue::new(semconv::attribute::URL_SCHEME, "http"),
                KeyValue::new(semconv::attribute::URL_QUERY, "fields=name"),
                KeyValue::new(semconv::attribute::SERVER_ADDRESS, "example.com"),
                KeyValue::new(semconv::attribute::SERVER_PORT, 8080),
            ]
        );
    }

    #[cfg(feature = "axum")]
    #[test]
    fn server_request_attributes_read_the_peer_address() {
        struct TestCase {
            name: &'static str,
            headers: &'static [(&'static str, &'static str)],
            expected: Vec<KeyValue>,
        }

        let test_cases = [
            TestCase {
                name: "peer is the client",
                headers: &[("host", "example.com")],
                expected: vec![
                    KeyValue::new(semconv::attribute::URL_SCHEME, "http"),
                    KeyValue::new(semconv::attribute::CLIENT_ADDRESS, "198.51.100.17"),
                    KeyValue::new(semconv::attribute::NETWORK_PEER_ADDRESS, "198.51.100.17"),
                    KeyValue::new(semconv::attribute::NETWORK_PEER_PORT, 41234),
                    KeyValue::new(semconv::attribute::SERVER_ADDRESS, "example.com"),
                ],
            },
            TestCase {
                name: "peer is a proxy",
                headers: &[("host", "example.com"), ("x-forwarded-for", "203.0.113.7")],
                expected: vec![
                    KeyValue::new(semconv::attribute::URL_SCHEME, "http"),
                    KeyValue::new(semconv::attribute::CLIENT_ADDRESS, "203.0.113.7"),
                    KeyValue::new(semconv::attribute::NETWORK_PEER_ADDRESS, "198.51.100.17"),
                    KeyValue::new(semconv::attribute::NETWORK_PEER_PORT, 41234),
                    KeyValue::new(semconv::attribute::SERVER_ADDRESS, "example.com"),
                ],
            },
        ];

        for test_case in test_cases {
            let mut req = request("/users/456", test_case.headers);
            req.extensions_mut()
                .insert(axum::extract::ConnectInfo(SocketAddr::from((
                    [198, 51, 100, 17],
                    41234,
                ))));

            let result = flatten(server_request_attributes(&req));

            assert_eq!(result, test_case.expected, "{}", test_case.name);
        }
    }

    /// Collects the resolved attributes in the order the server layer records
    /// them on a span.
    fn flatten(attributes: ServerRequestAttributes) -> Vec<KeyValue> {
        let ServerRequestAttributes {
            url_scheme_kv,
            url_query_kv_opt,
            client_address_kv_opt,
            network_peer_kv_opt,
            server_address_kv_opt,
            server_port_kv_opt,
        } = attributes;

        let mut flattened = vec![url_scheme_kv];
        flattened.extend(url_query_kv_opt);
        flattened.extend(client_address_kv_opt);
        if let Some((peer_address_kv, peer_port_kv)) = network_peer_kv_opt {
            flattened.push(peer_address_kv);
            flattened.push(peer_port_kv);
        }
        flattened.extend(server_address_kv_opt);
        flattened.extend(server_port_kv_opt);
        flattened
    }
}
