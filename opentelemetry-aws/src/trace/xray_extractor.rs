use http::HeaderMap;
use opentelemetry::propagation::Extractor;
use std::{collections::HashMap, env};

pub const TRACE_ID_ENVIRONMENT_VARIABLE: &str = "_X_AMZN_TRACE_ID";

pub const TRACE_ID_HEADER: &str = "x-amzn-trace-id";

/// Extractor to provide the X-Ray Trace ID based on the [`TRACE_ID_HEADER`] and using the [`TRACE_ID_ENVIRONMENT_VARIABLE`] as a fallback.
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

        if let Ok(value) = env::var(TRACE_ID_ENVIRONMENT_VARIABLE) {
            values.insert(TRACE_ID_HEADER.to_string(), value);
        }

        header_map.iter().for_each(|(key, value)| {
            if let Ok(value) = value.to_str() {
                values.insert(key.to_string(), value.to_string());
            }
        });

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

    fn test_data() -> Vec<(
        Option<&'static str>,
        Option<&'static str>,
        Option<&'static str>,
    )> {
        vec![
            (None, None, None),
            (Some("1"), None, Some("1")),
            (None, Some("2"), Some("2")),
            (Some("1"), Some("2"), Some("2")),
        ]
    }

    #[sealed_test]
    fn test_get() {
        for (environment_variable, header_value, expected) in test_data() {
            temp_env::with_vars(
                [(TRACE_ID_ENVIRONMENT_VARIABLE, environment_variable)],
                || {
                    let extractor: XrayExtractor;

                    if let Some(header_value) = header_value {
                        let header_map = vec![(
                            HeaderName::from_static(TRACE_ID_HEADER),
                            HeaderValue::from_static(header_value),
                        )]
                        .into_iter()
                        .collect();
                        extractor = XrayExtractor::from_header_map(header_map);
                    } else {
                        extractor = XrayExtractor::new();
                    }

                    assert_eq!(extractor.get(TRACE_ID_HEADER), expected);
                },
            );
        }
    }

    #[test]
    fn test_keys() {
        let header_map: HeaderMap = vec![(
            HeaderName::from_static(TRACE_ID_HEADER),
            HeaderValue::from_static(""),
        )]
        .into_iter()
        .collect();
        let extractor = XrayExtractor::from_header_map(header_map);
        assert_eq!(extractor.keys(), vec![TRACE_ID_HEADER]);
    }
}
