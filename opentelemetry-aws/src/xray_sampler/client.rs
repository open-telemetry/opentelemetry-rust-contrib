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
pub(super) mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::thread;

    /// Starts a server that answers one request, and returns the request line it received.
    pub(in crate::xray_sampler) fn serve_once(
        status: &'static str,
        body: &'static str,
    ) -> (String, thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request_line = String::new();
            reader.read_line(&mut request_line).unwrap();
            let mut content_length = 0;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" {
                    break;
                }
                if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    content_length = value.trim().parse().unwrap();
                }
            }
            reader
                .take(content_length)
                .read_to_end(&mut Vec::new())
                .unwrap();
            write!(
                stream,
                "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
            request_line.trim_end().to_owned()
        });
        (endpoint, handle)
    }

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
