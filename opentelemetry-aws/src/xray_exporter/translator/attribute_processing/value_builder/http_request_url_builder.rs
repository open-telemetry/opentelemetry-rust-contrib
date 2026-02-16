use std::borrow::Cow;

use opentelemetry::Value;

use crate::xray_exporter::translator::{
    attribute_processing::{get_integer, get_str, semconv, SpanAttributeProcessor},
    error::Result,
    AnyDocumentBuilder,
};

use super::ValueBuilder;

/// Builds the HTTP request URL for X-Ray segments and subsegments.
///
/// Constructs the `http.request.url` field by either using the full URL attribute or assembling it
/// from component parts (scheme, domain, port, path, query). The builder intelligently handles
/// server vs. client span kinds to select appropriate address and port attributes. Applies to both
/// segments and subsegments.
#[derive(Debug, Default)]
pub(in crate::xray_exporter::translator) struct HttpRequestUrlBuilder<'a> {
    span_kind_is_server: bool,
    url_full: Option<&'a str>,
    url_scheme: Option<&'a str>,
    url_domain: Option<&'a str>,
    url_port: Option<u16>,
    url_path: Option<&'a str>,
    url_query: Option<&'a str>,
    server_address: Option<&'a str>,
    client_address: Option<&'a str>,
    server_port: Option<u16>,
    client_port: Option<u16>,
}

impl<'a> HttpRequestUrlBuilder<'a> {
    pub fn new(span_kind_is_server: bool) -> Self {
        Self {
            span_kind_is_server,
            ..Default::default()
        }
    }

    fn url_full(&mut self, value: &'a Value) -> bool {
        self.url_full = get_str(value);
        self.url_full.is_some()
    }
    fn url_scheme(&mut self, value: &'a Value) -> bool {
        self.url_scheme = get_str(value);
        self.url_scheme.is_some()
    }
    fn url_domain(&mut self, value: &'a Value) -> bool {
        self.url_domain = get_str(value);
        self.url_domain.is_some()
    }
    fn url_port(&mut self, value: &'a Value) -> bool {
        self.url_port = get_integer(value).map(|port| port as u16);
        self.url_port.is_some()
    }
    fn url_path(&mut self, value: &'a Value) -> bool {
        self.url_path = get_str(value);
        self.url_path.is_some()
    }
    fn url_query(&mut self, value: &'a Value) -> bool {
        self.url_query = get_str(value);
        self.url_query.is_some()
    }
    fn server_address(&mut self, value: &'a Value) -> bool {
        self.server_address = get_str(value);
        self.server_address.is_some()
    }
    fn client_address(&mut self, value: &'a Value) -> bool {
        self.client_address = get_str(value);
        self.client_address.is_some()
    }
    fn server_port(&mut self, value: &'a Value) -> bool {
        self.server_port = get_integer(value).map(|port| port as u16);
        self.server_port.is_some()
    }
    fn client_port(&mut self, value: &'a Value) -> bool {
        self.client_port = get_integer(value).map(|port| port as u16);
        self.client_port.is_some()
    }
}

impl<'value> ValueBuilder<'value> for HttpRequestUrlBuilder<'value> {
    fn resolve(self, segment_builder: &mut AnyDocumentBuilder<'value>) -> Result<()> {
        let url = if let Some(url_full) = self.url_full {
            // Full URL available so no need to assemble
            Some(Cow::Borrowed(url_full))
        } else if let Some(domain) = self.url_domain.or(if self.span_kind_is_server {
            self.server_address
        } else {
            self.client_address
        }) {
            let port = self.url_port.or(if self.span_kind_is_server {
                self.server_port
            } else {
                self.client_port
            });

            let (scheme, need_port, port) = match (self.url_scheme, port) {
                (None, None) => ("http", false, 0),
                (None, Some(port)) => {
                    if port == 80 {
                        ("http", false, port)
                    } else if port == 443 {
                        ("https", false, port)
                    } else {
                        ("http", true, port)
                    }
                }
                (Some(scheme), None) => (scheme, false, 0),
                (Some(scheme), Some(port)) => {
                    if scheme == "https" && port == 443 || scheme == "http" && port == 80 {
                        (scheme, false, port)
                    } else {
                        (scheme, true, port)
                    }
                }
            };
            let port_sep = if need_port { ":" } else { "" };
            let (query, query_sep) = if let Some(query) = self.url_query {
                if query.starts_with('?') {
                    (query, "")
                } else {
                    (query, "?")
                }
            } else {
                ("", "")
            };

            let (path, path_sep) = if let Some(path) = self.url_path {
                if path.starts_with('/') {
                    (path, "")
                } else {
                    (path, "/")
                }
            } else {
                ("", "")
            };
            Some(Cow::Owned(if need_port {
                format!("{scheme}://{domain}{port_sep}{port}{path_sep}{path}{query_sep}{query}")
            } else {
                format!("{scheme}://{domain}{path_sep}{path}{query_sep}{query}")
            }))
        } else {
            None
        };
        if let Some(url) = url {
            match segment_builder {
                AnyDocumentBuilder::Segment(builder) => {
                    builder.http().request.url(url)?;
                }
                AnyDocumentBuilder::Subsegment(builder) => {
                    builder.http().request.url(url)?;
                }
            }
        }
        Ok(())
    }
}

impl<'v> SpanAttributeProcessor<'v, 11> for HttpRequestUrlBuilder<'v> {
    const HANDLERS: [(&'static str, fn(&mut Self, &'v Value) -> bool); 11] = [
        (semconv::URL_FULL, Self::url_full),
        (
            #[allow(deprecated)]
            semconv::HTTP_URL,
            Self::url_full,
        ),
        (semconv::URL_SCHEME, Self::url_scheme),
        (semconv::URL_DOMAIN, Self::url_domain),
        (semconv::URL_PORT, Self::url_port),
        (semconv::URL_PATH, Self::url_path),
        (semconv::URL_QUERY, Self::url_query),
        (semconv::SERVER_ADDRESS, Self::server_address),
        (semconv::CLIENT_ADDRESS, Self::client_address),
        (semconv::SERVER_PORT, Self::server_port),
        (semconv::CLIENT_PORT, Self::client_port),
    ];
}
