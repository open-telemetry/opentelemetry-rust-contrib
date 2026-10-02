//! AWS X-Ray remote sampler (`xray-sampler` feature).
//!
//! Samples spans using [sampling rules] from AWS X-Ray. Reservoirs are not supported yet.
//!
//! [sampling rules]: https://docs.aws.amazon.com/xray/latest/devguide/xray-console-sampling.html

use std::sync::{Arc, OnceLock, PoisonError, RwLock, Weak};
use std::thread::{self, Thread};
use std::time::{Duration, Instant};

use opentelemetry::trace::{Link, SpanKind, TraceId};
use opentelemetry::{Context, KeyValue};
use opentelemetry_sdk::trace::{Sampler, SamplingResult, ShouldSample};
use opentelemetry_sdk::Resource;

mod client;
mod rule;

use client::SamplingClient;
use rule::{ResourceInfo, SamplingRule};

const DEFAULT_ENDPOINT: &str = "http://localhost:2000";
const DEFAULT_POLLING_INTERVAL: Duration = Duration::from_secs(300);
const MIN_POLLING_INTERVAL: Duration = Duration::from_secs(10);
/// How long fetched rules stay valid.
const RULES_TTL: Duration = Duration::from_secs(3600);
const FALLBACK_RATE: f64 = 0.05;

/// Samples spans using AWS X-Ray sampling rules.
///
/// New traces use the rate of the first matching rule, or 5% if no rules are available.
/// Spans with a parent follow the parent's decision.
///
/// Rules are refreshed in a background thread.
///
/// # Examples
///
/// ```no_run
/// use opentelemetry_aws::xray_sampler::AwsXrayRemoteSampler;
/// use opentelemetry_sdk::{trace::SdkTracerProvider, Resource};
///
/// let resource = Resource::builder().build();
/// let sampler = AwsXrayRemoteSampler::builder(&resource).build();
///
/// let provider = SdkTracerProvider::builder()
///     .with_resource(resource)
///     .with_sampler(sampler)
///     .build();
/// ```
#[derive(Clone, Debug)]
pub struct AwsXrayRemoteSampler {
    root: Sampler,
}

impl AwsXrayRemoteSampler {
    /// Creates a builder. `resource` is used to match rules.
    pub fn builder(resource: &Resource) -> AwsXrayRemoteSamplerBuilder {
        AwsXrayRemoteSamplerBuilder {
            resource: ResourceInfo::new(resource),
            endpoint: DEFAULT_ENDPOINT.to_owned(),
            polling_interval: DEFAULT_POLLING_INTERVAL,
        }
    }
}

impl ShouldSample for AwsXrayRemoteSampler {
    fn should_sample(
        &self,
        parent_context: Option<&Context>,
        trace_id: TraceId,
        name: &str,
        span_kind: &SpanKind,
        attributes: &[KeyValue],
        links: &[Link],
    ) -> SamplingResult {
        self.root
            .should_sample(parent_context, trace_id, name, span_kind, attributes, links)
    }
}

/// Builder for [`AwsXrayRemoteSampler`].
#[derive(Debug)]
pub struct AwsXrayRemoteSamplerBuilder {
    resource: ResourceInfo,
    endpoint: String,
    polling_interval: Duration,
}

impl AwsXrayRemoteSamplerBuilder {
    /// Sets the X-Ray proxy endpoint. Defaults to `http://localhost:2000`.
    pub fn with_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = endpoint.into();
        self
    }

    /// Sets how often rules are fetched. Defaults to 5 minutes; minimum 10 seconds.
    pub fn with_polling_interval(mut self, interval: Duration) -> Self {
        self.polling_interval = interval.max(MIN_POLLING_INTERVAL);
        self
    }

    /// Builds the sampler and starts fetching rules.
    pub fn build(self) -> AwsXrayRemoteSampler {
        let rules_sampler = RulesSampler::new(self.resource);
        rules_sampler.spawn_poller(SamplingClient::new(&self.endpoint), self.polling_interval);
        AwsXrayRemoteSampler {
            root: Sampler::ParentBased(Box::new(rules_sampler)),
        }
    }
}

/// Samples new traces using the fetched rules.
#[derive(Clone, Debug)]
struct RulesSampler {
    state: Arc<State>,
}

#[derive(Debug)]
struct State {
    resource: ResourceInfo,
    rules: RwLock<Option<CachedRules>>,
    poller: OnceLock<Thread>,
}

#[derive(Debug)]
struct CachedRules {
    rules: Vec<SamplingRule>,
    fetched_at: Instant,
}

impl State {
    fn set_rules(&self, rules: Vec<SamplingRule>) {
        let fetched_at = Instant::now();
        *self.rules.write().unwrap_or_else(PoisonError::into_inner) =
            Some(CachedRules { rules, fetched_at });
    }

    /// Returns the rate of the first matching rule, or the default rate.
    fn rate(&self, attributes: &[KeyValue]) -> f64 {
        let rules = self.rules.read().unwrap_or_else(PoisonError::into_inner);
        rules
            .as_ref()
            .filter(|cached| cached.fetched_at.elapsed() < RULES_TTL)
            .and_then(|cached| {
                cached
                    .rules
                    .iter()
                    .find(|rule| rule.matches(&self.resource, attributes))
            })
            .map_or(FALLBACK_RATE, |rule| rule.fixed_rate)
    }
}

impl Drop for State {
    fn drop(&mut self) {
        // Wake the poller so it can exit.
        if let Some(poller) = self.poller.get() {
            poller.unpark();
        }
    }
}

impl RulesSampler {
    fn new(resource: ResourceInfo) -> Self {
        Self {
            state: Arc::new(State {
                resource,
                rules: RwLock::new(None),
                poller: OnceLock::new(),
            }),
        }
    }

    fn spawn_poller(&self, client: SamplingClient, interval: Duration) {
        let state = Arc::downgrade(&self.state);
        let spawned = thread::Builder::new()
            .name("opentelemetry-aws-xray-sampler".to_owned())
            .spawn(move || poll_rules(state, client, interval));
        match spawned {
            Ok(handle) => {
                let _ = self.state.poller.set(handle.thread().clone());
            }
            Err(_error) => {
                #[cfg(feature = "internal-logs")]
                tracing::warn!(error = %_error, "Failed to start AWS X-Ray sampling rules poller");
            }
        }
    }
}

/// Fetches rules every `interval` until the sampler is dropped.
fn poll_rules(state: Weak<State>, client: SamplingClient, interval: Duration) {
    while state.strong_count() > 0 {
        // On failure, keep the previous rules.
        if let Some(rules) = client.get_sampling_rules() {
            let Some(state) = state.upgrade() else {
                return;
            };
            state.set_rules(rules);
        }

        let deadline = Instant::now() + interval;
        while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
            if state.strong_count() == 0 {
                return;
            }
            thread::park_timeout(remaining);
        }
    }
}

impl ShouldSample for RulesSampler {
    fn should_sample(
        &self,
        parent_context: Option<&Context>,
        trace_id: TraceId,
        name: &str,
        span_kind: &SpanKind,
        attributes: &[KeyValue],
        links: &[Link],
    ) -> SamplingResult {
        Sampler::TraceIdRatioBased(self.state.rate(attributes)).should_sample(
            parent_context,
            trace_id,
            name,
            span_kind,
            attributes,
            links,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry::trace::{SpanContext, SpanId, TraceContextExt, TraceFlags, TraceState};
    use opentelemetry_sdk::trace::SamplingDecision;

    fn rules(json: &str) -> Vec<SamplingRule> {
        serde_json::from_str::<rule::GetSamplingRulesResponse>(json)
            .unwrap()
            .into_rules()
    }

    fn decision(
        sampler: &impl ShouldSample,
        parent: Option<&Context>,
        path: &str,
    ) -> SamplingDecision {
        sampler
            .should_sample(
                parent,
                TraceId::from(u128::MAX),
                "span",
                &SpanKind::Server,
                &[KeyValue::new("url.path", path.to_owned())],
                &[],
            )
            .decision
    }

    fn sampler_with_rules(json: &str) -> RulesSampler {
        let sampler = RulesSampler::new(ResourceInfo::default());
        sampler.state.set_rules(rules(json));
        sampler
    }

    const RULES: &str = r#"{"SamplingRuleRecords": [
        {"SamplingRule": {"RuleName": "all-api", "Priority": 1, "FixedRate": 1.0, "URLPath": "/api/*",
            "Host": "*", "HTTPMethod": "*", "ServiceName": "*", "ServiceType": "*", "ResourceARN": "*", "Version": 1}},
        {"SamplingRule": {"RuleName": "Default", "Priority": 10000, "FixedRate": 0.0, "URLPath": "*",
            "Host": "*", "HTTPMethod": "*", "ServiceName": "*", "ServiceType": "*", "ResourceARN": "*", "Version": 1}}
    ]}"#;

    #[test]
    fn uses_fixed_rate_of_first_matching_rule() {
        let sampler = sampler_with_rules(RULES);
        assert_eq!(
            decision(&sampler, None, "/api/orders"),
            SamplingDecision::RecordAndSample
        );
        assert_eq!(decision(&sampler, None, "/health"), SamplingDecision::Drop);
    }

    #[test]
    fn uses_fallback_without_rules() {
        let sampler = RulesSampler::new(ResourceInfo::default());
        // This trace ID is dropped at 5%.
        assert_eq!(
            decision(&sampler, None, "/api/orders"),
            SamplingDecision::Drop
        );
        let low =
            sampler.should_sample(None, TraceId::from(1), "span", &SpanKind::Server, &[], &[]);
        assert_eq!(low.decision, SamplingDecision::RecordAndSample);
    }

    #[test]
    fn uses_fallback_when_rules_expire() {
        let Some(expired) = Instant::now().checked_sub(RULES_TTL) else {
            return; // the monotonic clock started less than an hour ago
        };
        let sampler = sampler_with_rules(RULES);
        sampler
            .state
            .rules
            .write()
            .unwrap()
            .as_mut()
            .unwrap()
            .fetched_at = expired;
        assert_eq!(
            decision(&sampler, None, "/api/orders"),
            SamplingDecision::Drop
        );
    }

    #[test]
    fn follows_parent_decision() {
        let sampler = AwsXrayRemoteSampler {
            root: Sampler::ParentBased(Box::new(sampler_with_rules(RULES))),
        };
        let parent = |flags| {
            Context::new().with_remote_span_context(SpanContext::new(
                TraceId::from(1),
                SpanId::from(1),
                flags,
                true,
                TraceState::default(),
            ))
        };

        assert_eq!(
            decision(&sampler, Some(&parent(TraceFlags::SAMPLED)), "/health"),
            SamplingDecision::RecordAndSample
        );
        assert_eq!(
            decision(
                &sampler,
                Some(&parent(TraceFlags::default())),
                "/api/orders"
            ),
            SamplingDecision::Drop
        );
        assert_eq!(
            decision(&sampler, None, "/api/orders"),
            SamplingDecision::RecordAndSample
        );
    }

    #[test]
    fn fetches_rules_in_background() {
        let (endpoint, server) = client::tests::serve_once(
            "200 OK",
            r#"{"SamplingRuleRecords": [{"SamplingRule": {"RuleName": "all", "Priority": 1, "FixedRate": 1.0,
                "URLPath": "*", "Host": "*", "HTTPMethod": "*", "ServiceName": "*", "ServiceType": "*",
                "ResourceARN": "*", "Version": 1}}]}"#,
        );
        let sampler = AwsXrayRemoteSampler::builder(&Resource::builder_empty().build())
            .with_endpoint(endpoint)
            .build();
        server.join().unwrap();

        // Dropped by the default rate, kept by the fetched rule.
        let deadline = Instant::now() + Duration::from_secs(5);
        while decision(&sampler, None, "/") == SamplingDecision::Drop {
            assert!(Instant::now() < deadline, "rules were not applied");
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn polling_interval_has_minimum() {
        let builder = AwsXrayRemoteSampler::builder(&Resource::builder_empty().build())
            .with_polling_interval(Duration::from_secs(1));
        assert_eq!(builder.polling_interval, MIN_POLLING_INTERVAL);
    }
}
