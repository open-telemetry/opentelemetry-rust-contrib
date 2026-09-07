//! Common HTTP attribute helpers.
//!
//! The helpers here follow the OpenTelemetry HTTP semantic conventions:
//! <https://opentelemetry.io/docs/specs/semconv/http/http-spans/>

use opentelemetry::KeyValue;
use opentelemetry_semantic_conventions as semconv;

/// Value of `http.request.method` for a method the instrumentation does not know.
const HTTP_REQUEST_METHOD_OTHER: &str = "_OTHER";

/// Maps the HTTP methods known to the semantic conventions to a `&'static str`
/// so the resulting `KeyValue` stores the method as a static string (no heap
/// allocation, allocation-free `KeyValue::clone()`).
///
/// The known set is the methods of
/// [RFC 9110](https://www.rfc-editor.org/rfc/rfc9110.html#name-methods), `PATCH`
/// from [RFC 5789](https://www.rfc-editor.org/rfc/rfc5789.html), and `QUERY`.
/// Method names are case sensitive, so a method that differs only in case is
/// unknown. Returns `None` for every other method.
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
        _ if m.as_str() == "QUERY" => Some("QUERY"),
        _ => None,
    }
}

/// Builds the `http.request.method` [`KeyValue`] and, for a method the
/// instrumentation does not know, the `http.request.method_original`
/// [`KeyValue`] that keeps the value from the request line.
#[inline]
pub(crate) fn method_kvs(method: &http::Method) -> (KeyValue, Option<KeyValue>) {
    match method_as_static(method) {
        Some(known) => (
            KeyValue::new(semconv::attribute::HTTP_REQUEST_METHOD, known),
            None,
        ),
        None => (
            KeyValue::new(
                semconv::attribute::HTTP_REQUEST_METHOD,
                HTTP_REQUEST_METHOD_OTHER,
            ),
            Some(KeyValue::new(
                semconv::attribute::HTTP_REQUEST_METHOD_ORIGINAL,
                method.as_str().to_owned(),
            )),
        ),
    }
}

/// Builds the `url.scheme` [`KeyValue`], promoting the common `http`/`https`
/// schemes to a `&'static str`.
#[inline]
pub(crate) fn url_scheme_kv(scheme: &str) -> KeyValue {
    match scheme {
        "http" => KeyValue::new(semconv::attribute::URL_SCHEME, "http"),
        "https" => KeyValue::new(semconv::attribute::URL_SCHEME, "https"),
        other => KeyValue::new(semconv::attribute::URL_SCHEME, other.to_owned()),
    }
}

/// Maps an HTTP version to its `network.protocol.version` value.
///
/// Returns `None` for a version the instrumentation does not know, because the
/// conventions ask instrumentations to leave the attribute unset when the
/// protocol version is unknown.
#[inline]
pub(crate) fn protocol_version(http_version: http::Version) -> Option<&'static str> {
    match http_version {
        http::Version::HTTP_09 => Some("0.9"),
        http::Version::HTTP_10 => Some("1.0"),
        http::Version::HTTP_11 => Some("1.1"),
        http::Version::HTTP_2 => Some("2"),
        http::Version::HTTP_3 => Some("3"),
        _ => None,
    }
}

/// Reads a directive from the first element of the
/// [RFC 7239](https://www.rfc-editor.org/rfc/rfc7239) `Forwarded` header.
///
/// The first element holds the values closest to the original client. Directive
/// names are case insensitive, and values may be quoted.
pub(crate) fn forwarded_directive<'h>(
    headers: &'h http::HeaderMap,
    directive: &str,
) -> Option<&'h str> {
    let header = headers.get("forwarded")?.to_str().ok()?;
    let first_element = header.split(',').next()?;

    for pair in first_element.split(';') {
        let mut parts = pair.splitn(2, '=');
        let name = parts.next().unwrap_or_default().trim();
        if !name.eq_ignore_ascii_case(directive) {
            continue;
        }
        let value = parts.next().unwrap_or_default().trim().trim_matches('"');
        if value.is_empty() {
            return None;
        }
        return Some(value);
    }

    None
}

/// Reads the first, and therefore most original, value of a comma-separated
/// `X-Forwarded-*` header.
pub(crate) fn first_forwarded_value<'h>(
    headers: &'h http::HeaderMap,
    header: &str,
) -> Option<&'h str> {
    let value = headers
        .get(header)?
        .to_str()
        .ok()?
        .split(',')
        .next()?
        .trim();
    if value.is_empty() {
        return None;
    }
    Some(value)
}

/// Splits an authority into its host and port, dropping the brackets of an IPv6
/// literal so that the host reads like the address examples in the conventions.
pub(crate) fn split_host_port(authority: &str) -> Option<(&str, Option<u16>)> {
    let authority = authority.trim();
    if authority.is_empty() {
        return None;
    }

    let (host, port) = match authority.strip_prefix('[') {
        // IPv6 literal: the port, if any, follows the closing bracket.
        Some(rest) => {
            let (host, after) = rest.split_once(']')?;
            (host, after.strip_prefix(':'))
        }
        None => match authority.split_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (authority, None),
        },
    };

    if host.is_empty() {
        return None;
    }

    Some((host, port.and_then(|port| port.parse().ok())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn method_kvs_maps_known_and_unknown_methods() {
        struct TestCase {
            name: &'static str,
            method: &'static str,
            expected_method: KeyValue,
            expected_original: Option<KeyValue>,
        }

        let method_kv = |value: &'static str| {
            KeyValue::new(semconv::attribute::HTTP_REQUEST_METHOD, value.to_owned())
        };
        let original_kv = |value: &'static str| {
            KeyValue::new(
                semconv::attribute::HTTP_REQUEST_METHOD_ORIGINAL,
                value.to_owned(),
            )
        };

        let test_cases = [
            TestCase {
                name: "known method",
                method: "GET",
                expected_method: method_kv("GET"),
                expected_original: None,
            },
            TestCase {
                name: "patch is known",
                method: "PATCH",
                expected_method: method_kv("PATCH"),
                expected_original: None,
            },
            TestCase {
                name: "query is known",
                method: "QUERY",
                expected_method: method_kv("QUERY"),
                expected_original: None,
            },
            TestCase {
                name: "method names are case sensitive",
                method: "GeT",
                expected_method: method_kv("_OTHER"),
                expected_original: Some(original_kv("GeT")),
            },
            TestCase {
                name: "extension method is unknown",
                method: "ACL",
                expected_method: method_kv("_OTHER"),
                expected_original: Some(original_kv("ACL")),
            },
        ];

        for test_case in test_cases {
            let method = http::Method::from_bytes(test_case.method.as_bytes()).unwrap();

            let result = method_kvs(&method);

            assert_eq!(
                result,
                (test_case.expected_method, test_case.expected_original),
                "{}",
                test_case.name
            );
        }
    }

    #[test]
    fn protocol_version_maps_known_versions() {
        struct TestCase {
            name: &'static str,
            version: http::Version,
            expected: Option<&'static str>,
        }

        let test_cases = [
            TestCase {
                name: "http 0.9",
                version: http::Version::HTTP_09,
                expected: Some("0.9"),
            },
            TestCase {
                name: "http 1.0",
                version: http::Version::HTTP_10,
                expected: Some("1.0"),
            },
            TestCase {
                name: "http 1.1",
                version: http::Version::HTTP_11,
                expected: Some("1.1"),
            },
            TestCase {
                name: "http 2 has no minor version",
                version: http::Version::HTTP_2,
                expected: Some("2"),
            },
            TestCase {
                name: "http 3 has no minor version",
                version: http::Version::HTTP_3,
                expected: Some("3"),
            },
        ];

        for test_case in test_cases {
            let result = protocol_version(test_case.version);

            assert_eq!(result, test_case.expected, "{}", test_case.name);
        }
    }

    #[test]
    fn forwarded_directive_reads_the_first_element() {
        struct TestCase {
            name: &'static str,
            header: Option<&'static str>,
            directive: &'static str,
            expected: Option<&'static str>,
        }

        let test_cases = [
            TestCase {
                name: "missing header",
                header: None,
                directive: "for",
                expected: None,
            },
            TestCase {
                name: "single directive",
                header: Some("for=203.0.113.7"),
                directive: "for",
                expected: Some("203.0.113.7"),
            },
            TestCase {
                name: "directive names are case insensitive",
                header: Some("For=203.0.113.7"),
                directive: "for",
                expected: Some("203.0.113.7"),
            },
            TestCase {
                name: "quoted value",
                header: Some(r#"for="[2001:db8::17]:4711""#),
                directive: "for",
                expected: Some("[2001:db8::17]:4711"),
            },
            TestCase {
                name: "several directives",
                header: Some("by=198.51.100.1;for=203.0.113.7;host=example.com;proto=https"),
                directive: "host",
                expected: Some("example.com"),
            },
            TestCase {
                name: "first element wins",
                header: Some("for=203.0.113.7, for=198.51.100.17"),
                directive: "for",
                expected: Some("203.0.113.7"),
            },
            TestCase {
                name: "directive of a later element is ignored",
                header: Some("for=203.0.113.7, host=example.com"),
                directive: "host",
                expected: None,
            },
            TestCase {
                name: "absent directive",
                header: Some("for=203.0.113.7"),
                directive: "proto",
                expected: None,
            },
        ];

        for test_case in test_cases {
            let mut headers = http::HeaderMap::new();
            if let Some(header) = test_case.header {
                headers.insert("forwarded", http::HeaderValue::from_static(header));
            }

            let result = forwarded_directive(&headers, test_case.directive);

            assert_eq!(result, test_case.expected, "{}", test_case.name);
        }
    }

    #[test]
    fn first_forwarded_value_reads_the_first_entry() {
        struct TestCase {
            name: &'static str,
            header: Option<&'static str>,
            expected: Option<&'static str>,
        }

        let test_cases = [
            TestCase {
                name: "missing header",
                header: None,
                expected: None,
            },
            TestCase {
                name: "single entry",
                header: Some("203.0.113.7"),
                expected: Some("203.0.113.7"),
            },
            TestCase {
                name: "proxy chain",
                header: Some("203.0.113.7, 198.51.100.17, 10.0.0.1"),
                expected: Some("203.0.113.7"),
            },
            TestCase {
                name: "empty value",
                header: Some(""),
                expected: None,
            },
        ];

        for test_case in test_cases {
            let mut headers = http::HeaderMap::new();
            if let Some(header) = test_case.header {
                headers.insert("x-forwarded-for", http::HeaderValue::from_static(header));
            }

            let result = first_forwarded_value(&headers, "x-forwarded-for");

            assert_eq!(result, test_case.expected, "{}", test_case.name);
        }
    }

    #[test]
    fn split_host_port_splits_authorities() {
        struct TestCase {
            name: &'static str,
            authority: &'static str,
            expected: Option<(&'static str, Option<u16>)>,
        }

        let test_cases = [
            TestCase {
                name: "host only",
                authority: "example.com",
                expected: Some(("example.com", None)),
            },
            TestCase {
                name: "host and port",
                authority: "example.com:8443",
                expected: Some(("example.com", Some(8443))),
            },
            TestCase {
                name: "default port is kept",
                authority: "example.com:80",
                expected: Some(("example.com", Some(80))),
            },
            TestCase {
                name: "ipv4 and port",
                authority: "10.1.2.80:8080",
                expected: Some(("10.1.2.80", Some(8080))),
            },
            TestCase {
                name: "ipv6 literal loses its brackets",
                authority: "[2001:db8::17]",
                expected: Some(("2001:db8::17", None)),
            },
            TestCase {
                name: "ipv6 literal and port",
                authority: "[2001:db8::17]:4711",
                expected: Some(("2001:db8::17", Some(4711))),
            },
            TestCase {
                name: "surrounding spaces",
                authority: " example.com:8443 ",
                expected: Some(("example.com", Some(8443))),
            },
            TestCase {
                name: "empty authority",
                authority: "",
                expected: None,
            },
            TestCase {
                name: "port only",
                authority: ":8443",
                expected: None,
            },
            TestCase {
                name: "unparsable port",
                authority: "example.com:https",
                expected: Some(("example.com", None)),
            },
        ];

        for test_case in test_cases {
            let result = split_host_port(test_case.authority);

            assert_eq!(result, test_case.expected, "{}", test_case.name);
        }
    }
}
