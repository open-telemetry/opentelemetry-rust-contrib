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
        false
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
