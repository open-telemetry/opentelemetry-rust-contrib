//! Common HTTP attribute helpers.

use std::borrow::Cow;

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

/// Query parameter keys whose values the conventions ask instrumentations to
/// redact by default.
///
/// Each key names a credential or a signature of a pre-signed URL. The
/// conventions state that this list changes over time, and they ask for a
/// case-sensitive match.
///
/// See <https://opentelemetry.io/docs/specs/semconv/registry/attributes/url/>.
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
///
/// Returns the query unchanged, and allocates nothing, when it holds no
/// sensitive parameter. That is the common case.
pub(crate) fn redact_query<'q, S>(query: &'q str, sensitive: &[S]) -> Cow<'q, str>
where
    S: AsRef<str>,
{
    if !sensitive
        .iter()
        .any(|parameter| query.contains(parameter.as_ref()))
    {
        return Cow::Borrowed(query);
    }

    let mut redacted = String::with_capacity(query.len());
    for (index, pair) in query.split('&').enumerate() {
        if index > 0 {
            redacted.push('&');
        }

        let key = pair.split('=').next().unwrap_or(pair);
        if sensitive.iter().any(|parameter| parameter.as_ref() == key) {
            redacted.push_str(key);
            redacted.push('=');
            redacted.push_str(REDACTED);
        } else {
            redacted.push_str(pair);
        }
    }

    Cow::Owned(redacted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redact_query_replaces_sensitive_values() {
        struct TestCase {
            name: &'static str,
            query: &'static str,
            sensitive: &'static [&'static str],
            expected: Cow<'static, str>,
            /// Whether the helper returns the query without allocating. The
            /// fast path tests for a substring, so a key that merely contains a
            /// sensitive key allocates even though nothing is redacted.
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
                expected_borrowed: false,
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
}
