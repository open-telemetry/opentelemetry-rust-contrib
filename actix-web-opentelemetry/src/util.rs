use actix_http::header::{self, CONTENT_LENGTH};
use actix_web::{
    dev::ServiceRequest,
    http::{Method, Version},
};
use opentelemetry::{KeyValue, Value};
use opentelemetry_semantic_conventions::{
    attribute::MESSAGING_MESSAGE_BODY_SIZE,
    trace::{
        CLIENT_ADDRESS, HTTP_REQUEST_METHOD, HTTP_ROUTE, NETWORK_PEER_ADDRESS,
        NETWORK_PROTOCOL_VERSION, SERVER_ADDRESS, SERVER_PORT, URL_PATH, URL_QUERY, URL_SCHEME,
        USER_AGENT_ORIGINAL,
    },
};

#[cfg(feature = "awc")]
#[inline]
pub(super) fn http_url(uri: &actix_web::http::Uri) -> String {
    let scheme = uri.scheme().map(|s| s.as_str()).unwrap_or_default();
    let host = uri.host().unwrap_or_default();
    let path = uri.path();
    let port = uri.port_u16().filter(|&p| p != 80 && p != 443);
    let (query, query_delimiter) = if let Some(query) = uri.query() {
        (query, "?")
    } else {
        ("", "")
    };

    if let Some(port) = port {
        format!("{scheme}://{host}:{port}{path}{query_delimiter}{query}")
    } else {
        format!("{scheme}://{host}{path}{query_delimiter}{query}")
    }
}

#[inline]
pub(super) fn http_method_str(method: &Method) -> Value {
    match method {
        &Method::OPTIONS => "OPTIONS".into(),
        &Method::GET => "GET".into(),
        &Method::POST => "POST".into(),
        &Method::PUT => "PUT".into(),
        &Method::DELETE => "DELETE".into(),
        &Method::HEAD => "HEAD".into(),
        &Method::TRACE => "TRACE".into(),
        &Method::CONNECT => "CONNECT".into(),
        &Method::PATCH => "PATCH".into(),
        other => other.to_string().into(),
    }
}

#[inline]
pub(super) fn protocol_version(version: Version) -> Value {
    match version {
        Version::HTTP_09 => "0.9".into(),
        Version::HTTP_10 => "1.0".into(),
        Version::HTTP_11 => "1.1".into(),
        Version::HTTP_2 => "2".into(),
        Version::HTTP_3 => "3".into(),
        other => format!("{:?}", other).into(),
    }
}

#[inline]
pub(super) fn url_scheme(scheme: &str) -> Value {
    match scheme {
        "http" => "http".into(),
        "https" => "https".into(),
        other => other.to_string().into(),
    }
}

pub(super) fn trace_attributes_from_request(
    req: &ServiceRequest,
    http_route: &str,
) -> Vec<KeyValue> {
    let conn_info = req.connection_info();
    let remote_addr = conn_info.realip_remote_addr();

    let mut attributes = Vec::with_capacity(14);

    // Server attrs
    // <https://github.com/open-telemetry/semantic-conventions/blob/v1.21.0/docs/http/http-spans.md#http-server>
    attributes.push(KeyValue::new(HTTP_ROUTE, http_route.to_owned()));
    if let Some(remote) = remote_addr {
        attributes.push(KeyValue::new(CLIENT_ADDRESS, remote.to_string()));
    }
    if let Some(peer_addr) = req.peer_addr().map(|socket| socket.ip().to_string()) {
        if Some(peer_addr.as_str()) != remote_addr {
            // Client is going through a proxy
            attributes.push(KeyValue::new(NETWORK_PEER_ADDRESS, peer_addr));
        }
    }
    let mut host_parts = conn_info.host().split_terminator(':');
    if let Some(host) = host_parts.next() {
        attributes.push(KeyValue::new(SERVER_ADDRESS, host.to_string()));
    }
    if let Some(port) = host_parts.next().and_then(|port| port.parse::<i64>().ok()) {
        if port != 80 && port != 443 {
            attributes.push(KeyValue::new(SERVER_PORT, port));
        }
    }
    if let Some(path_query) = req.uri().path_and_query() {
        if path_query.path() != "/" {
            attributes.push(KeyValue::new(URL_PATH, path_query.path().to_string()));
        }
        if let Some(query) = path_query.query() {
            attributes.push(KeyValue::new(URL_QUERY, query.to_string()));
        }
    }
    attributes.push(KeyValue::new(URL_SCHEME, url_scheme(conn_info.scheme())));

    // Common attrs
    // <https://github.com/open-telemetry/semantic-conventions/blob/v1.21.0/docs/http/http-spans.md#common-attributes>
    attributes.push(KeyValue::new(
        HTTP_REQUEST_METHOD,
        http_method_str(req.method()),
    ));
    attributes.push(KeyValue::new(
        NETWORK_PROTOCOL_VERSION,
        protocol_version(req.version()),
    ));

    if let Some(content_length) = req
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|len| len.to_str().ok().and_then(|s| s.parse::<i64>().ok()))
        .filter(|&len| len > 0)
    {
        attributes.push(KeyValue::new(MESSAGING_MESSAGE_BODY_SIZE, content_length));
    }

    if let Some(user_agent) = req
        .headers()
        .get(header::USER_AGENT)
        .and_then(|s| s.to_str().ok())
    {
        attributes.push(KeyValue::new(USER_AGENT_ORIGINAL, user_agent.to_string()));
    }

    attributes
}

/// Create metric attributes for the given request
#[cfg(feature = "metrics")]
pub fn metrics_attributes_from_request(
    req: &ServiceRequest,
    http_route: std::borrow::Cow<'static, str>,
) -> Vec<KeyValue> {
    let conn_info = req.connection_info();

    let mut attributes = Vec::with_capacity(7);
    attributes.push(KeyValue::new(HTTP_ROUTE, http_route));
    attributes.push(KeyValue::new(
        HTTP_REQUEST_METHOD,
        http_method_str(req.method()),
    ));
    attributes.push(KeyValue::new(
        NETWORK_PROTOCOL_VERSION,
        protocol_version(req.version()),
    ));

    let mut host_parts = conn_info.host().split_terminator(':');
    if let Some(host) = host_parts.next() {
        attributes.push(KeyValue::new(SERVER_ADDRESS, host.to_string()));
    }
    if let Some(port) = host_parts.next().and_then(|port| port.parse::<i64>().ok()) {
        attributes.push(KeyValue::new(SERVER_PORT, port))
    }
    attributes.push(KeyValue::new(URL_SCHEME, url_scheme(conn_info.scheme())));

    attributes
}
