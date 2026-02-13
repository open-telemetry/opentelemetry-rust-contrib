use opentelemetry::InstrumentationScope;

#[cfg(feature = "metrics")]
#[cfg_attr(docsrs, doc(cfg(feature = "metrics")))]
pub(crate) mod metrics;
pub(crate) mod route_formatter;
pub(crate) mod trace;

pub(crate) fn get_scope() -> InstrumentationScope {
    InstrumentationScope::builder("opentelemetry-instrumentation-actix-web")
        .with_version(env!("CARGO_PKG_VERSION"))
        .with_schema_url(opentelemetry_semantic_conventions::SCHEMA_URL)
        .build()
}

#[cfg(test)]
mod tests {
    use actix_web::{test, web, App, HttpResponse};
    use opentelemetry::global;
    use opentelemetry::trace::Status;
    use opentelemetry_sdk::propagation::TraceContextPropagator;
    use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};
    use serial_test::serial;

    use super::route_formatter::RouteFormatter;
    use super::trace::RequestTracing;

    /// Helper to set up a tracer provider with in-memory exporter for testing
    fn setup_test_tracer() -> (SdkTracerProvider, InMemorySpanExporter) {
        let exporter = InMemorySpanExporter::default();
        let provider = SdkTracerProvider::builder()
            .with_simple_exporter(exporter.clone())
            .build();
        global::set_tracer_provider(provider.clone());
        (provider, exporter)
    }

    async fn index_handler() -> HttpResponse {
        HttpResponse::Ok().body("Hello, World!")
    }

    async fn user_handler(path: web::Path<u32>) -> HttpResponse {
        HttpResponse::Ok().body(format!("User: {}", path.into_inner()))
    }

    async fn error_handler() -> HttpResponse {
        HttpResponse::InternalServerError().body("Error")
    }

    async fn not_found_handler() -> HttpResponse {
        HttpResponse::NotFound().body("Not Found")
    }

    /// Test that tracing creates spans with correct span name format (method + route).
    #[actix_web::test]
    #[serial]
    async fn test_span_name_follows_semconv() {
        let (provider, exporter) = setup_test_tracer();

        let app = test::init_service(
            App::new()
                .wrap(RequestTracing::new())
                .route("/users/{id}", web::get().to(user_handler)),
        )
        .await;

        let req = test::TestRequest::get().uri("/users/123").to_request();
        let resp = test::call_service(&app, req).await;
        assert!(resp.status().is_success());

        provider.force_flush().unwrap();

        let spans = exporter.get_finished_spans().unwrap();
        assert_eq!(spans.len(), 1, "Expected exactly one span");

        let span = &spans[0];
        // Span name should follow semconv: "{method} {route}"
        assert_eq!(
            span.name, "GET /users/{id}",
            "Span name should be 'METHOD route'"
        );
    }

    /// Test that span has required HTTP semantic convention attributes.
    #[actix_web::test]
    #[serial]
    async fn test_span_has_required_attributes() {
        let (provider, exporter) = setup_test_tracer();

        let app = test::init_service(
            App::new()
                .wrap(RequestTracing::new())
                .route("/api/test", web::get().to(index_handler)),
        )
        .await;

        let req = test::TestRequest::get()
            .uri("/api/test?q=search")
            .insert_header(("User-Agent", "test-agent/1.0"))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert!(resp.status().is_success());

        provider.force_flush().unwrap();

        let spans = exporter.get_finished_spans().unwrap();
        assert_eq!(spans.len(), 1);

        let span = &spans[0];
        let attr_keys: Vec<&str> = span.attributes.iter().map(|kv| kv.key.as_str()).collect();

        // Required attributes per HTTP semconv
        assert!(
            attr_keys.contains(&"http.request.method"),
            "Should have http.request.method"
        );
        assert!(attr_keys.contains(&"http.route"), "Should have http.route");
        assert!(attr_keys.contains(&"url.scheme"), "Should have url.scheme");
        assert!(
            attr_keys.contains(&"http.response.status_code"),
            "Should have http.response.status_code"
        );

        // Verify http.request.method value
        let method_attr = span
            .attributes
            .iter()
            .find(|kv| kv.key.as_str() == "http.request.method")
            .unwrap();
        assert_eq!(method_attr.value.as_str(), "GET");

        // Verify http.response.status_code value
        let status_attr = span
            .attributes
            .iter()
            .find(|kv| kv.key.as_str() == "http.response.status_code")
            .unwrap();
        if let opentelemetry::Value::I64(code) = &status_attr.value {
            assert_eq!(*code, 200);
        } else {
            panic!("Expected i64 status code");
        }
    }

    /// Test that 5xx responses set span status to Error and add error.type attribute.
    #[actix_web::test]
    #[serial]
    async fn test_5xx_sets_error_status_and_attribute() {
        let (provider, exporter) = setup_test_tracer();

        let app = test::init_service(
            App::new()
                .wrap(RequestTracing::new())
                .route("/error", web::get().to(error_handler)),
        )
        .await;

        let req = test::TestRequest::get().uri("/error").to_request();
        let resp = test::call_service(&app, req).await;
        assert!(resp.status().is_server_error());

        provider.force_flush().unwrap();

        let spans = exporter.get_finished_spans().unwrap();
        assert_eq!(spans.len(), 1);

        let span = &spans[0];

        // Verify error.type attribute is set to status code
        let error_type_attr = span
            .attributes
            .iter()
            .find(|kv| kv.key.as_str() == "error.type");
        assert!(
            error_type_attr.is_some(),
            "error.type attribute should be set for 5xx"
        );
        assert_eq!(error_type_attr.unwrap().value.as_str(), "500");

        // Verify span status is Error (per semconv, server 5xx is error)
        assert!(
            matches!(span.status, Status::Error { .. }),
            "Span status should be Error for 5xx response"
        );
    }

    /// Test that 4xx responses set error.type but NOT span status error (server semconv).
    #[actix_web::test]
    #[serial]
    async fn test_4xx_sets_error_type_but_not_status() {
        let (provider, exporter) = setup_test_tracer();

        let app = test::init_service(
            App::new()
                .wrap(RequestTracing::new())
                .route("/notfound", web::get().to(not_found_handler)),
        )
        .await;

        let req = test::TestRequest::get().uri("/notfound").to_request();
        let resp = test::call_service(&app, req).await;
        assert!(resp.status().is_client_error());

        provider.force_flush().unwrap();

        let spans = exporter.get_finished_spans().unwrap();
        assert_eq!(spans.len(), 1);

        let span = &spans[0];

        // Verify error.type attribute is set for 4xx
        let error_type_attr = span
            .attributes
            .iter()
            .find(|kv| kv.key.as_str() == "error.type");
        assert!(
            error_type_attr.is_some(),
            "error.type attribute should be set for 4xx"
        );
        assert_eq!(error_type_attr.unwrap().value.as_str(), "404");

        // For server spans, 4xx should NOT set span status to error per semconv
        assert!(
            matches!(span.status, Status::Unset),
            "Span status should be Unset for 4xx on server span, got: {:?}",
            span.status
        );
    }

    /// Test that route formatter affects span name.
    #[actix_web::test]
    #[serial]
    async fn test_route_formatter_affects_span_name() {
        let (provider, exporter) = setup_test_tracer();

        #[derive(Debug)]
        struct LowercaseFormatter;

        impl RouteFormatter for LowercaseFormatter {
            fn format(&self, path: &str) -> String {
                path.to_lowercase()
            }
        }

        let app = test::init_service(
            App::new()
                .wrap(RequestTracing::with_formatter(LowercaseFormatter))
                .route("/USERS/{id}", web::get().to(user_handler)),
        )
        .await;

        let req = test::TestRequest::get().uri("/USERS/123").to_request();
        let resp = test::call_service(&app, req).await;
        assert!(resp.status().is_success());

        provider.force_flush().unwrap();

        let spans = exporter.get_finished_spans().unwrap();
        assert_eq!(spans.len(), 1);

        let span = &spans[0];
        // Route should be formatted to lowercase
        assert_eq!(
            span.name, "GET /users/{id}",
            "Span name should use formatted route"
        );
    }

    /// Test that trace context propagation works (traceparent header).
    #[actix_web::test]
    #[serial]
    async fn test_trace_context_propagation() {
        // Set up the W3C TraceContext propagator
        global::set_text_map_propagator(TraceContextPropagator::new());

        let (provider, exporter) = setup_test_tracer();

        let app = test::init_service(
            App::new()
                .wrap(RequestTracing::new())
                .route("/", web::get().to(index_handler)),
        )
        .await;

        // W3C trace-context header
        let trace_id = "0af7651916cd43dd8448eb211c80319c";
        let parent_span_id = "b7ad6b7169203331";
        let traceparent = format!("00-{}-{}-01", trace_id, parent_span_id);

        let req = test::TestRequest::get()
            .uri("/")
            .insert_header(("traceparent", traceparent))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert!(resp.status().is_success());

        provider.force_flush().unwrap();

        let spans = exporter.get_finished_spans().unwrap();
        assert_eq!(spans.len(), 1);

        let span = &spans[0];

        // Verify the span inherited the trace ID from the parent
        assert_eq!(
            format!("{:032x}", span.span_context.trace_id()),
            trace_id,
            "Span should have parent's trace ID"
        );

        // Verify parent span ID is set
        assert_eq!(
            format!("{:016x}", span.parent_span_id),
            parent_span_id,
            "Span should have correct parent span ID"
        );
    }

    /// Test that different HTTP methods produce correct span names.
    #[actix_web::test]
    #[serial]
    async fn test_various_http_methods_span_names() {
        let (provider, exporter) = setup_test_tracer();

        async fn post_handler() -> HttpResponse {
            HttpResponse::Created().body("Created")
        }

        let app = test::init_service(
            App::new()
                .wrap(RequestTracing::new())
                .route("/resource", web::get().to(index_handler))
                .route("/resource", web::post().to(post_handler)),
        )
        .await;

        // Test GET
        let req = test::TestRequest::get().uri("/resource").to_request();
        test::call_service(&app, req).await;

        // Test POST
        let req = test::TestRequest::post().uri("/resource").to_request();
        test::call_service(&app, req).await;

        provider.force_flush().unwrap();

        let spans = exporter.get_finished_spans().unwrap();
        assert_eq!(spans.len(), 2, "Expected two spans");

        let span_names: Vec<&str> = spans.iter().map(|s| s.name.as_ref()).collect();
        assert!(span_names.contains(&"GET /resource"));
        assert!(span_names.contains(&"POST /resource"));
    }

    /// Test user_agent.original attribute is captured.
    #[actix_web::test]
    #[serial]
    async fn test_user_agent_attribute() {
        let (provider, exporter) = setup_test_tracer();

        let app = test::init_service(
            App::new()
                .wrap(RequestTracing::new())
                .route("/", web::get().to(index_handler)),
        )
        .await;

        let req = test::TestRequest::get()
            .uri("/")
            .insert_header(("User-Agent", "Mozilla/5.0 TestBrowser"))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert!(resp.status().is_success());

        provider.force_flush().unwrap();

        let spans = exporter.get_finished_spans().unwrap();
        assert_eq!(spans.len(), 1);

        let span = &spans[0];
        let user_agent_attr = span
            .attributes
            .iter()
            .find(|kv| kv.key.as_str() == "user_agent.original");
        assert!(
            user_agent_attr.is_some(),
            "user_agent.original should be present"
        );
        assert_eq!(
            user_agent_attr.unwrap().value.as_str(),
            "Mozilla/5.0 TestBrowser"
        );
    }
}

/// Metrics tests using InMemoryMetricExporter
#[cfg(test)]
#[cfg(feature = "metrics")]
mod metrics_tests {
    use actix_web::{test, web, App, HttpResponse};
    use opentelemetry::global;
    use opentelemetry_sdk::metrics::data::{AggregatedMetrics, MetricData};
    use opentelemetry_sdk::metrics::{InMemoryMetricExporter, PeriodicReader, SdkMeterProvider};
    use serial_test::serial;
    use std::time::Duration;

    use super::metrics::RequestMetrics;

    /// Helper to set up a meter provider with in-memory exporter for testing
    fn setup_test_meter() -> (SdkMeterProvider, InMemoryMetricExporter) {
        let exporter = InMemoryMetricExporter::default();
        // Use a very large interval to ensure metrics are only exported via force_flush
        let reader = PeriodicReader::builder(exporter.clone())
            .with_interval(Duration::from_secs(3600))
            .build();
        let provider = SdkMeterProvider::builder().with_reader(reader).build();
        global::set_meter_provider(provider.clone());
        (provider, exporter)
    }

    async fn index_handler() -> HttpResponse {
        HttpResponse::Ok().body("Hello, World!")
    }

    async fn error_handler() -> HttpResponse {
        HttpResponse::InternalServerError().body("Error")
    }

    /// Test that metrics middleware emits http.server.request.duration metric with correct attributes.
    #[actix_web::test]
    #[serial]
    async fn test_request_duration_metric_emitted() {
        let (provider, exporter) = setup_test_meter();

        let app = test::init_service(
            App::new()
                .wrap(RequestMetrics::default())
                .route("/api/test", web::get().to(index_handler)),
        )
        .await;

        let req = test::TestRequest::get().uri("/api/test").to_request();
        let resp = test::call_service(&app, req).await;
        assert!(resp.status().is_success());

        // Force flush metrics
        provider.force_flush().unwrap();

        let metrics = exporter.get_finished_metrics().unwrap();
        assert!(!metrics.is_empty(), "Should have exported metrics");

        let resource_metrics = &metrics[0];
        let scope_metrics = resource_metrics
            .scope_metrics()
            .find(|sm| sm.scope().name() == "opentelemetry-instrumentation-actix-web");

        assert!(scope_metrics.is_some(), "Should have scope metrics");
        let scope_metrics = scope_metrics.unwrap();

        // Check http.server.request.duration metric exists
        let duration_metric = scope_metrics
            .metrics()
            .find(|m| m.name() == "http.server.request.duration");
        assert!(
            duration_metric.is_some(),
            "http.server.request.duration metric should exist"
        );

        let duration_metric = duration_metric.unwrap();
        if let AggregatedMetrics::F64(MetricData::Histogram(histogram)) = duration_metric.data() {
            let data_point = histogram.data_points().next();
            assert!(data_point.is_some(), "Should have data point");

            let data_point = data_point.unwrap();
            let attributes: Vec<_> = data_point.attributes().collect();

            // Check required attributes are present
            let has_method = attributes
                .iter()
                .any(|kv| kv.key.as_str() == "http.request.method");
            let has_route = attributes.iter().any(|kv| kv.key.as_str() == "http.route");
            let has_status_code = attributes
                .iter()
                .any(|kv| kv.key.as_str() == "http.response.status_code");
            let has_url_scheme = attributes.iter().any(|kv| kv.key.as_str() == "url.scheme");

            assert!(has_method, "Should have http.request.method attribute");
            assert!(has_route, "Should have http.route attribute");
            assert!(
                has_status_code,
                "Should have http.response.status_code attribute"
            );
            assert!(has_url_scheme, "Should have url.scheme attribute");

            // Verify attribute values
            let method_attr = attributes
                .iter()
                .find(|kv| kv.key.as_str() == "http.request.method")
                .unwrap();
            assert_eq!(method_attr.value.as_str(), "GET");

            let route_attr = attributes
                .iter()
                .find(|kv| kv.key.as_str() == "http.route")
                .unwrap();
            assert_eq!(route_attr.value.as_str(), "/api/test");

            let status_attr = attributes
                .iter()
                .find(|kv| kv.key.as_str() == "http.response.status_code")
                .unwrap();
            if let opentelemetry::Value::I64(code) = &status_attr.value {
                assert_eq!(*code, 200);
            } else {
                panic!("Expected i64 status code");
            }
        } else {
            panic!("Expected histogram data for duration metric");
        }
    }

    /// Test that 5xx responses add error.type attribute to metrics.
    #[actix_web::test]
    #[serial]
    async fn test_5xx_adds_error_type_to_metrics() {
        let (provider, exporter) = setup_test_meter();

        let app = test::init_service(
            App::new()
                .wrap(RequestMetrics::default())
                .route("/error", web::get().to(error_handler)),
        )
        .await;

        let req = test::TestRequest::get().uri("/error").to_request();
        let resp = test::call_service(&app, req).await;
        assert!(resp.status().is_server_error());

        // Force flush metrics
        provider.force_flush().unwrap();

        let metrics = exporter.get_finished_metrics().unwrap();
        assert!(!metrics.is_empty(), "Should have exported metrics");

        let resource_metrics = &metrics[0];
        let scope_metrics = resource_metrics
            .scope_metrics()
            .find(|sm| sm.scope().name() == "opentelemetry-instrumentation-actix-web");

        assert!(scope_metrics.is_some(), "Should have scope metrics");
        let scope_metrics = scope_metrics.unwrap();

        let duration_metric = scope_metrics
            .metrics()
            .find(|m| m.name() == "http.server.request.duration");
        assert!(duration_metric.is_some(), "Duration metric should exist");

        let duration_metric = duration_metric.unwrap();
        if let AggregatedMetrics::F64(MetricData::Histogram(histogram)) = duration_metric.data() {
            let data_point = histogram.data_points().next().unwrap();
            let attributes: Vec<_> = data_point.attributes().collect();

            // Check error.type attribute is present for 5xx
            let error_type_attr = attributes.iter().find(|kv| kv.key.as_str() == "error.type");
            assert!(
                error_type_attr.is_some(),
                "error.type attribute should be present for 5xx"
            );
            assert_eq!(error_type_attr.unwrap().value.as_str(), "500");

            // Verify status code is 500
            let status_attr = attributes
                .iter()
                .find(|kv| kv.key.as_str() == "http.response.status_code")
                .unwrap();
            if let opentelemetry::Value::I64(code) = &status_attr.value {
                assert_eq!(*code, 500);
            } else {
                panic!("Expected i64 status code");
            }
        } else {
            panic!("Expected histogram data");
        }
    }

    /// Test that body size metrics are emitted.
    #[actix_web::test]
    #[serial]
    async fn test_body_size_metrics_emitted() {
        let (provider, exporter) = setup_test_meter();

        let app = test::init_service(
            App::new()
                .wrap(RequestMetrics::default())
                .route("/", web::get().to(index_handler)),
        )
        .await;

        let req = test::TestRequest::get().uri("/").to_request();
        let resp = test::call_service(&app, req).await;
        assert!(resp.status().is_success());

        // Force flush metrics
        provider.force_flush().unwrap();

        let metrics = exporter.get_finished_metrics().unwrap();
        assert!(!metrics.is_empty(), "Should have exported metrics");

        let resource_metrics = &metrics[0];
        let scope_metrics = resource_metrics
            .scope_metrics()
            .find(|sm| sm.scope().name() == "opentelemetry-instrumentation-actix-web");

        assert!(scope_metrics.is_some(), "Should have scope metrics");
        let scope_metrics = scope_metrics.unwrap();

        // Collect all metric names
        let metric_names: Vec<&str> = scope_metrics.metrics().map(|m| m.name()).collect();

        // Check that response body size metric exists
        assert!(
            metric_names.contains(&"http.server.response.body.size"),
            "http.server.response.body.size metric should exist. Found: {:?}",
            metric_names
        );
    }

    /// Test that all expected metrics are emitted with correct names.
    #[actix_web::test]
    #[serial]
    async fn test_all_expected_metrics_emitted() {
        let (provider, exporter) = setup_test_meter();

        let app = test::init_service(
            App::new()
                .wrap(RequestMetrics::default())
                .route("/", web::get().to(index_handler)),
        )
        .await;

        let req = test::TestRequest::get()
            .uri("/")
            .insert_header(("Content-Length", "100"))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert!(resp.status().is_success());

        // Force flush metrics
        provider.force_flush().unwrap();

        let metrics = exporter.get_finished_metrics().unwrap();
        assert!(!metrics.is_empty(), "Should have exported metrics");

        let resource_metrics = &metrics[0];
        let scope_metrics = resource_metrics
            .scope_metrics()
            .find(|sm| sm.scope().name() == "opentelemetry-instrumentation-actix-web");

        assert!(scope_metrics.is_some(), "Should have scope metrics");
        let scope_metrics = scope_metrics.unwrap();

        let metric_names: Vec<&str> = scope_metrics.metrics().map(|m| m.name()).collect();

        // These are the metrics that should be emitted per HTTP semantic conventions
        assert!(
            metric_names.contains(&"http.server.request.duration"),
            "Should emit http.server.request.duration"
        );
        assert!(
            metric_names.contains(&"http.server.response.body.size"),
            "Should emit http.server.response.body.size"
        );
        // Note: http.server.request.body.size is only emitted when Content-Length > 0
        // and http.server.active_requests is an UpDownCounter (may not show in histogram exports)
    }
}
