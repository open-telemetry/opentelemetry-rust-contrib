use http::HeaderMap;
use opentelemetry::propagation::Extractor;
use std::{collections::HashMap, env};

pub const AWS_XRAY_TRACE_ENVIRONMENT_VARIABLE: &str = "_X_AMZN_TRACE_ID";

pub const AWS_XRAY_TRACE_HEADER: &str = "x-amzn-trace-id";

/// Extractor to provide the X-Ray Trace ID based on the [`AWS_XRAY_TRACE_HEADER`] and using the [`AWS_XRAY_TRACE_ENVIRONMENT_VARIABLE`] as a fallback.
#[derive(Clone, Debug, Default)]
pub struct XrayExtractor {
    values: HashMap<String, String>,
}

impl XrayExtractor {
    /// Creates a new XrayExtractor.
    pub fn new() -> Self {
        Self::from_header_map(HeaderMap::new())
    }

    /// Creates a new XrayExtractor with the given [`HeaderMap`].
    pub fn from_header_map(header_map: HeaderMap) -> Self {
        let mut values: HashMap<String, String> = HashMap::new();

        if let Ok(value) = env::var(AWS_XRAY_TRACE_ENVIRONMENT_VARIABLE) {
            values.insert(AWS_XRAY_TRACE_ENVIRONMENT_VARIABLE.to_string(), value);
        }

        if let Some(value) = header_map.get(AWS_XRAY_TRACE_HEADER) {
            if let Ok(header_value) = value.to_str() {
                values.insert(AWS_XRAY_TRACE_HEADER.to_string(), header_value.to_string());
            }
        }

        XrayExtractor { values }
    }
}

impl Extractor for XrayExtractor {
    fn get(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(|x| x.as_str())
    }

    fn keys(&self) -> Vec<&str> {
        self.values
            .keys()
            .map(|value| value.as_str())
            .collect::<Vec<_>>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::{HeaderMap, HeaderName, HeaderValue};
    use opentelemetry::propagation::Extractor;
    use sealed_test::prelude::{rusty_fork_test, sealed_test, tempfile};

    struct Pair(&'static str, &'static str);

    const NO_TRACE_ID_HEADER: &str = "x-test";

    fn test_data() -> Vec<(Option<&'static str>, Option<Pair>, Option<Pair>)> {
        vec![
            (None, None, None),
            (None, Some(Pair(NO_TRACE_ID_HEADER, "2")), None),
            (
                None,
                Some(Pair(AWS_XRAY_TRACE_HEADER, "2")),
                Some(Pair(AWS_XRAY_TRACE_HEADER, "2")),
            ),
            (
                Some("1"),
                None,
                Some(Pair(AWS_XRAY_TRACE_ENVIRONMENT_VARIABLE, "1")),
            ),
            (
                Some("1"),
                Some(Pair(NO_TRACE_ID_HEADER, "2")),
                Some(Pair(AWS_XRAY_TRACE_ENVIRONMENT_VARIABLE, "1")),
            ),
            (
                Some("1"),
                Some(Pair(AWS_XRAY_TRACE_HEADER, "2")),
                Some(Pair(AWS_XRAY_TRACE_HEADER, "2")),
            ),
        ]
    }

    #[sealed_test]
    fn test_get() {
        for (environment_variable, header_name_value, expected) in test_data() {
            temp_env::with_vars(
                [(AWS_XRAY_TRACE_ENVIRONMENT_VARIABLE, environment_variable)],
                || {
                    let extractor: XrayExtractor;

                    if let Some(pair) = header_name_value {
                        let header_map = vec![(
                            HeaderName::from_static(pair.0),
                            HeaderValue::from_static(pair.1),
                        )]
                        .into_iter()
                        .collect();
                        extractor = XrayExtractor::from_header_map(header_map);
                    } else {
                        extractor = XrayExtractor::new();
                    }

                    match expected {
                        None => {
                            assert!(extractor.values.is_empty());
                        }
                        Some(pair) => {
                            assert_eq!(extractor.get(pair.0), Some(pair.1));
                        }
                    }
                },
            );
        }
    }

    #[test]
    fn test_keys() {
        let header_map: HeaderMap = vec![(
            HeaderName::from_static(AWS_XRAY_TRACE_HEADER),
            HeaderValue::from_static(""),
        )]
        .into_iter()
        .collect();
        let extractor = XrayExtractor::from_header_map(header_map);
        assert_eq!(extractor.keys(), vec![AWS_XRAY_TRACE_HEADER]);
    }
}
