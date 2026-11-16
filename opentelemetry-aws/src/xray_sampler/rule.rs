use std::collections::HashMap;

use opentelemetry::{Key, KeyValue, Value};
use opentelemetry_sdk::Resource;
use opentelemetry_semantic_conventions::attribute as semco;
use serde::Deserialize;

/// Response body of the `GetSamplingRules` API.
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
pub(super) struct GetSamplingRulesResponse {
    sampling_rule_records: Vec<SamplingRuleRecord>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct SamplingRuleRecord {
    sampling_rule: Option<SamplingRule>,
}

impl GetSamplingRulesResponse {
    /// Returns the usable rules, sorted by priority and then by name.
    pub(super) fn into_rules(self) -> Vec<SamplingRule> {
        let mut rules: Vec<_> = self
            .sampling_rule_records
            .into_iter()
            .filter_map(|record| record.sampling_rule)
            .filter(|rule| rule.version == 1 && !rule.rule_name.is_empty())
            .collect();
        rules.sort_by(|a, b| {
            a.priority
                .cmp(&b.priority)
                .then_with(|| a.rule_name.cmp(&b.rule_name))
        });
        rules
    }
}

/// An X-Ray sampling rule, as returned by `GetSamplingRules`.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub(super) struct SamplingRule {
    pub(super) rule_name: String,
    #[serde(default = "default_priority")]
    priority: i64,
    pub(super) fixed_rate: f64,
    host: String,
    #[serde(rename = "HTTPMethod")]
    http_method: String,
    #[serde(rename = "ResourceARN")]
    resource_arn: String,
    service_name: String,
    service_type: String,
    #[serde(rename = "URLPath")]
    url_path: String,
    version: i64,
    attributes: HashMap<String, String>,
}

/// Used when a rule has no priority, so it sorts after the default rule (10000).
fn default_priority() -> i64 {
    10001
}

impl SamplingRule {
    /// Returns whether a span with these attributes, in this resource, matches the rule.
    pub(super) fn matches(&self, resource: &ResourceInfo, attributes: &[KeyValue]) -> bool {
        let attr = |keys: &[&str]| {
            keys.iter()
                .find_map(|key| attributes.iter().find(|kv| kv.key.as_str() == *key))
                .map(|kv| &kv.value)
        };

        let url_path = match attr(&[semco::URL_PATH, "http.target"]) {
            Some(value) => as_str(value),
            None => match attr(&[semco::URL_FULL, "http.url"]) {
                Some(value) => as_str(value).and_then(path_of_url),
                None => Some("/"),
            },
        };
        let arn = match &resource.arn {
            Some(arn) => arn,
            None if resource.is_lambda => attr(&[semco::CLOUD_RESOURCE_ID, "faas.id"])
                .and_then(as_str)
                .unwrap_or(""),
            None => "",
        };

        self.attributes_match(attributes)
            && wildcard_match(&self.url_path, url_path)
            && wildcard_match(
                &self.http_method,
                attr(&[semco::HTTP_REQUEST_METHOD, "http.method"]).and_then(as_str),
            )
            && wildcard_match(
                &self.host,
                attr(&[semco::SERVER_ADDRESS, "http.host"]).and_then(as_str),
            )
            && wildcard_match(&self.service_name, Some(&resource.service_name))
            && wildcard_match(&self.service_type, Some(resource.service_type))
            && wildcard_match(&self.resource_arn, Some(arn))
    }

    fn attributes_match(&self, attributes: &[KeyValue]) -> bool {
        self.attributes.iter().all(|(key, pattern)| {
            attributes
                .iter()
                .find(|kv| kv.key.as_str() == key)
                .is_some_and(|kv| wildcard_match(pattern, as_str(&kv.value)))
        })
    }
}

/// Resource values used for matching, read once when the sampler is built.
#[derive(Debug, Default)]
pub(super) struct ResourceInfo {
    service_name: String,
    service_type: &'static str,
    arn: Option<String>,
    is_lambda: bool,
}

impl ResourceInfo {
    pub(super) fn new(resource: &Resource) -> Self {
        let get = |key: &'static str| {
            resource
                .get_ref(&Key::from_static_str(key))
                .and_then(as_str)
                .map(str::to_owned)
        };
        let cloud_platform = get(semco::CLOUD_PLATFORM).unwrap_or_default();
        let is_lambda = cloud_platform == "aws_lambda";
        let mut arn = get(semco::AWS_ECS_CONTAINER_ARN);
        if arn.is_none() && is_lambda {
            arn = get(semco::CLOUD_RESOURCE_ID).or_else(|| get("faas.id"));
        }

        Self {
            service_name: get(semco::SERVICE_NAME).unwrap_or_default(),
            service_type: service_type(&cloud_platform),
            arn,
            is_lambda,
        }
    }
}

/// Maps `cloud.platform` to the X-Ray service type.
fn service_type(cloud_platform: &str) -> &'static str {
    match cloud_platform {
        "aws_lambda" => "AWS::Lambda::Function",
        "aws_elastic_beanstalk" => "AWS::ElasticBeanstalk::Environment",
        "aws_ec2" => "AWS::EC2::Instance",
        "aws_ecs" => "AWS::ECS::Container",
        "aws_eks" => "AWS::EKS::Container",
        _ => "",
    }
}

fn as_str(value: &Value) -> Option<&str> {
    match value {
        Value::String(s) => Some(s.as_str()),
        _ => None,
    }
}

/// Returns the path of a URL such as `https://host/path?query`, or `None` if it has no scheme.
fn path_of_url(url: &str) -> Option<&str> {
    let rest = &url[url.find("://")? + 3..];
    let rest = &rest[..rest.find(['?', '#']).unwrap_or(rest.len())];
    Some(rest.find('/').map_or("/", |start| &rest[start..]))
}

/// Matches X-Ray wildcards: `*` matches any text and `?` matches one character.
fn wildcard_match(pattern: &str, text: Option<&str>) -> bool {
    if pattern == "*" {
        return true;
    }
    let Some(text) = text else {
        return false;
    };
    if !pattern.contains(['*', '?']) {
        return pattern == text;
    }

    let pattern: Vec<char> = pattern.chars().collect();
    let text: Vec<char> = text.chars().collect();
    let (mut p, mut t) = (0, 0);
    let mut backtrack = None;
    while t < text.len() {
        if p < pattern.len() && (pattern[p] == '?' || pattern[p] == text[t]) {
            p += 1;
            t += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            backtrack = Some((p, t));
            p += 1;
        } else if let Some((star, matched)) = backtrack {
            backtrack = Some((star, matched + 1));
            p = star + 1;
            t = matched + 1;
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(|&c| c == '*')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(json: &str) -> SamplingRule {
        serde_json::from_str(json).unwrap()
    }

    fn resource(attributes: &[(&'static str, &'static str)]) -> ResourceInfo {
        ResourceInfo::new(
            &Resource::builder_empty()
                .with_attributes(attributes.iter().map(|(k, v)| KeyValue::new(*k, *v)))
                .build(),
        )
    }

    #[test]
    fn wildcard() {
        assert!(wildcard_match("*", None));
        assert!(wildcard_match("", Some("")));
        assert!(!wildcard_match("", Some("a")));
        assert!(!wildcard_match("GET", None));
        assert!(wildcard_match("GET", Some("GET")));
        assert!(!wildcard_match("GET", Some("get")));
        assert!(wildcard_match("/api/*", Some("/api/users/1")));
        assert!(wildcard_match("*.example.com", Some("www.example.com")));
        assert!(wildcard_match("v?", Some("v1")));
        assert!(!wildcard_match("v?", Some("v10")));
        assert!(wildcard_match("a*b*c", Some("aXbYbZc")));
        assert!(!wildcard_match("a*b*c", Some("aXbYbZ")));
    }

    #[test]
    fn url_path() {
        assert_eq!(path_of_url("http://host/a/b?x=1"), Some("/a/b"));
        assert_eq!(path_of_url("https://host"), Some("/"));
        assert_eq!(path_of_url("https://host?x=1"), Some("/"));
        assert_eq!(path_of_url("host/a"), None);
    }

    #[test]
    fn parses_filters_and_sorts_rules() {
        let response: GetSamplingRulesResponse = serde_json::from_str(
            r#"{"SamplingRuleRecords": [
                {"SamplingRule": {"RuleName": "Default", "Priority": 10000, "FixedRate": 0.05, "Version": 1}},
                {"SamplingRule": {"RuleName": "b", "Priority": 1, "FixedRate": 1.0, "Version": 1}},
                {"SamplingRule": {"RuleName": "a", "Priority": 1, "FixedRate": 1.0, "Version": 1}},
                {"SamplingRule": {"RuleName": "old", "Priority": 1, "FixedRate": 1.0, "Version": 2}},
                {"SamplingRule": {"Priority": 1, "FixedRate": 1.0, "Version": 1}},
                {}
            ]}"#,
        )
        .unwrap();

        let names: Vec<_> = response
            .into_rules()
            .into_iter()
            .map(|r| r.rule_name)
            .collect();
        assert_eq!(names, ["a", "b", "Default"]);
    }

    #[test]
    fn matches_span_and_resource() {
        let rule = rule(
            r#"{"RuleName": "r", "Host": "*.example.com", "HTTPMethod": "GET", "URLPath": "/api/*",
                "ServiceName": "checkout", "ServiceType": "AWS::EC2::Instance", "ResourceARN": "*",
                "Attributes": {"tier": "gold"}, "Version": 1}"#,
        );
        let ec2 = resource(&[
            (semco::SERVICE_NAME, "checkout"),
            (semco::CLOUD_PLATFORM, "aws_ec2"),
        ]);
        let attributes = [
            KeyValue::new(semco::SERVER_ADDRESS, "www.example.com"),
            KeyValue::new(semco::HTTP_REQUEST_METHOD, "GET"),
            KeyValue::new(semco::URL_PATH, "/api/users"),
            KeyValue::new("tier", "gold"),
        ];
        assert!(rule.matches(&ec2, &attributes));

        assert!(!rule.matches(&ec2, &attributes[..3]));
        let mut post = attributes.clone();
        post[1] = KeyValue::new(semco::HTTP_REQUEST_METHOD, "POST");
        assert!(!rule.matches(&ec2, &post));
        let other = resource(&[(semco::SERVICE_NAME, "other")]);
        assert!(!rule.matches(&other, &attributes));
    }

    #[test]
    fn matches_legacy_attributes_and_full_url() {
        let rule = rule(
            r#"{"RuleName": "r", "Host": "*", "HTTPMethod": "POST", "URLPath": "/orders",
                "ServiceName": "*", "ServiceType": "*", "ResourceARN": "*", "Version": 1}"#,
        );
        let resource = ResourceInfo::default();
        assert!(rule.matches(
            &resource,
            &[
                KeyValue::new("http.method", "POST"),
                KeyValue::new("http.target", "/orders"),
            ],
        ));
        assert!(rule.matches(
            &resource,
            &[
                KeyValue::new("http.method", "POST"),
                KeyValue::new(semco::URL_FULL, "https://host/orders?id=1"),
            ],
        ));
    }

    #[test]
    fn default_url_path_and_lambda_arn() {
        let rule = rule(
            r#"{"RuleName": "r", "Host": "*", "HTTPMethod": "*", "URLPath": "/",
                "ServiceName": "*", "ServiceType": "AWS::Lambda::Function",
                "ResourceARN": "arn:aws:lambda:*", "Version": 1}"#,
        );
        let lambda = resource(&[
            (semco::CLOUD_PLATFORM, "aws_lambda"),
            (
                semco::CLOUD_RESOURCE_ID,
                "arn:aws:lambda:us-east-1:123:function:f",
            ),
        ]);
        assert!(rule.matches(&lambda, &[]));
        assert!(!rule.matches(&ResourceInfo::default(), &[]));
    }
}
