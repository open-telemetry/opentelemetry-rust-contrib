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
/// Reads the query once. A query that holds no sensitive parameter, which is the
/// common case, is returned unchanged and allocates nothing. A query that holds
/// one allocates on the first match, and copies the regions in between verbatim.
pub(crate) fn redact_query<'q, S>(query: &'q str, sensitive: &[S]) -> Cow<'q, str>
where
    S: AsRef<str>,
{
    let mut redacted: Option<String> = None;
    // Byte offset up to which `query` has been copied into `redacted`.
    let mut copied = 0;
    // Byte offset of the parameter under inspection.
    let mut pair_start = 0;

    loop {
        let pair_end = query[pair_start..]
            .find('&')
            .map_or(query.len(), |offset| pair_start + offset);
        let key_end = query[pair_start..pair_end]
            .find('=')
            .map_or(pair_end, |offset| pair_start + offset);

        if sensitive
            .iter()
            .any(|parameter| parameter.as_ref() == &query[pair_start..key_end])
        {
            let redacted =
                redacted.get_or_insert_with(|| String::with_capacity(query.len() + REDACTED.len()));
            // Everything up to and including the key stays as it arrived.
            redacted.push_str(&query[copied..key_end]);
            redacted.push('=');
            redacted.push_str(REDACTED);
            copied = pair_end;
        }

        if pair_end == query.len() {
            break;
        }
        pair_start = pair_end + 1;
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
    fn redact_query_replaces_sensitive_values() {
        struct TestCase {
            name: &'static str,
            query: &'static str,
            sensitive: &'static [&'static str],
            expected: Cow<'static, str>,
            /// Whether the helper returns the query without allocating.
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
}
