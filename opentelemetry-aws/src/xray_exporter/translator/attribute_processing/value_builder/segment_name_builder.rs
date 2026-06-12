use opentelemetry::Value;

use crate::xray_exporter::translator::{
    attribute_processing::{get_str, semconv, SpanAttributeProcessor},
    error::Result,
    AnyDocumentBuilder,
};

use super::ValueBuilder;

/// Builds the segment or subsegment name according to X-Ray naming conventions.
///
/// Constructs the `name` field by selecting from multiple span attributes in priority order:
/// peer.service, aws.service, rpc.service (for AWS API), db.service, service.name (server spans only),
/// or finally the span name. This naming strategy helps identify the remote service or operation being traced.
/// Applies to both segments and subsegments.
#[derive(Debug)]
pub(in crate::xray_exporter::translator) struct SegmentNameBuilder<'a> {
    span_name: &'a str,
    span_kind_is_server: bool,
    rpc_system_is_aws_api: bool,
    peer_service: Option<&'a str>,
    aws_service: Option<&'a str>,
    rpc_service: Option<&'a str>,
    db_service: Option<&'a str>,
    service_name: Option<&'a str>,
}

impl<'a> SegmentNameBuilder<'a> {
    pub fn new(span_name: &'a str, span_kind_is_server: bool) -> Self {
        Self {
            span_name,
            span_kind_is_server,
            rpc_system_is_aws_api: false,
            peer_service: None,
            aws_service: None,
            rpc_service: None,
            db_service: None,
            service_name: None,
        }
    }

    fn rpc_system_is_aws_api(&mut self, value: &'a Value) -> bool {
        if value.as_str() == "aws-api" {
            self.rpc_system_is_aws_api = true;
        }
        self.rpc_system_is_aws_api
    }
    fn peer_service(&mut self, value: &'a Value) -> bool {
        self.peer_service = get_str(value);
        false
    }
    fn aws_service(&mut self, value: &'a Value) -> bool {
        self.aws_service = get_str(value);
        false
    }
    fn rpc_service(&mut self, value: &'a Value) -> bool {
        self.rpc_service = get_str(value);
        false
    }
    fn db_service(&mut self, value: &'a Value) -> bool {
        self.db_service = get_str(value);
        false
    }
    fn service_name(&mut self, value: &'a Value) -> bool {
        self.service_name = get_str(value);
        false
    }

    fn name(&self) -> &'a str {
        // Name field is set to peer.service if not empty
        if let Some(peer_service) = self.peer_service {
            return peer_service;
        }

        // If peer.service is empty and aws.service attribute key is not empty, name is set to aws.service
        if let Some(aws_service) = self.aws_service {
            return aws_service;
        }

        // If the rpc-system is AWS and we have a rpc.service, use it
        if self.rpc_system_is_aws_api {
            if let Some(rpc_service) = self.rpc_service {
                return rpc_service;
            }
        }

        // If aws.service is empty and db.service attribute key is not empty, name is set to db.service
        if let Some(db_service) = self.db_service {
            return db_service;
        }

        // If none of these attribute keys has a value, and span.kind = "Server", then name is set to value of service.name attribute key
        if self.span_kind_is_server {
            if let Some(service_name) = self.service_name {
                return service_name;
            }
        }

        // If none of the prior conditions are met, name is set to the name of the span
        self.span_name
    }
}

impl<'value> ValueBuilder<'value> for SegmentNameBuilder<'value> {
    fn resolve(self, segment_builder: &mut AnyDocumentBuilder<'value>) -> Result<()> {
        let name = self.name();
        match segment_builder {
            AnyDocumentBuilder::Segment(builder) => {
                builder.name(name)?;
            }
            AnyDocumentBuilder::Subsegment(builder) => {
                builder.name(name)?;
            }
        }
        Ok(())
    }
}

impl<'v> SpanAttributeProcessor<'v, 6> for SegmentNameBuilder<'v> {
    const HANDLERS: [(&'static str, fn(&mut Self, &'v Value) -> bool); 6] = [
        (semconv::RPC_SYSTEM, Self::rpc_system_is_aws_api),
        (semconv::PEER_SERVICE, Self::peer_service),
        (semconv::AWS_SERVICE, Self::aws_service),
        (semconv::RPC_SERVICE, Self::rpc_service),
        (semconv::DB_SERVICE, Self::db_service),
        (semconv::SERVICE_NAME, Self::service_name),
    ];
}

#[cfg(test)]
mod tests {
    use super::*;

    // Tests for name() priority resolution — valid priority levels

    #[test]
    fn name_priority_peer_service_is_highest() {
        // peer.service takes priority over all other attributes
        let peer = Value::String("my-peer-service".into());
        let aws = Value::String("my-aws-service".into());
        let rpc_sys = Value::String("aws-api".into());
        let rpc_svc = Value::String("my-rpc-service".into());
        let db = Value::String("my-db-service".into());
        let svc = Value::String("my-service-name".into());

        let mut builder = SegmentNameBuilder::new("fallback-span", true);
        builder.peer_service(&peer);
        builder.aws_service(&aws);
        builder.rpc_system_is_aws_api(&rpc_sys);
        builder.rpc_service(&rpc_svc);
        builder.db_service(&db);
        builder.service_name(&svc);

        assert_eq!(builder.name(), "my-peer-service");
    }

    #[test]
    fn name_priority_aws_service_second() {
        // aws.service is used when peer.service is absent
        let aws = Value::String("my-aws-service".into());
        let rpc_sys = Value::String("aws-api".into());
        let rpc_svc = Value::String("my-rpc-service".into());
        let db = Value::String("my-db-service".into());
        let svc = Value::String("my-service-name".into());

        let mut builder = SegmentNameBuilder::new("fallback-span", true);
        builder.aws_service(&aws);
        builder.rpc_system_is_aws_api(&rpc_sys);
        builder.rpc_service(&rpc_svc);
        builder.db_service(&db);
        builder.service_name(&svc);

        assert_eq!(builder.name(), "my-aws-service");
    }

    #[test]
    fn name_priority_rpc_service_with_aws_api_third() {
        // rpc.service is used when rpc.system=aws-api and peer/aws are absent
        let rpc_sys = Value::String("aws-api".into());
        let rpc_svc = Value::String("my-rpc-service".into());
        let db = Value::String("my-db-service".into());
        let svc = Value::String("my-service-name".into());

        let mut builder = SegmentNameBuilder::new("fallback-span", true);
        builder.rpc_system_is_aws_api(&rpc_sys);
        builder.rpc_service(&rpc_svc);
        builder.db_service(&db);
        builder.service_name(&svc);

        assert_eq!(builder.name(), "my-rpc-service");
    }

    #[test]
    fn name_priority_db_service_fourth() {
        // db.service is used when peer, aws, and rpc (aws-api) are absent
        let db = Value::String("my-db-service".into());
        let svc = Value::String("my-service-name".into());

        let mut builder = SegmentNameBuilder::new("fallback-span", true);
        builder.db_service(&db);
        builder.service_name(&svc);

        assert_eq!(builder.name(), "my-db-service");
    }

    #[test]
    fn name_priority_service_name_for_server_spans_fifth() {
        // service.name is used for server spans when higher-priority attrs are absent
        let svc = Value::String("my-service-name".into());

        let mut builder = SegmentNameBuilder::new("fallback-span", true);
        builder.service_name(&svc);

        assert_eq!(builder.name(), "my-service-name");
    }

    #[test]
    fn name_priority_span_name_fallback() {
        // span_name is the final fallback when nothing else is set
        let builder = SegmentNameBuilder::new("fallback-span", false);
        assert_eq!(builder.name(), "fallback-span");

        // Also falls back for server spans with no service.name
        let builder_server = SegmentNameBuilder::new("server-fallback", true);
        assert_eq!(builder_server.name(), "server-fallback");
    }

    // Tests for name() edge cases — conditions that cause fallthrough

    #[test]
    fn name_rpc_service_without_aws_api_falls_through() {
        // rpc.service is set but rpc.system is NOT aws-api → rpc.service is skipped
        let rpc_sys = Value::String("grpc".into());
        let rpc_svc = Value::String("my-rpc-service".into());

        let mut builder = SegmentNameBuilder::new("fallback-span", false);
        builder.rpc_system_is_aws_api(&rpc_sys);
        builder.rpc_service(&rpc_svc);

        // rpc.system != "aws-api", so rpc.service is not used → falls to span_name
        assert_eq!(builder.name(), "fallback-span");
    }

    #[test]
    fn name_rpc_service_without_aws_api_falls_to_db_service() {
        // rpc.service set, rpc.system != aws-api, but db.service is available
        let rpc_sys = Value::String("grpc".into());
        let rpc_svc = Value::String("my-rpc-service".into());
        let db = Value::String("my-db-service".into());

        let mut builder = SegmentNameBuilder::new("fallback-span", false);
        builder.rpc_system_is_aws_api(&rpc_sys);
        builder.rpc_service(&rpc_svc);
        builder.db_service(&db);

        assert_eq!(builder.name(), "my-db-service");
    }

    #[test]
    fn name_service_name_ignored_for_non_server_spans() {
        // service.name is set but span_kind_is_server=false → falls to span_name
        let svc = Value::String("my-service-name".into());

        let mut builder = SegmentNameBuilder::new("fallback-span", false);
        builder.service_name(&svc);

        assert_eq!(builder.name(), "fallback-span");
    }

    #[test]
    fn name_non_string_values_are_ignored() {
        // Non-string Value types return None from get_str, so they're skipped
        let int_val = Value::I64(42);
        let bool_val = Value::Bool(true);

        let mut builder = SegmentNameBuilder::new("fallback-span", true);
        builder.peer_service(&int_val);
        builder.aws_service(&bool_val);

        // Both are non-string, so they're None → falls to span_name
        assert_eq!(builder.name(), "fallback-span");
    }

    // Tests for resolve() — integration with AnyDocumentBuilder

    #[test]
    fn resolve_sets_name_on_segment() {
        use crate::xray_exporter::types::{Id, SegmentDocumentBuilder, TraceId};

        let peer = Value::String("resolved-peer".into());
        let mut builder = SegmentNameBuilder::new("fallback", false);
        builder.peer_service(&peer);

        let mut doc_builder = AnyDocumentBuilder::Segment(SegmentDocumentBuilder::default());
        builder.resolve(&mut doc_builder).unwrap();

        // Set required fields and build to verify name in JSON
        match doc_builder {
            AnyDocumentBuilder::Segment(mut b) => {
                b.id(Id::from(0xABCDu64));
                b.start_time(1_000_000.0);
                b.trace_id(TraceId::new(), true).unwrap();
                let doc = b.build().unwrap();
                let json = doc.to_string();
                assert!(
                    json.contains("\"name\":\"resolved-peer\""),
                    "expected name 'resolved-peer' in JSON, got: {json}"
                );
            }
            _ => panic!("expected Segment variant"),
        }
    }

    #[test]
    fn resolve_sets_name_on_subsegment() {
        use crate::xray_exporter::types::{Id, SubsegmentDocumentBuilder, TraceId};

        let aws = Value::String("resolved-aws".into());
        let mut builder = SegmentNameBuilder::new("fallback", false);
        builder.aws_service(&aws);

        let mut doc_builder = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder.resolve(&mut doc_builder).unwrap();

        // Set required fields and build to verify name in JSON
        match doc_builder {
            AnyDocumentBuilder::Subsegment(mut b) => {
                b.id(Id::from(0xABCDu64));
                b.parent_id(Id::from(0x1234u64));
                b.start_time(1_000_000.0);
                b.trace_id(TraceId::new(), true).unwrap();
                let doc = b.build().unwrap();
                let json = doc.to_string();
                assert!(
                    json.contains("\"name\":\"resolved-aws\""),
                    "expected name 'resolved-aws' in JSON, got: {json}"
                );
            }
            _ => panic!("expected Subsegment variant"),
        }
    }

    #[test]
    fn resolve_uses_fallback_span_name_when_no_attributes() {
        use crate::xray_exporter::types::{Id, SubsegmentDocumentBuilder, TraceId};

        let builder = SegmentNameBuilder::new("my-span-operation", false);

        let mut doc_builder = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder.resolve(&mut doc_builder).unwrap();

        match doc_builder {
            AnyDocumentBuilder::Subsegment(mut b) => {
                b.id(Id::from(0xABCDu64));
                b.parent_id(Id::from(0x1234u64));
                b.start_time(1_000_000.0);
                b.trace_id(TraceId::new(), true).unwrap();
                let doc = b.build().unwrap();
                let json = doc.to_string();
                assert!(
                    json.contains("\"name\":\"my-span-operation\""),
                    "expected fallback name in JSON, got: {json}"
                );
            }
            _ => panic!("expected Subsegment variant"),
        }
    }

    // Tests for handler methods

    #[test]
    fn handler_rpc_system_is_aws_api_returns_true_only_for_aws_api() {
        // "aws-api" sets the flag and returns true
        let aws_api = Value::String("aws-api".into());
        let mut builder = SegmentNameBuilder::new("span", false);
        assert!(builder.rpc_system_is_aws_api(&aws_api));
        assert!(builder.rpc_system_is_aws_api);

        // Other values don't set the flag
        let grpc = Value::String("grpc".into());
        let mut builder2 = SegmentNameBuilder::new("span", false);
        assert!(!builder2.rpc_system_is_aws_api(&grpc));
        assert!(!builder2.rpc_system_is_aws_api);
    }

    #[test]
    fn handler_rpc_system_sticky_flag() {
        // Once set to true, the flag stays true even if called again with non-aws-api
        let aws_api = Value::String("aws-api".into());
        let grpc = Value::String("grpc".into());

        let mut builder = SegmentNameBuilder::new("span", false);
        builder.rpc_system_is_aws_api(&aws_api);
        assert!(builder.rpc_system_is_aws_api);

        // Calling again with a different value doesn't unset the flag
        let result = builder.rpc_system_is_aws_api(&grpc);
        assert!(result); // returns current flag value (true)
        assert!(builder.rpc_system_is_aws_api);
    }

    #[test]
    fn handler_methods_return_false() {
        // All attribute-setting handlers return false (they never "consume" the attribute)
        let value = Value::String("test-value".into());
        let mut builder = SegmentNameBuilder::new("span", false);

        assert!(!builder.peer_service(&value));
        assert!(!builder.aws_service(&value));
        assert!(!builder.rpc_service(&value));
        assert!(!builder.db_service(&value));
        assert!(!builder.service_name(&value));
    }

    #[test]
    fn handlers_array_maps_correct_semconv_keys() {
        // Verify the HANDLERS array maps the expected attribute keys
        let keys: Vec<&str> = SegmentNameBuilder::HANDLERS
            .iter()
            .map(|(key, _)| *key)
            .collect();

        assert_eq!(keys[0], "rpc.system");
        assert_eq!(keys[1], "peer.service");
        assert_eq!(keys[2], "aws.service");
        assert_eq!(keys[3], "rpc.service");
        assert_eq!(keys[4], "db.service");
        assert_eq!(keys[5], "service.name");
        assert_eq!(keys.len(), 6);
    }
}
