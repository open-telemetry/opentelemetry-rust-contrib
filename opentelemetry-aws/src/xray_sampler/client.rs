use std::time::Duration;

use super::rule::{GetSamplingRulesResponse, SamplingRule};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

/// Client for the sampling API of the X-Ray daemon or OpenTelemetry Collector.
#[derive(Debug)]
pub(super) struct SamplingClient {
    agent: ureq::Agent,
    rules_url: String,
}

impl SamplingClient {
    pub(super) fn new(endpoint: &str) -> Self {
        Self {
            agent: ureq::Agent::config_builder()
                .proxy(None)
                .timeout_global(Some(REQUEST_TIMEOUT))
                .build()
                .into(),
            rules_url: format!("{}/GetSamplingRules", endpoint.trim_end_matches('/')),
        }
    }

    /// Fetches the sampling rules. Returns `None` if the request fails or the response is invalid.
    pub(super) fn get_sampling_rules(&self) -> Option<Vec<SamplingRule>> {
        let result = self
            .agent
            .post(&self.rules_url)
            .header("content-type", "application/json")
            .send_empty()
            .and_then(|mut response| response.body_mut().read_json::<GetSamplingRulesResponse>());

        match result {
            Ok(response) => Some(response.into_rules()),
            Err(_error) => {
                #[cfg(feature = "internal-logs")]
                tracing::warn!(error = %_error, "Failed to fetch AWS X-Ray sampling rules");
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::serve_once;

    #[test]
    fn fetches_rules() {
        let (endpoint, server) = serve_once(
            "200 OK",
            r#"{"SamplingRuleRecords": [{"SamplingRule": {"RuleName": "Default", "Priority": 10000, "FixedRate": 0.05, "Version": 1}}]}"#,
        );

        let rules = SamplingClient::new(&endpoint).get_sampling_rules().unwrap();

        assert_eq!(server.join().unwrap(), "POST /GetSamplingRules HTTP/1.1");
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].rule_name, "Default");
        assert_eq!(rules[0].fixed_rate, 0.05);
    }

    #[test]
    fn returns_none_on_error_status() {
        let (endpoint, server) = serve_once("500 Internal Server Error", "{}");
        assert!(SamplingClient::new(&endpoint)
            .get_sampling_rules()
            .is_none());
        server.join().unwrap();
    }

    #[test]
    fn returns_none_on_invalid_body() {
        let (endpoint, server) = serve_once("200 OK", r#"{"unexpected": true}"#);
        assert!(SamplingClient::new(&endpoint)
            .get_sampling_rules()
            .is_none());
        server.join().unwrap();
    }
}
