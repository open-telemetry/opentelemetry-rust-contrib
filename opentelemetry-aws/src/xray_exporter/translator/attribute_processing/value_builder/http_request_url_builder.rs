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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xray_exporter::translator::AnyDocumentBuilder;
    use crate::xray_exporter::types::{Id, SubsegmentDocumentBuilder, TraceId};
    use opentelemetry::Value;

    /// Finalize a subsegment builder by setting required fields, build it,
    /// and return the JSON string for assertion.
    fn build_json(builder: AnyDocumentBuilder<'_>) -> String {
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

    /// Extract the URL value from the JSON output, or None if not present.
    fn extract_url(json: &str) -> Option<String> {
        // The JSON contains "url":"<value>" inside the http.request object.
        // Use serde_json to parse reliably.
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        v.get("http")
            .and_then(|h| h.get("request"))
            .and_then(|r| r.get("url"))
            .and_then(|u| u.as_str())
            .map(|s| s.to_string())
    }

    // ---------------------------------------------------------------
    // Tests for resolve(): url_full shortcut
    // ---------------------------------------------------------------

    #[test]
    fn resolve_url_full_uses_value_directly() {
        // When url_full is set, it should be used as-is regardless of other fields
        let full = Value::String("https://example.com/full?q=1".into());
        let mut builder = HttpRequestUrlBuilder::new(false);
        builder.url_full(&full);

        let mut doc = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder.resolve(&mut doc).unwrap();
        let json = build_json(doc);
        assert_eq!(
            extract_url(&json).as_deref(),
            Some("https://example.com/full?q=1"),
            "url_full should be used verbatim, got: {json}"
        );

        // url_full takes priority even when component parts are also set
        let full2 = Value::String("http://override.test/path".into());
        let domain = Value::String("ignored.com".into());
        let scheme = Value::String("https".into());
        let mut builder2 = HttpRequestUrlBuilder::new(true);
        builder2.url_full(&full2);
        builder2.url_domain(&domain);
        builder2.url_scheme(&scheme);

        let mut doc2 = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder2.resolve(&mut doc2).unwrap();
        let json2 = build_json(doc2);
        assert_eq!(
            extract_url(&json2).as_deref(),
            Some("http://override.test/path"),
            "url_full should take priority over component parts, got: {json2}"
        );
    }

    // ---------------------------------------------------------------
    // Tests for resolve(): no domain → no URL
    // ---------------------------------------------------------------

    #[test]
    fn resolve_no_domain_produces_no_url() {
        // No domain, no server_address, no client_address → no URL set
        let mut builder = HttpRequestUrlBuilder::new(false);
        let scheme = Value::String("https".into());
        builder.url_scheme(&scheme);

        let mut doc = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder.resolve(&mut doc).unwrap();
        let json = build_json(doc);
        assert!(
            extract_url(&json).is_none(),
            "no domain should produce no URL, got: {json}"
        );

        // Also for server span with no addresses
        let builder2 = HttpRequestUrlBuilder::new(true);
        let mut doc2 = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder2.resolve(&mut doc2).unwrap();
        let json2 = build_json(doc2);
        assert!(
            extract_url(&json2).is_none(),
            "empty builder should produce no URL, got: {json2}"
        );
    }

    // ---------------------------------------------------------------
    // Tests for resolve(): basic URL assembly from parts
    // ---------------------------------------------------------------

    #[test]
    fn resolve_assembles_url_from_parts_valid() {
        // Domain + scheme only → minimal URL
        let domain = Value::String("example.com".into());
        let scheme = Value::String("https".into());
        let mut builder = HttpRequestUrlBuilder::new(false);
        builder.url_domain(&domain);
        builder.url_scheme(&scheme);

        let mut doc = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder.resolve(&mut doc).unwrap();
        let json = build_json(doc);
        assert_eq!(
            extract_url(&json).as_deref(),
            Some("https://example.com"),
            "scheme + domain should produce basic URL, got: {json}"
        );

        // Domain + scheme + path + query
        let domain2 = Value::String("api.test.io".into());
        let scheme2 = Value::String("http".into());
        let path2 = Value::String("/v2/resource".into());
        let query2 = Value::String("page=1&size=10".into());
        let mut builder2 = HttpRequestUrlBuilder::new(false);
        builder2.url_domain(&domain2);
        builder2.url_scheme(&scheme2);
        builder2.url_path(&path2);
        builder2.url_query(&query2);

        let mut doc2 = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder2.resolve(&mut doc2).unwrap();
        let json2 = build_json(doc2);
        assert_eq!(
            extract_url(&json2).as_deref(),
            Some("http://api.test.io/v2/resource?page=1&size=10"),
            "full assembly should work, got: {json2}"
        );

        // Domain + scheme + non-default port + path + query
        let domain3 = Value::String("myhost.local".into());
        let scheme3 = Value::String("http".into());
        let port3 = Value::I64(9090);
        let path3 = Value::String("/api".into());
        let query3 = Value::String("?debug=true".into());
        let mut builder3 = HttpRequestUrlBuilder::new(false);
        builder3.url_domain(&domain3);
        builder3.url_scheme(&scheme3);
        builder3.url_port(&port3);
        builder3.url_path(&path3);
        builder3.url_query(&query3);

        let mut doc3 = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder3.resolve(&mut doc3).unwrap();
        let json3 = build_json(doc3);
        assert_eq!(
            extract_url(&json3).as_deref(),
            Some("http://myhost.local:9090/api?debug=true"),
            "non-default port should be included, got: {json3}"
        );
    }

    // ---------------------------------------------------------------
    // Tests for resolve(): default scheme inference from port
    // ---------------------------------------------------------------

    #[test]
    fn resolve_infers_scheme_from_port_valid() {
        // No scheme, port=80 → defaults to "http", port omitted
        let domain = Value::String("example.com".into());
        let port80 = Value::I64(80);
        let mut builder = HttpRequestUrlBuilder::new(false);
        builder.url_domain(&domain);
        builder.url_port(&port80);

        let mut doc = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder.resolve(&mut doc).unwrap();
        let json = build_json(doc);
        assert_eq!(
            extract_url(&json).as_deref(),
            Some("http://example.com"),
            "port 80 with no scheme → http, port omitted, got: {json}"
        );

        // No scheme, port=443 → defaults to "https", port omitted
        let domain2 = Value::String("secure.example.com".into());
        let port443 = Value::I64(443);
        let mut builder2 = HttpRequestUrlBuilder::new(false);
        builder2.url_domain(&domain2);
        builder2.url_port(&port443);

        let mut doc2 = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder2.resolve(&mut doc2).unwrap();
        let json2 = build_json(doc2);
        assert_eq!(
            extract_url(&json2).as_deref(),
            Some("https://secure.example.com"),
            "port 443 with no scheme → https, port omitted, got: {json2}"
        );

        // No scheme, port=8080 → defaults to "http", port included
        let domain3 = Value::String("dev.example.com".into());
        let port8080 = Value::I64(8080);
        let mut builder3 = HttpRequestUrlBuilder::new(false);
        builder3.url_domain(&domain3);
        builder3.url_port(&port8080);

        let mut doc3 = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder3.resolve(&mut doc3).unwrap();
        let json3 = build_json(doc3);
        assert_eq!(
            extract_url(&json3).as_deref(),
            Some("http://dev.example.com:8080"),
            "port 8080 with no scheme → http, port included, got: {json3}"
        );

        // No scheme, no port → defaults to "http", no port
        let domain4 = Value::String("bare.example.com".into());
        let mut builder4 = HttpRequestUrlBuilder::new(false);
        builder4.url_domain(&domain4);

        let mut doc4 = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder4.resolve(&mut doc4).unwrap();
        let json4 = build_json(doc4);
        assert_eq!(
            extract_url(&json4).as_deref(),
            Some("http://bare.example.com"),
            "no scheme, no port → http default, got: {json4}"
        );
    }

    // ---------------------------------------------------------------
    // Tests for resolve(): default port omission with explicit scheme
    // ---------------------------------------------------------------

    #[test]
    fn resolve_omits_default_port_for_scheme() {
        // scheme=https, port=443 → port omitted (default for https)
        let domain = Value::String("example.com".into());
        let scheme = Value::String("https".into());
        let port443 = Value::I64(443);
        let mut builder = HttpRequestUrlBuilder::new(false);
        builder.url_domain(&domain);
        builder.url_scheme(&scheme);
        builder.url_port(&port443);

        let mut doc = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder.resolve(&mut doc).unwrap();
        let json = build_json(doc);
        assert_eq!(
            extract_url(&json).as_deref(),
            Some("https://example.com"),
            "https + port 443 → port omitted, got: {json}"
        );

        // scheme=http, port=80 → port omitted (default for http)
        let domain2 = Value::String("example.com".into());
        let scheme2 = Value::String("http".into());
        let port80 = Value::I64(80);
        let mut builder2 = HttpRequestUrlBuilder::new(false);
        builder2.url_domain(&domain2);
        builder2.url_scheme(&scheme2);
        builder2.url_port(&port80);

        let mut doc2 = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder2.resolve(&mut doc2).unwrap();
        let json2 = build_json(doc2);
        assert_eq!(
            extract_url(&json2).as_deref(),
            Some("http://example.com"),
            "http + port 80 → port omitted, got: {json2}"
        );

        // scheme=https, port=8443 → port included (non-default)
        let domain3 = Value::String("example.com".into());
        let scheme3 = Value::String("https".into());
        let port8443 = Value::I64(8443);
        let mut builder3 = HttpRequestUrlBuilder::new(false);
        builder3.url_domain(&domain3);
        builder3.url_scheme(&scheme3);
        builder3.url_port(&port8443);

        let mut doc3 = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder3.resolve(&mut doc3).unwrap();
        let json3 = build_json(doc3);
        assert_eq!(
            extract_url(&json3).as_deref(),
            Some("https://example.com:8443"),
            "https + port 8443 → port included, got: {json3}"
        );

        // scheme=http, port=3000 → port included (non-default)
        let domain4 = Value::String("localhost".into());
        let scheme4 = Value::String("http".into());
        let port3000 = Value::I64(3000);
        let mut builder4 = HttpRequestUrlBuilder::new(false);
        builder4.url_domain(&domain4);
        builder4.url_scheme(&scheme4);
        builder4.url_port(&port3000);

        let mut doc4 = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder4.resolve(&mut doc4).unwrap();
        let json4 = build_json(doc4);
        assert_eq!(
            extract_url(&json4).as_deref(),
            Some("http://localhost:3000"),
            "http + port 3000 → port included, got: {json4}"
        );

        // scheme with no port → no port in URL
        let domain5 = Value::String("example.com".into());
        let scheme5 = Value::String("ftp".into());
        let mut builder5 = HttpRequestUrlBuilder::new(false);
        builder5.url_domain(&domain5);
        builder5.url_scheme(&scheme5);

        let mut doc5 = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder5.resolve(&mut doc5).unwrap();
        let json5 = build_json(doc5);
        assert_eq!(
            extract_url(&json5).as_deref(),
            Some("ftp://example.com"),
            "custom scheme with no port → no port, got: {json5}"
        );
    }

    // ---------------------------------------------------------------
    // Tests for resolve(): path separator handling
    // ---------------------------------------------------------------

    #[test]
    fn resolve_path_separator_handling() {
        let domain = Value::String("example.com".into());
        let scheme = Value::String("http".into());

        // Path without leading '/' → separator added
        let path_no_slash = Value::String("api/v1/items".into());
        let mut builder = HttpRequestUrlBuilder::new(false);
        builder.url_domain(&domain);
        builder.url_scheme(&scheme);
        builder.url_path(&path_no_slash);

        let mut doc = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder.resolve(&mut doc).unwrap();
        let json = build_json(doc);
        assert_eq!(
            extract_url(&json).as_deref(),
            Some("http://example.com/api/v1/items"),
            "path without leading / should get separator, got: {json}"
        );

        // Path with leading '/' → no extra separator
        let path_with_slash = Value::String("/api/v1/items".into());
        let domain2 = Value::String("example.com".into());
        let scheme2 = Value::String("http".into());
        let mut builder2 = HttpRequestUrlBuilder::new(false);
        builder2.url_domain(&domain2);
        builder2.url_scheme(&scheme2);
        builder2.url_path(&path_with_slash);

        let mut doc2 = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder2.resolve(&mut doc2).unwrap();
        let json2 = build_json(doc2);
        assert_eq!(
            extract_url(&json2).as_deref(),
            Some("http://example.com/api/v1/items"),
            "path with leading / should not get extra separator, got: {json2}"
        );
    }

    // ---------------------------------------------------------------
    // Tests for resolve(): query separator handling
    // ---------------------------------------------------------------

    #[test]
    fn resolve_query_separator_handling() {
        let domain = Value::String("example.com".into());
        let scheme = Value::String("http".into());

        // Query without leading '?' → separator added
        let query_no_qmark = Value::String("key=value&foo=bar".into());
        let mut builder = HttpRequestUrlBuilder::new(false);
        builder.url_domain(&domain);
        builder.url_scheme(&scheme);
        builder.url_query(&query_no_qmark);

        let mut doc = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder.resolve(&mut doc).unwrap();
        let json = build_json(doc);
        assert_eq!(
            extract_url(&json).as_deref(),
            Some("http://example.com?key=value&foo=bar"),
            "query without leading ? should get separator, got: {json}"
        );

        // Query with leading '?' → no extra separator
        let query_with_qmark = Value::String("?key=value&foo=bar".into());
        let domain2 = Value::String("example.com".into());
        let scheme2 = Value::String("http".into());
        let mut builder2 = HttpRequestUrlBuilder::new(false);
        builder2.url_domain(&domain2);
        builder2.url_scheme(&scheme2);
        builder2.url_query(&query_with_qmark);

        let mut doc2 = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder2.resolve(&mut doc2).unwrap();
        let json2 = build_json(doc2);
        assert_eq!(
            extract_url(&json2).as_deref(),
            Some("http://example.com?key=value&foo=bar"),
            "query with leading ? should not get extra separator, got: {json2}"
        );
    }

    // ---------------------------------------------------------------
    // Tests for resolve(): server span fallback (server_address/server_port)
    // ---------------------------------------------------------------

    #[test]
    fn resolve_server_span_uses_server_address_fallback() {
        // Server span: no url_domain → falls back to server_address
        let server_addr = Value::String("backend.internal".into());
        let server_p = Value::I64(9090);
        let scheme = Value::String("http".into());
        let path = Value::String("/health".into());
        let mut builder = HttpRequestUrlBuilder::new(true); // server span
        builder.url_scheme(&scheme);
        builder.server_address(&server_addr);
        builder.server_port(&server_p);
        builder.url_path(&path);

        let mut doc = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder.resolve(&mut doc).unwrap();
        let json = build_json(doc);
        assert_eq!(
            extract_url(&json).as_deref(),
            Some("http://backend.internal:9090/health"),
            "server span should fall back to server_address/server_port, got: {json}"
        );

        // Server span: url_domain is set → uses url_domain, not server_address
        let domain = Value::String("primary.host".into());
        let server_addr2 = Value::String("fallback.host".into());
        let scheme2 = Value::String("https".into());
        let mut builder2 = HttpRequestUrlBuilder::new(true);
        builder2.url_domain(&domain);
        builder2.url_scheme(&scheme2);
        builder2.server_address(&server_addr2);

        let mut doc2 = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder2.resolve(&mut doc2).unwrap();
        let json2 = build_json(doc2);
        assert_eq!(
            extract_url(&json2).as_deref(),
            Some("https://primary.host"),
            "url_domain should take priority over server_address, got: {json2}"
        );

        // Server span: server_address only (no port, no scheme) → http default
        let server_addr3 = Value::String("simple.server".into());
        let mut builder3 = HttpRequestUrlBuilder::new(true);
        builder3.server_address(&server_addr3);

        let mut doc3 = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder3.resolve(&mut doc3).unwrap();
        let json3 = build_json(doc3);
        assert_eq!(
            extract_url(&json3).as_deref(),
            Some("http://simple.server"),
            "server_address alone should produce http URL, got: {json3}"
        );
    }

    // ---------------------------------------------------------------
    // Tests for resolve(): non-server span fallback (client_address/client_port)
    // ---------------------------------------------------------------

    #[test]
    fn resolve_client_span_uses_client_address_fallback() {
        // Non-server span: no url_domain → falls back to client_address
        let client_addr = Value::String("client.host".into());
        let client_p = Value::I64(8443);
        let scheme = Value::String("https".into());
        let mut builder = HttpRequestUrlBuilder::new(false); // non-server span
        builder.url_scheme(&scheme);
        builder.client_address(&client_addr);
        builder.client_port(&client_p);

        let mut doc = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder.resolve(&mut doc).unwrap();
        let json = build_json(doc);
        assert_eq!(
            extract_url(&json).as_deref(),
            Some("https://client.host:8443"),
            "non-server span should fall back to client_address/client_port, got: {json}"
        );

        // Non-server span: server_address is set but should NOT be used as fallback
        let server_addr = Value::String("server.host".into());
        let mut builder2 = HttpRequestUrlBuilder::new(false);
        builder2.server_address(&server_addr);

        let mut doc2 = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder2.resolve(&mut doc2).unwrap();
        let json2 = build_json(doc2);
        assert!(
            extract_url(&json2).is_none(),
            "non-server span should NOT use server_address as fallback, got: {json2}"
        );

        // Non-server span: client_address only (no port, no scheme) → http default
        let client_addr3 = Value::String("requester.local".into());
        let mut builder3 = HttpRequestUrlBuilder::new(false);
        builder3.client_address(&client_addr3);

        let mut doc3 = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder3.resolve(&mut doc3).unwrap();
        let json3 = build_json(doc3);
        assert_eq!(
            extract_url(&json3).as_deref(),
            Some("http://requester.local"),
            "client_address alone should produce http URL, got: {json3}"
        );
    }

    // ---------------------------------------------------------------
    // Tests for resolve(): server span ignores client_address
    // ---------------------------------------------------------------

    #[test]
    fn resolve_server_span_ignores_client_address() {
        // Server span with only client_address → no URL (client_address not used for server spans)
        let client_addr = Value::String("client.only".into());
        let mut builder = HttpRequestUrlBuilder::new(true); // server span
        builder.client_address(&client_addr);

        let mut doc = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder.resolve(&mut doc).unwrap();
        let json = build_json(doc);
        assert!(
            extract_url(&json).is_none(),
            "server span should NOT use client_address as domain fallback, got: {json}"
        );
    }

    // ---------------------------------------------------------------
    // Tests for resolve(): port fallback from server_port/client_port
    // ---------------------------------------------------------------

    #[test]
    fn resolve_port_fallback_from_address_port() {
        // Server span: url_domain set, no url_port → falls back to server_port
        let domain = Value::String("example.com".into());
        let scheme = Value::String("http".into());
        let server_p = Value::I64(3000);
        let mut builder = HttpRequestUrlBuilder::new(true);
        builder.url_domain(&domain);
        builder.url_scheme(&scheme);
        builder.server_port(&server_p);

        let mut doc = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder.resolve(&mut doc).unwrap();
        let json = build_json(doc);
        assert_eq!(
            extract_url(&json).as_deref(),
            Some("http://example.com:3000"),
            "server span should fall back to server_port, got: {json}"
        );

        // Non-server span: url_domain set, no url_port → falls back to client_port
        let domain2 = Value::String("example.com".into());
        let scheme2 = Value::String("https".into());
        let client_p = Value::I64(9443);
        let mut builder2 = HttpRequestUrlBuilder::new(false);
        builder2.url_domain(&domain2);
        builder2.url_scheme(&scheme2);
        builder2.client_port(&client_p);

        let mut doc2 = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder2.resolve(&mut doc2).unwrap();
        let json2 = build_json(doc2);
        assert_eq!(
            extract_url(&json2).as_deref(),
            Some("https://example.com:9443"),
            "non-server span should fall back to client_port, got: {json2}"
        );

        // url_port takes priority over server_port/client_port
        let domain3 = Value::String("example.com".into());
        let scheme3 = Value::String("http".into());
        let url_p = Value::I64(5000);
        let server_p2 = Value::I64(6000);
        let mut builder3 = HttpRequestUrlBuilder::new(true);
        builder3.url_domain(&domain3);
        builder3.url_scheme(&scheme3);
        builder3.url_port(&url_p);
        builder3.server_port(&server_p2);

        let mut doc3 = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder3.resolve(&mut doc3).unwrap();
        let json3 = build_json(doc3);
        assert_eq!(
            extract_url(&json3).as_deref(),
            Some("http://example.com:5000"),
            "url_port should take priority over server_port, got: {json3}"
        );
    }

    // ---------------------------------------------------------------
    // Tests for resolve(): combined path + query + port
    // ---------------------------------------------------------------

    #[test]
    fn resolve_full_assembly_with_all_parts() {
        // All parts set: scheme + domain + non-default port + path (no leading /) + query (no leading ?)
        let domain = Value::String("api.example.com".into());
        let scheme = Value::String("https".into());
        let port = Value::I64(8443);
        let path = Value::String("v1/users".into());
        let query = Value::String("active=true&limit=50".into());
        let mut builder = HttpRequestUrlBuilder::new(false);
        builder.url_domain(&domain);
        builder.url_scheme(&scheme);
        builder.url_port(&port);
        builder.url_path(&path);
        builder.url_query(&query);

        let mut doc = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder.resolve(&mut doc).unwrap();
        let json = build_json(doc);
        assert_eq!(
            extract_url(&json).as_deref(),
            Some("https://api.example.com:8443/v1/users?active=true&limit=50"),
            "full assembly with all parts, got: {json}"
        );

        // All parts set: scheme + domain + default port (omitted) + path (with /) + query (with ?)
        let domain2 = Value::String("api.example.com".into());
        let scheme2 = Value::String("https".into());
        let port2 = Value::I64(443);
        let path2 = Value::String("/v2/orders".into());
        let query2 = Value::String("?status=pending".into());
        let mut builder2 = HttpRequestUrlBuilder::new(false);
        builder2.url_domain(&domain2);
        builder2.url_scheme(&scheme2);
        builder2.url_port(&port2);
        builder2.url_path(&path2);
        builder2.url_query(&query2);

        let mut doc2 = AnyDocumentBuilder::Subsegment(SubsegmentDocumentBuilder::default());
        builder2.resolve(&mut doc2).unwrap();
        let json2 = build_json(doc2);
        assert_eq!(
            extract_url(&json2).as_deref(),
            Some("https://api.example.com/v2/orders?status=pending"),
            "default port omitted with leading separators, got: {json2}"
        );
    }
}
