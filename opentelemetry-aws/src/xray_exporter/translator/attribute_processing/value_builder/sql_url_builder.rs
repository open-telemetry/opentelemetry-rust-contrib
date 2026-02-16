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
