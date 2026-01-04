use http::HeaderMap;
use opentelemetry::propagation::Extractor;
use std::{collections::HashMap, env};

const TRACE_ID_ENVIRONMENT_VARIABLE: &str = "_X_AMZN_TRACE_ID";

const TRACE_ID_HEADER: &str = "x-amzn-trace-id";

/// Extractor to provide the X-Ray Trace ID based on the [`TRACE_ID_HEADER`] and using the [`TRACE_ID_ENVIRONMENT_VARIABLE`] as a fallback.
#[derive(Clone, Debug, Default)]
pub struct XRayExtractor {
    values: HashMap<String, String>,
}

impl XRayExtractor {
    /// Creates a new XRayExtractor.
    pub fn new() -> Self {
        Self::from_header_map(HeaderMap::new())
    }

    /// Creates a new XRayExtractor with the given [`HeaderMap`].
    pub fn from_header_map(header_map: HeaderMap) -> Self {
        let mut values: HashMap<String, String> = HashMap::new();

        if let Ok(value) = env::var(TRACE_ID_ENVIRONMENT_VARIABLE) {
            values.insert(TRACE_ID_HEADER.to_string(), value);
        }

        for (header_name, header_value) in header_map {
            if let Some(key) = header_name {
                if let Ok(value) = header_value.to_str() {
                    values.insert(key.to_string(), value.to_string());
                }
            }
        }

        XRayExtractor { values }
    }
}

impl Extractor for XRayExtractor {
    fn get(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(|x| x.as_str())
    }

    fn keys(&self) -> Vec<&str> {
        self.values
            .keys()
            .map(|value| value.as_str())
            .collect::<Vec<_>>()
    }

    fn get_all(&self, key: &str) -> Option<Vec<&str>> {
        self.values.get_all(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::{HeaderMap, HeaderName, HeaderValue};
    use opentelemetry::propagation::Extractor;
    use std::env;

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

    #[test]
    fn test_get() {
        for (environment_variable, header_value, expected) in test_data() {
            if let Some(environment_variable) = environment_variable {
                unsafe {
                    env::set_var(TRACE_ID_ENVIRONMENT_VARIABLE, environment_variable);
                }
            }

            let extractor: XRayExtractor;

            if let Some(header_value) = header_value {
                let header_map = vec![(
                    HeaderName::from_static(TRACE_ID_HEADER),
                    HeaderValue::from_static(header_value),
                )]
                .into_iter()
                .collect();
                extractor = XRayExtractor::from_header_map(header_map);
            } else {
                extractor = XRayExtractor::new();
            }

            assert_eq!(extractor.get(TRACE_ID_HEADER), expected);

            if environment_variable.is_some() {
                unsafe {
                    env::remove_var(TRACE_ID_ENVIRONMENT_VARIABLE);
                }
            }
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
        let extractor = XRayExtractor::from_header_map(header_map);
        assert_eq!(extractor.keys(), vec![TRACE_ID_HEADER]);
    }

    #[test]
    fn test_get_all() {
        let header_map: HeaderMap = vec![(
            HeaderName::from_static(TRACE_ID_HEADER),
            HeaderValue::from_static("1"),
        )]
        .into_iter()
        .collect();
        let extractor = XRayExtractor::from_header_map(header_map);
        assert_eq!(extractor.get_all(TRACE_ID_HEADER), Some(vec!["1"]));
    }
}
