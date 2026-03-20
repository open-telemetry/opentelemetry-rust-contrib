use std::borrow::Cow;

use opentelemetry::Value;

use crate::xray_exporter::translator::{
    attribute_processing::{get_integer, get_str, semconv, SpanAttributeProcessor},
    error::Result,
    AnyDocumentBuilder,
};

use super::ValueBuilder;

/// Builds the SQL database connection URL for X-Ray subsegments.
///
/// Constructs the `sql.url` field by assembling database server, port, and name attributes into a
/// connection URL format. Only applies to subsegments representing database operations.
#[derive(Debug, Default)]
pub(in crate::xray_exporter::translator) struct SqlUrlBuilder<'a> {
    db_server: Option<&'a str>,
    db_port: Option<u16>,
    db_name: Option<&'a str>,
}

impl<'a> SqlUrlBuilder<'a> {
    fn db_server(&mut self, value: &'a Value) -> bool {
        self.db_server = get_str(value);
        self.db_server.is_some()
    }
    fn db_port(&mut self, value: &'a Value) -> bool {
        match get_integer(value) {
            Some(port) => {
                self.db_port = Some(port as u16);
                true
            }
            None => false,
        }
    }
    fn db_name(&mut self, value: &'a Value) -> bool {
        self.db_name = get_str(value);
        self.db_name.is_some()
    }
}

impl<'value> ValueBuilder<'value> for SqlUrlBuilder<'value> {
    fn resolve(self, segment_builder: &mut AnyDocumentBuilder<'value>) -> Result<()> {
        if let Some(db_server) = self.db_server {
            if !db_server.is_empty() {
                if let AnyDocumentBuilder::Subsegment(builder) = segment_builder {
                    let url = match (self.db_port, self.db_name) {
                        (None, None) => Cow::Borrowed(db_server),
                        (None, Some(db_name)) => Cow::Owned(format!("{db_server}/{db_name}")),
                        (Some(port), None) => Cow::Owned(format!("{db_server}:{port}")),
                        (Some(port), Some(db_name)) => {
                            Cow::Owned(format!("{db_server}:{port}/{db_name}"))
                        }
                    };
                    builder.sql().url(url);
                }
            };
        }
        Ok(())
    }
}

impl<'v> SpanAttributeProcessor<'v, 3> for SqlUrlBuilder<'v> {
    const HANDLERS: [(&'static str, fn(&mut Self, &'v Value) -> bool); 3] = [
        (semconv::SERVER_ADDRESS, Self::db_server),
        (semconv::SERVER_PORT, Self::db_port),
        (semconv::DB_NAMESPACE, Self::db_name),
    ];
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xray_exporter::types::{
        Id, SegmentDocumentBuilder, SubsegmentDocumentBuilder, TraceId,
    };
    use opentelemetry::Value;

    /// Finalize a subsegment builder by setting required fields, build it,
    /// and return the JSON string for assertion.
    fn build_subsegment_json(builder: AnyDocumentBuilder<'_>) -> String {
        match builder {
            AnyDocumentBuilder::Subsegment(mut b) => {
                b.name("test-subsegment").unwrap();
                b.id(Id::from(0xABCDu64));
                b.parent_id(Id::from(0x1234u64));
                b.start_time(1_000_000.0);
                b.trace_id(TraceId::new(), true).unwrap();
                b.build().unwrap().to_string()
            }
            _ => panic!("expected Subsegment variant"),
        }
    }

    /// Finalize a segment builder by setting required fields, build it,
    /// and return the JSON string for assertion.
    fn build_segment_json(builder: AnyDocumentBuilder<'_>) -> String {
        match builder {
            AnyDocumentBuilder::Segment(mut b) => {
                b.name("test-segment").unwrap();
                b.id(Id::from(0xABCDu64));
                b.start_time(1_000_000.0);
                b.trace_id(TraceId::new(), true).unwrap();
                b.build().unwrap().to_string()
            }
            _ => panic!("expected Segment variant"),
        }
    }

    /// Extract the sql.url value from the JSON output, or None if not present.
    fn extract_sql_url(json: &str) -> Option<String> {
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        v.get("sql")
            .and_then(|s| s.get("url"))
            .and_then(|u| u.as_str())
            .map(|s| s.to_string())
    }

    #[test]
    fn test_resolve_valid_url_combinations() {
        // Server only → URL is "myhost"
        let server = Value::String("myhost".into());
        let mut builder = SqlUrlBuilder::default();
        builder.db_server(&server);
        let mut doc = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder.resolve(&mut doc).unwrap();
        let json = build_subsegment_json(doc);
        assert_eq!(
            extract_sql_url(&json).as_deref(),
            Some("myhost"),
            "server only should produce bare server URL, got: {json}"
        );

        // Server + port → URL is "myhost:5432"
        let server = Value::String("myhost".into());
        let port = Value::I64(5432);
        let mut builder = SqlUrlBuilder::default();
        builder.db_server(&server);
        builder.db_port(&port);
        let mut doc = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder.resolve(&mut doc).unwrap();
        let json = build_subsegment_json(doc);
        assert_eq!(
            extract_sql_url(&json).as_deref(),
            Some("myhost:5432"),
            "server+port should produce server:port URL, got: {json}"
        );

        // Server + name → URL is "myhost/mydb"
        let server = Value::String("myhost".into());
        let name = Value::String("mydb".into());
        let mut builder = SqlUrlBuilder::default();
        builder.db_server(&server);
        builder.db_name(&name);
        let mut doc = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder.resolve(&mut doc).unwrap();
        let json = build_subsegment_json(doc);
        assert_eq!(
            extract_sql_url(&json).as_deref(),
            Some("myhost/mydb"),
            "server+name should produce server/name URL, got: {json}"
        );

        // Server + port + name → URL is "myhost:5432/mydb"
        let server = Value::String("myhost".into());
        let port = Value::I64(5432);
        let name = Value::String("mydb".into());
        let mut builder = SqlUrlBuilder::default();
        builder.db_server(&server);
        builder.db_port(&port);
        builder.db_name(&name);
        let mut doc = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder.resolve(&mut doc).unwrap();
        let json = build_subsegment_json(doc);
        assert_eq!(
            extract_sql_url(&json).as_deref(),
            Some("myhost:5432/mydb"),
            "server+port+name should produce server:port/name URL, got: {json}"
        );
    }

    #[test]
    fn test_resolve_no_url_set() {
        // No server → no URL set
        let port = Value::I64(5432);
        let name = Value::String("mydb".into());
        let mut builder = SqlUrlBuilder::default();
        builder.db_port(&port);
        builder.db_name(&name);
        let mut doc = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder.resolve(&mut doc).unwrap();
        let json = build_subsegment_json(doc);
        assert!(
            extract_sql_url(&json).is_none(),
            "no server should produce no sql.url, got: {json}"
        );

        // Empty server → no URL set
        let server = Value::String("".into());
        let port = Value::I64(3306);
        let mut builder = SqlUrlBuilder::default();
        builder.db_server(&server);
        builder.db_port(&port);
        let mut doc = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder.resolve(&mut doc).unwrap();
        let json = build_subsegment_json(doc);
        assert!(
            extract_sql_url(&json).is_none(),
            "empty server should produce no sql.url, got: {json}"
        );

        // Segment builder → no URL set (sql.url is subsegment-only)
        let server = Value::String("myhost".into());
        let port = Value::I64(5432);
        let name = Value::String("mydb".into());
        let mut builder = SqlUrlBuilder::default();
        builder.db_server(&server);
        builder.db_port(&port);
        builder.db_name(&name);
        let mut doc = AnyDocumentBuilder::Segment(SegmentDocumentBuilder::default());
        builder.resolve(&mut doc).unwrap();
        let json = build_segment_json(doc);
        assert!(
            extract_sql_url(&json).is_none(),
            "segment builder should not have sql.url, got: {json}"
        );
    }
}
