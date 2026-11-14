//! Common HTTP attribute helpers.

use std::borrow::Cow;

use http::StatusCode;
use opentelemetry::KeyValue;
use opentelemetry_semantic_conventions as semconv;
use opentelemetry_semantic_conventions::attribute::{CLIENT_ADDRESS, ERROR_TYPE};

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

/// Builds the `error.type` [`KeyValue`] for a response whose status code the
/// HTTP conventions treat as an error.
///
/// Promotes common 5xx codes to a `&'static str` for an
/// allocation-free clone in the hot path, and falls back to an owned `String`
/// for every other status code.
#[inline]
pub(crate) fn error_type_kv(status: http::StatusCode) -> KeyValue {
    // From the semantic convention: once a status code exists and indicates an
    // error, `error.type` should be "the status code number (represented as a
    // string)".
    match status {
        StatusCode::INTERNAL_SERVER_ERROR => KeyValue::new(ERROR_TYPE, "500"),
        StatusCode::NOT_IMPLEMENTED => KeyValue::new(ERROR_TYPE, "501"),
        StatusCode::BAD_GATEWAY => KeyValue::new(ERROR_TYPE, "502"),
        StatusCode::SERVICE_UNAVAILABLE => KeyValue::new(ERROR_TYPE, "503"),
        StatusCode::GATEWAY_TIMEOUT => KeyValue::new(ERROR_TYPE, "504"),
        StatusCode::HTTP_VERSION_NOT_SUPPORTED => KeyValue::new(ERROR_TYPE, "505"),
        _ => KeyValue::new(ERROR_TYPE, status.as_str().to_owned()),
    }
}

/// Extracts the `client.address` value from an `X-Forwarded-For` header value.
///
/// The conventions ask for the original client address behind all proxies. When
/// a chain of proxies has forwarded the request, the first entry of the header
/// is the original client, and every following entry was added by an
/// intermediary.
pub(crate) fn client_address_from_forwarded_for(value: &str) -> &str {
    match value.find(',') {
        Some(index) => value[..index].trim(),
        None => value.trim(),
    }
}

/// Builds the `client.address` [`KeyValue`] for a server request.
///
/// The conventions ask for the original client address behind all proxies when
/// one is available, and for the immediate peer address otherwise. A Tower
/// `Service` sees the request after the network stack has parsed it, so this
/// reads the forwarded headers, and falls back to the socket address of the
/// immediate peer when the caller can provide one.
#[inline]
pub(crate) fn client_address_kv(
    forwarded_for: Option<&str>,
    forwarded: Option<&str>,
    connect_info: Option<&str>,
) -> Option<KeyValue> {
    let address = forwarded_for
        .map(client_address_from_forwarded_for)
        .filter(|address| !address.is_empty())
        .or_else(|| forwarded.and_then(for_node_name).filter(|a| !a.is_empty()))
        .or_else(|| connect_info.and_then(parse_socket_address_host));
    address.map(|address| KeyValue::new(CLIENT_ADDRESS, address.to_owned()))
}

/// Extracts the node name from an RFC 7239 `Forwarded` header value.
///
/// The `for` directive can appear several times, one per proxy hop, and the
/// first one describes the original client. The node name may be quoted and may
/// carry an obfuscated identifier, which the conventions accept as
/// `client.address`.
fn for_node_name(value: &str) -> Option<&str> {
    let start = value.find("for=")?;
    let after = &value[start + 4..];
    if after.is_empty() {
        return None;
    }
    if let Some(quoted) = after.strip_prefix('"') {
        return quoted.find('"').map(|end| &quoted[..end]);
    }
    Some(after.split(';').next().unwrap_or(after))
}

/// Extracts the host from a socket address, for example `"10.1.2.80:65123"` or
/// `"[2001:db8::1]:443"`.
fn parse_socket_address_host(socket_address: &str) -> Option<&str> {
    match socket_address.rsplit_once(':') {
        Some((host, port))
            if !port.is_empty() && port.bytes().all(|byte| byte.is_ascii_digit()) =>
        {
            Some(host.trim_start_matches('[').trim_end_matches(']'))
        }
        _ => None,
    }
}

/// Query parameter keys whose values the conventions ask instrumentations to
/// redact by default.
///
/// See the note on
/// [`URL_QUERY`](https://opentelemetry.io/docs/specs/semconv/registry/attributes/url/#url-query).
/// The conventions state that the list changes over time, and they ask for a
/// case-sensitive match.
pub(crate) const DEFAULT_SENSITIVE_QUERY_PARAMETERS: &[&str] = &[
    "X-Amz-Signature",
    "X-Amz-Credential",
    "X-Amz-Security-Token",
    "AWSAccessKeyId",
    "Signature",
    "sig",
    "X-Goog-Signature",
];

/// Value that replaces the value of a sensitive query parameter.
const REDACTED: &str = "REDACTED";

/// Replaces the value of every sensitive query parameter with `REDACTED`, and
/// keeps the key.
pub(crate) fn redact_query<'q, S>(query: &'q str, sensitive: &[S]) -> Cow<'q, str>
where
    S: AsRef<str>,
{
    let mut redacted: Option<String> = None;
    // Byte offset up to which `query` has been copied into `redacted`.
    let mut copied = 0;
    // Byte offset of the parameter under inspection.
    let mut pair_start = 0;

    for pair in query.split('&') {
        let key = pair.split('=').next().unwrap_or(pair);

        if sensitive.iter().any(|parameter| parameter.as_ref() == key) {
            let redacted =
                redacted.get_or_insert_with(|| String::with_capacity(query.len() + REDACTED.len()));
            // Everything up to and including the key stays as it arrived.
            redacted.push_str(&query[copied..pair_start + key.len()]);
            redacted.push('=');
            redacted.push_str(REDACTED);
            copied = pair_start + pair.len();
        }

        pair_start += pair.len() + '&'.len_utf8();
    }

    match redacted {
        Some(mut redacted) => {
            redacted.push_str(&query[copied..]);
            Cow::Owned(redacted)
        }
        None => Cow::Borrowed(query),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_address_kv_prefers_the_first_forwarded_for_entry() {
        struct TestCase {
            name: &'static str,
            forwarded_for: Option<&'static str>,
            forwarded: Option<&'static str>,
            connect_info: Option<&'static str>,
            expected: Option<KeyValue>,
        }

        let test_cases = [
            TestCase {
                name: "the first entry of a forwarded chain is the original client",
                forwarded_for: Some("203.0.113.195, 70.41.3.18, 150.172.238.178"),
                forwarded: None,
                connect_info: Some("10.1.2.80:65123"),
                expected: Some(KeyValue::new(CLIENT_ADDRESS, "203.0.113.195")),
            },
            TestCase {
                name: "a single entry is the client itself",
                forwarded_for: Some("203.0.113.195"),
                forwarded: None,
                connect_info: Some("10.1.2.80:65123"),
                expected: Some(KeyValue::new(CLIENT_ADDRESS, "203.0.113.195")),
            },
            TestCase {
                name: "a value with spaces around it is trimmed",
                forwarded_for: Some(" 203.0.113.195 , 70.41.3.18"),
                forwarded: None,
                connect_info: Some("10.1.2.80:65123"),
                expected: Some(KeyValue::new(CLIENT_ADDRESS, "203.0.113.195")),
            },
            TestCase {
                name: "an empty header falls back to the next source",
                forwarded_for: Some(""),
                forwarded: None,
                connect_info: Some("10.1.2.80:65123"),
                expected: Some(KeyValue::new(CLIENT_ADDRESS, "10.1.2.80")),
            },
            TestCase {
                name: "a Forwarded header is used when X-Forwarded-For is absent",
                forwarded_for: None,
                forwarded: Some("for=192.0.2.43;by=203.0.113.60, for=198.51.100.17"),
                connect_info: Some("10.1.2.80:65123"),
                expected: Some(KeyValue::new(CLIENT_ADDRESS, "192.0.2.43")),
            },
            TestCase {
                name: "a quoted Forwarded node name keeps its inner value",
                forwarded_for: None,
                forwarded: Some("for=\"[2001:db8:cafe::17]:4711\""),
                connect_info: Some("10.1.2.80:65123"),
                expected: Some(KeyValue::new(CLIENT_ADDRESS, "[2001:db8:cafe::17]:4711")),
            },
            TestCase {
                name: "an obfuscated Forwarded identifier is accepted as an address",
                forwarded_for: None,
                forwarded: Some("for=_hidden"),
                connect_info: Some("10.1.2.80:65123"),
                expected: Some(KeyValue::new(CLIENT_ADDRESS, "_hidden")),
            },
            TestCase {
                name: "a Forwarded header without a for directive falls back",
                forwarded_for: None,
                forwarded: Some("by=203.0.113.60;host=example.com"),
                connect_info: Some("10.1.2.80:65123"),
                expected: Some(KeyValue::new(CLIENT_ADDRESS, "10.1.2.80")),
            },
            TestCase {
                name: "an IPv6 socket address keeps its brackets",
                forwarded_for: None,
                forwarded: None,
                connect_info: Some("[2001:db8::1]:443"),
                expected: Some(KeyValue::new(CLIENT_ADDRESS, "2001:db8::1")),
            },
            TestCase {
                name: "a Unix domain socket path is not a host and port pair",
                forwarded_for: None,
                forwarded: None,
                connect_info: Some("/tmp/my.sock"),
                expected: None,
            },
            TestCase {
                name: "no source at all emits no attribute",
                forwarded_for: None,
                forwarded: None,
                connect_info: None,
                expected: None,
            },
        ];

        for test_case in test_cases {
            let result = client_address_kv(
                test_case.forwarded_for,
                test_case.forwarded,
                test_case.connect_info,
            );

            assert_eq!(result, test_case.expected, "{}", test_case.name);
        }
    }

    #[test]
    fn redact_query_replaces_sensitive_values() {
        struct TestCase {
            name: &'static str,
            query: &'static str,
            sensitive: &'static [&'static str],
            expected: Cow<'static, str>,
            expected_borrowed: bool,
        }

        let test_cases = [
            TestCase {
                name: "no sensitive parameter borrows the query",
                query: "fields=name&verbose=true",
                sensitive: DEFAULT_SENSITIVE_QUERY_PARAMETERS,
                expected: Cow::Borrowed("fields=name&verbose=true"),
                expected_borrowed: true,
            },
            TestCase {
                name: "empty query",
                query: "",
                sensitive: DEFAULT_SENSITIVE_QUERY_PARAMETERS,
                expected: Cow::Borrowed(""),
                expected_borrowed: true,
            },
            TestCase {
                name: "single sensitive parameter",
                query: "sig=abc123",
                sensitive: DEFAULT_SENSITIVE_QUERY_PARAMETERS,
                expected: Cow::Owned(String::from("sig=REDACTED")),
                expected_borrowed: false,
            },
            TestCase {
                name: "sensitive parameter keeps its position",
                query: "q=OpenTelemetry&sig=abc123",
                sensitive: DEFAULT_SENSITIVE_QUERY_PARAMETERS,
                expected: Cow::Owned(String::from("q=OpenTelemetry&sig=REDACTED")),
                expected_borrowed: false,
            },
            TestCase {
                name: "several sensitive parameters",
                query: "X-Amz-Credential=key&X-Amz-Date=today&X-Amz-Signature=abc",
                sensitive: DEFAULT_SENSITIVE_QUERY_PARAMETERS,
                expected: Cow::Owned(String::from(
                    "X-Amz-Credential=REDACTED&X-Amz-Date=today&X-Amz-Signature=REDACTED",
                )),
                expected_borrowed: false,
            },
            TestCase {
                name: "matching is case sensitive",
                query: "SIG=abc123",
                sensitive: DEFAULT_SENSITIVE_QUERY_PARAMETERS,
                expected: Cow::Borrowed("SIG=abc123"),
                expected_borrowed: true,
            },
            TestCase {
                name: "a key that only contains a sensitive key is kept",
                query: "design=modern",
                sensitive: DEFAULT_SENSITIVE_QUERY_PARAMETERS,
                expected: Cow::Borrowed("design=modern"),
                expected_borrowed: true,
            },
            TestCase {
                name: "sensitive key without a value",
                query: "q=OpenTelemetry&sig",
                sensitive: DEFAULT_SENSITIVE_QUERY_PARAMETERS,
                expected: Cow::Owned(String::from("q=OpenTelemetry&sig=REDACTED")),
                expected_borrowed: false,
            },
            TestCase {
                name: "sensitive key with an empty value",
                query: "sig=",
                sensitive: DEFAULT_SENSITIVE_QUERY_PARAMETERS,
                expected: Cow::Owned(String::from("sig=REDACTED")),
                expected_borrowed: false,
            },
            TestCase {
                name: "a value that contains an equals sign",
                query: "sig=abc==&q=OpenTelemetry",
                sensitive: DEFAULT_SENSITIVE_QUERY_PARAMETERS,
                expected: Cow::Owned(String::from("sig=REDACTED&q=OpenTelemetry")),
                expected_borrowed: false,
            },
            TestCase {
                name: "consecutive separators",
                query: "a=1&&sig=abc",
                sensitive: DEFAULT_SENSITIVE_QUERY_PARAMETERS,
                expected: Cow::Owned(String::from("a=1&&sig=REDACTED")),
                expected_borrowed: false,
            },
            TestCase {
                name: "trailing separator",
                query: "sig=abc&",
                sensitive: DEFAULT_SENSITIVE_QUERY_PARAMETERS,
                expected: Cow::Owned(String::from("sig=REDACTED&")),
                expected_borrowed: false,
            },
            TestCase {
                name: "several parameters between two sensitive ones",
                query: "X-Amz-Credential=key&a=1&b=2&X-Amz-Signature=abc",
                sensitive: DEFAULT_SENSITIVE_QUERY_PARAMETERS,
                expected: Cow::Owned(String::from(
                    "X-Amz-Credential=REDACTED&a=1&b=2&X-Amz-Signature=REDACTED",
                )),
                expected_borrowed: false,
            },
            TestCase {
                name: "an empty list redacts nothing",
                query: "sig=abc123",
                sensitive: &[],
                expected: Cow::Borrowed("sig=abc123"),
                expected_borrowed: true,
            },
            TestCase {
                name: "a custom list replaces the default one",
                query: "token=abc123&sig=abc123",
                sensitive: &["token"],
                expected: Cow::Owned(String::from("token=REDACTED&sig=abc123")),
                expected_borrowed: false,
            },
        ];

        for test_case in test_cases {
            let result = redact_query(test_case.query, test_case.sensitive);

            assert_eq!(
                (matches!(result, Cow::Borrowed(_)), result),
                (test_case.expected_borrowed, test_case.expected),
                "{}",
                test_case.name
            );
        }
    }

    #[test]
    fn error_type_kv_builds_the_error_type_attribute() {
        struct TestCase {
            name: &'static str,
            status: http::StatusCode,
            expected: KeyValue,
        }

        let test_cases = [
            TestCase {
                name: "500 uses the static path",
                status: http::StatusCode::INTERNAL_SERVER_ERROR,
                expected: KeyValue::new(semconv::attribute::ERROR_TYPE, "500"),
            },
            TestCase {
                name: "501 uses the static path",
                status: http::StatusCode::NOT_IMPLEMENTED,
                expected: KeyValue::new(semconv::attribute::ERROR_TYPE, "501"),
            },
            TestCase {
                name: "502 uses the static path",
                status: http::StatusCode::BAD_GATEWAY,
                expected: KeyValue::new(semconv::attribute::ERROR_TYPE, "502"),
            },
            TestCase {
                name: "503 uses the static path",
                status: http::StatusCode::SERVICE_UNAVAILABLE,
                expected: KeyValue::new(semconv::attribute::ERROR_TYPE, "503"),
            },
            TestCase {
                name: "504 uses the static path",
                status: http::StatusCode::GATEWAY_TIMEOUT,
                expected: KeyValue::new(semconv::attribute::ERROR_TYPE, "504"),
            },
            TestCase {
                name: "505 uses the static path",
                status: http::StatusCode::HTTP_VERSION_NOT_SUPPORTED,
                expected: KeyValue::new(semconv::attribute::ERROR_TYPE, "505"),
            },
            TestCase {
                name: "an uncommon status code uses the owned path",
                status: http::StatusCode::from_u16(599).unwrap(),
                expected: KeyValue::new(semconv::attribute::ERROR_TYPE, "599".to_string()),
            },
        ];

        for test_case in test_cases {
            let result = error_type_kv(test_case.status);

            assert_eq!(result, test_case.expected, "{}", test_case.name);
        }
    }
}
