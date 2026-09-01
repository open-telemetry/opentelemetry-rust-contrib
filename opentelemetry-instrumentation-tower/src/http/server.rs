//! HTTP server instrumentation layer.
//!
//! Produces a `SpanKind::Server` span and the standard HTTP server metrics for
//! every incoming request, extracting the parent context from request headers.

use std::borrow::Cow;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Instant;

use opentelemetry::global::{self, BoxedTracer};
use opentelemetry::metrics::{Histogram, Meter, MeterProvider, NoopMeterProvider, UpDownCounter};
use opentelemetry::trace::noop::NoopTracerProvider;
use opentelemetry::trace::{SpanKind, Status, TraceContextExt, Tracer, TracerProvider};
use opentelemetry::Context as OtelContext;
use opentelemetry::InstrumentationScope;
use opentelemetry::KeyValue;
use opentelemetry_http::HeaderExtractor;
use opentelemetry_semantic_conventions as semconv;
use pin_project_lite::pin_project;
use tower_layer::Layer as TowerLayer;
use tower_service::Service as TowerService;

use crate::common::attributes::{method_kv, split_and_format_protocol_version, url_scheme_kv};
use crate::http::extractors::{
    DefaultRouteExtractor, NoOpExtractor, RequestAttributeExtractor, ResponseAttributeExtractor,
    RouteExtractor,
};
use crate::Result;

const HTTP_SERVER_DURATION_UNIT: &str = "s";

const OTEL_DEFAULT_HTTP_SERVER_DURATION_BOUNDS: [f64; 14] = [
    0.005, 0.01, 0.025, 0.05, 0.075, 0.1, 0.25, 0.5, 0.75, 1.0, 2.5, 5.0, 7.5, 10.0,
];

const HTTP_SERVER_ACTIVE_REQUESTS_UNIT: &str = "{request}";

const HTTP_SERVER_REQUEST_BODY_SIZE_UNIT: &str = "By";

const HTTP_SERVER_RESPONSE_BODY_SIZE_UNIT: &str = "By";

/// State scoped to the entire middleware [`Layer`].
struct LayerState {
    server_request_duration: Histogram<f64>,
    server_active_requests: UpDownCounter<i64>,
    server_request_body_size: Histogram<u64>,
    server_response_body_size: Histogram<u64>,
}

#[derive(Clone)]
/// [`tower_service::Service`] produced by [`Layer`].
pub struct Service<
    S,
    RouteExt = DefaultRouteExtractor,
    ReqExt = NoOpExtractor,
    ResExt = NoOpExtractor,
> {
    state: Arc<LayerState>,
    route_extractor: RouteExt,
    request_extractor: ReqExt,
    response_extractor: ResExt,
    inner_service: S,
    tracer: Arc<BoxedTracer>,
}

#[derive(Clone)]
/// [`tower_layer::Layer`] which applies OpenTelemetry HTTP server metrics and tracing.
pub struct Layer<RouteExt = DefaultRouteExtractor, ReqExt = NoOpExtractor, ResExt = NoOpExtractor> {
    state: Arc<LayerState>,
    route_extractor: RouteExt,
    request_extractor: ReqExt,
    response_extractor: ResExt,
    tracer: Arc<BoxedTracer>,
}

impl Layer {
    /// Create a new HTTP server layer with default configuration using global providers.
    pub fn new() -> Self {
        LayerBuilder::builder().build().unwrap()
    }
}

impl Default for Layer {
    fn default() -> Self {
        Self::new()
    }
}

/// Builder for the HTTP server [`Layer`].
pub struct LayerBuilder<
    RouteExt = DefaultRouteExtractor,
    ReqExt = NoOpExtractor,
    ResExt = NoOpExtractor,
> {
    tracer: Option<Arc<BoxedTracer>>,
    meter: Option<Meter>,
    tracing_enabled: bool,
    metrics_enabled: bool,
    req_dur_bounds: Option<Vec<f64>>,
    route_extractor: RouteExt,
    request_extractor: ReqExt,
    response_extractor: ResExt,
}

impl LayerBuilder {
    pub fn builder() -> Self {
        LayerBuilder {
            tracer: None,
            meter: None,
            tracing_enabled: true,
            metrics_enabled: true,
            req_dur_bounds: Some(Vec::from(OTEL_DEFAULT_HTTP_SERVER_DURATION_BOUNDS)),
            route_extractor: DefaultRouteExtractor::default(),
            request_extractor: NoOpExtractor,
            response_extractor: NoOpExtractor,
        }
    }
}

impl<RouteExt, ReqExt, ResExt> LayerBuilder<RouteExt, ReqExt, ResExt> {
    /// Set a custom route extractor.
    ///
    /// The route extractor determines how the route is extracted from requests.
    /// The extracted route is used for:
    /// - Span names: `"{method} {route}"` or just `"{method}"` if no route
    /// - The `http.route` metric attribute
    ///
    /// See [`RouteExtractor`] for details on implementing custom extractors.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let layer = LayerBuilder::builder()
    ///     .with_route_extractor(PathExtractor)
    ///     .build()
    ///     .unwrap();
    /// ```
    pub fn with_route_extractor<NewRoute>(
        self,
        extractor: NewRoute,
    ) -> LayerBuilder<NewRoute, ReqExt, ResExt> {
        LayerBuilder {
            tracer: self.tracer,
            meter: self.meter,
            tracing_enabled: self.tracing_enabled,
            metrics_enabled: self.metrics_enabled,
            req_dur_bounds: self.req_dur_bounds,
            route_extractor: extractor,
            request_extractor: self.request_extractor,
            response_extractor: self.response_extractor,
        }
    }

    /// Convenience method to set a function-based route extractor.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let layer = LayerBuilder::builder()
    ///     .with_route_extractor_fn(|req: &http::Request<_>| {
    ///         // Return Some(route) or None for method-only
    ///         Some(req.uri().path().to_owned())
    ///     })
    ///     .build()
    ///     .unwrap();
    /// ```
    pub fn with_route_extractor_fn<F, B>(
        self,
        f: F,
    ) -> LayerBuilder<crate::http::extractors::FnRouteExtractor<F>, ReqExt, ResExt>
    where
        F: Fn(&http::Request<B>) -> Option<String> + Clone + Send + Sync + 'static,
    {
        self.with_route_extractor(crate::http::extractors::FnRouteExtractor::new(f))
    }

    /// Set a request attribute extractor.
    pub fn with_request_extractor<NewReqExt, B>(
        self,
        extractor: NewReqExt,
    ) -> LayerBuilder<RouteExt, NewReqExt, ResExt>
    where
        NewReqExt: RequestAttributeExtractor<B>,
    {
        LayerBuilder {
            tracer: self.tracer,
            meter: self.meter,
            tracing_enabled: self.tracing_enabled,
            metrics_enabled: self.metrics_enabled,
            req_dur_bounds: self.req_dur_bounds,
            route_extractor: self.route_extractor,
            request_extractor: extractor,
            response_extractor: self.response_extractor,
        }
    }

    /// Set a response attribute extractor.
    pub fn with_response_extractor<NewResExt, B>(
        self,
        extractor: NewResExt,
    ) -> LayerBuilder<RouteExt, ReqExt, NewResExt>
    where
        NewResExt: ResponseAttributeExtractor<B>,
    {
        LayerBuilder {
            tracer: self.tracer,
            meter: self.meter,
            tracing_enabled: self.tracing_enabled,
            metrics_enabled: self.metrics_enabled,
            req_dur_bounds: self.req_dur_bounds,
            route_extractor: self.route_extractor,
            request_extractor: self.request_extractor,
            response_extractor: extractor,
        }
    }

    /// Convenience method to set a function-based request extractor.
    pub fn with_request_extractor_fn<F, B>(
        self,
        f: F,
    ) -> LayerBuilder<RouteExt, crate::http::extractors::FnRequestExtractor<F>, ResExt>
    where
        F: Fn(&http::Request<B>) -> Vec<KeyValue> + Clone + Send + Sync + 'static,
    {
        self.with_request_extractor(crate::http::extractors::FnRequestExtractor::new(f))
    }

    /// Convenience method to set a function-based response extractor.
    pub fn with_response_extractor_fn<F, B>(
        self,
        f: F,
    ) -> LayerBuilder<RouteExt, ReqExt, crate::http::extractors::FnResponseExtractor<F>>
    where
        F: Fn(&http::Response<B>) -> Vec<KeyValue> + Clone + Send + Sync + 'static,
    {
        self.with_response_extractor(crate::http::extractors::FnResponseExtractor::new(f))
    }

    pub fn build(self) -> Result<Layer<RouteExt, ReqExt, ResExt>> {
        let req_dur_bounds = self
            .req_dur_bounds
            .unwrap_or_else(|| Vec::from(OTEL_DEFAULT_HTTP_SERVER_DURATION_BOUNDS));

        let tracer = if self.tracing_enabled {
            self.tracer
                .unwrap_or_else(|| Arc::new(global::tracer_with_scope(instrumentation_scope())))
        } else {
            Arc::new(BoxedTracer::new(Box::new(
                NoopTracerProvider::new().tracer_with_scope(instrumentation_scope()),
            )))
        };

        let meter: Meter = if self.metrics_enabled {
            self.meter
                .unwrap_or_else(|| global::meter_with_scope(instrumentation_scope()))
        } else {
            NoopMeterProvider::new().meter_with_scope(instrumentation_scope())
        };

        Ok(Layer {
            state: Arc::from(make_state(meter, req_dur_bounds)),
            route_extractor: self.route_extractor,
            request_extractor: self.request_extractor,
            response_extractor: self.response_extractor,
            tracer,
        })
    }

    /// Enable or disable trace collection for this layer.
    ///
    /// Tracing is enabled by default. When disabled, the layer records no
    /// spans, but context propagation is unaffected: incoming trace headers
    /// are still extracted and the current context still flows to the inner
    /// service.
    pub fn with_tracing(mut self, enabled: bool) -> Self {
        self.tracing_enabled = enabled;
        self
    }

    /// Enable or disable metrics collection for this layer.
    ///
    /// Metrics are enabled by default. When disabled, the layer records no
    /// HTTP server metrics.
    pub fn with_metrics(mut self, enabled: bool) -> Self {
        self.metrics_enabled = enabled;
        self
    }

    #[cfg(test)]
    fn with_tracer_provider<P>(mut self, tracer_provider: P) -> Self
    where
        P: TracerProvider,
        P::Tracer: opentelemetry::global::ObjectSafeTracer + Send + Sync + 'static,
    {
        self.tracer = Some(Arc::new(BoxedTracer::new(Box::new(
            tracer_provider.tracer_with_scope(instrumentation_scope()),
        ))));
        self
    }

    #[cfg(test)]
    fn with_meter_provider(mut self, meter_provider: impl MeterProvider) -> Self {
        self.meter = Some(meter_provider.meter_with_scope(instrumentation_scope()));
        self
    }
}

fn instrumentation_scope() -> InstrumentationScope {
    InstrumentationScope::builder(crate::INSTRUMENTATION_NAME)
        .with_version(env!("CARGO_PKG_VERSION"))
        .with_schema_url(opentelemetry_semantic_conventions::SCHEMA_URL)
        .build()
}

fn make_state(meter: Meter, req_dur_bounds: Vec<f64>) -> LayerState {
    LayerState {
        server_request_duration: meter
            .f64_histogram(Cow::from(semconv::metric::HTTP_SERVER_REQUEST_DURATION))
            .with_description("Duration of HTTP server requests.")
            .with_unit(Cow::from(HTTP_SERVER_DURATION_UNIT))
            .with_boundaries(req_dur_bounds)
            .build(),
        server_active_requests: meter
            .i64_up_down_counter(Cow::from(semconv::metric::HTTP_SERVER_ACTIVE_REQUESTS))
            .with_description("Number of active HTTP server requests.")
            .with_unit(Cow::from(HTTP_SERVER_ACTIVE_REQUESTS_UNIT))
            .build(),
        server_request_body_size: meter
            .u64_histogram(semconv::metric::HTTP_SERVER_REQUEST_BODY_SIZE)
            .with_description("Size of HTTP server request bodies.")
            .with_unit(HTTP_SERVER_REQUEST_BODY_SIZE_UNIT)
            .build(),
        server_response_body_size: meter
            .u64_histogram(semconv::metric::HTTP_SERVER_RESPONSE_BODY_SIZE)
            .with_description("Size of HTTP server response bodies.")
            .with_unit(HTTP_SERVER_RESPONSE_BODY_SIZE_UNIT)
            .build(),
    }
}

impl<S, RouteExt, ReqExt, ResExt> TowerLayer<S> for Layer<RouteExt, ReqExt, ResExt>
where
    RouteExt: Clone,
    ReqExt: Clone,
    ResExt: Clone,
{
    type Service = Service<S, RouteExt, ReqExt, ResExt>;

    fn layer(&self, service: S) -> Self::Service {
        Service {
            state: self.state.clone(),
            route_extractor: self.route_extractor.clone(),
            request_extractor: self.request_extractor.clone(),
            response_extractor: self.response_extractor.clone(),
            inner_service: service,
            tracer: self.tracer.clone(),
        }
    }
}

/// Request data extracted before the inner service call.
/// This data is needed for metrics and span finalization after the response is received.
struct RequestData {
    duration_start: Instant,
    req_body_size: Option<u64>,

    protocol_name_kv: KeyValue,
    protocol_version_kv: KeyValue,
    url_scheme_kv: KeyValue,
    method_kv: KeyValue,
    route_kv_opt: Option<KeyValue>,

    custom_request_attributes: Vec<KeyValue>,
}

struct RequestFinalization<ResExt> {
    request_data: RequestData,
    layer_state: Arc<LayerState>,
    response_extractor: ResExt,
}

pin_project! {
    /// Future returned by the server [`Service`].
    ///
    /// This is a concrete future that avoids heap allocation by embedding the
    /// inner service future directly, rather than using `Pin<Box<dyn Future>>`.
    pub struct ResponseFuture<F, ResExt> {
        #[pin]
        inner: F,
        otel_cx: OtelContext,
        finalization: Option<RequestFinalization<ResExt>>,
    }
}

impl<F, ResBody, E, ResExt> Future for ResponseFuture<F, ResExt>
where
    F: Future<Output = std::result::Result<http::Response<ResBody>, E>>,
    E: std::fmt::Debug,
    ResBody: http_body::Body,
    ResExt: ResponseAttributeExtractor<ResBody>,
{
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        let _guard = this.otel_cx.clone().attach();
        let result = std::task::ready!(this.inner.poll(cx));
        if let Some(RequestFinalization {
            request_data,
            layer_state,
            response_extractor,
        }) = this.finalization.take()
        {
            finalize_request(&result, request_data, &layer_state, &response_extractor);
        }
        Poll::Ready(result)
    }
}

impl<S, ReqBody, ResBody, RouteExt, ReqExt, ResExt> TowerService<http::Request<ReqBody>>
    for Service<S, RouteExt, ReqExt, ResExt>
where
    S: TowerService<http::Request<ReqBody>, Response = http::Response<ResBody>>,
    S::Future: Send,
    S::Error: std::fmt::Debug,
    ResBody: http_body::Body,
    RouteExt: RouteExtractor<ReqBody>,
    ReqExt: RequestAttributeExtractor<ReqBody>,
    ResExt: ResponseAttributeExtractor<ResBody>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = ResponseFuture<S::Future, ResExt>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<std::result::Result<(), Self::Error>> {
        self.inner_service.poll_ready(cx)
    }

    fn call(&mut self, req: http::Request<ReqBody>) -> Self::Future {
        let duration_start = Instant::now();

        let headers = req.headers();
        let content_length = headers
            .get(http::header::CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok()?.parse::<u64>().ok());

        let (protocol, version) = split_and_format_protocol_version(req.version());
        let protocol_name_kv = KeyValue::new(semconv::attribute::NETWORK_PROTOCOL_NAME, protocol);
        let protocol_version_kv =
            KeyValue::new(semconv::attribute::NETWORK_PROTOCOL_VERSION, version);

        let url_scheme_kv = url_scheme_kv(req.uri());

        let method = req.method().as_str().to_owned();
        let method_kv = method_kv(req.method());

        // Extract route using the configured extractor
        let route = self.route_extractor.extract_route(&req);
        let route_kv_opt = route
            .as_ref()
            .map(|r| KeyValue::new(semconv::attribute::HTTP_ROUTE, r.clone()));

        // Build span name: "{method} {route}" or just "{method}"
        let span_name = match &route {
            Some(r) => format!("{} {}", method, r),
            None => method.clone(),
        };

        // Extract custom request attributes
        let custom_request_attributes = self.request_extractor.extract_attributes(&req);

        // Extract the context from the incoming request headers
        let parent_cx = global::get_text_map_propagator(|propagator| {
            propagator.extract(&HeaderExtractor(req.headers()))
        });

        let mut span_attributes = vec![
            KeyValue::new(semconv::attribute::HTTP_REQUEST_METHOD, method.clone()),
            url_scheme_kv.clone(),
            KeyValue::new(semconv::attribute::URL_PATH, req.uri().path().to_string()),
            KeyValue::new(semconv::attribute::URL_FULL, req.uri().to_string()),
        ];

        if let Some(user_agent) = req
            .headers()
            .get("user-agent")
            .and_then(|v| v.to_str().ok())
        {
            span_attributes.push(KeyValue::new(
                semconv::attribute::USER_AGENT_ORIGINAL,
                user_agent.to_string(),
            ));
        }

        if let Some(r) = &route {
            span_attributes.push(KeyValue::new(semconv::attribute::HTTP_ROUTE, r.clone()));
        }

        span_attributes.extend(custom_request_attributes.clone());

        let span = self
            .tracer
            .span_builder(span_name)
            .with_kind(SpanKind::Server)
            .with_attributes(span_attributes)
            .start_with_context(self.tracer.as_ref(), &parent_cx);

        let cx = parent_cx.with_span(span);

        self.state
            .server_active_requests
            .add(1, &[url_scheme_kv.clone(), method_kv.clone()]);

        let request_data = RequestData {
            duration_start,
            req_body_size: content_length,
            protocol_name_kv,
            protocol_version_kv,
            url_scheme_kv,
            method_kv,
            route_kv_opt,
            custom_request_attributes,
        };

        let layer_state = self.state.clone();
        let response_extractor = self.response_extractor.clone();

        let inner_future = self.inner_service.call(req);

        ResponseFuture {
            inner: inner_future,
            otel_cx: cx,
            finalization: Some(RequestFinalization {
                request_data,
                layer_state,
                response_extractor,
            }),
        }
    }
}

/// Finalizes the request by updating the span and recording metrics after the response is received.
fn finalize_request<ResBody, E, ResExt>(
    result: &std::result::Result<http::Response<ResBody>, E>,
    request_data: RequestData,
    layer_state: &Arc<LayerState>,
    response_extractor: &ResExt,
) where
    ResBody: http_body::Body,
    ResExt: ResponseAttributeExtractor<ResBody>,
    E: std::fmt::Debug,
{
    let RequestData {
        duration_start,
        req_body_size,
        protocol_name_kv,
        protocol_version_kv,
        url_scheme_kv,
        method_kv,
        route_kv_opt,
        custom_request_attributes,
    } = request_data;

    let cx = OtelContext::current();
    let span = cx.span();

    match result {
        Ok(response) => {
            let http_status = response.status();
            let status_code_kv = KeyValue::new(
                semconv::attribute::HTTP_RESPONSE_STATUS_CODE,
                i64::from(http_status.as_u16()),
            );

            // Extract custom response attributes (empty/non-allocating for NoOp).
            let custom_response_attributes = response_extractor.extract_attributes(response);

            // Update span
            span.set_attribute(status_code_kv.clone());
            for attr in &custom_response_attributes {
                span.set_attribute(attr.clone());
            }

            // Set span status based on HTTP status code. Per the HTTP semantic
            // conventions, a server span is only an error for 5xx responses.
            if http_status.is_server_error() {
                span.set_status(Status::Error {
                    description: format!("HTTP {}", http_status.as_u16()).into(),
                });
            }

            // Build label superset by moving owned values where possible.
            // `url_scheme_kv` and `method_kv` are cloned for the active-requests
            // decrement; their underlying strings are typically `&'static str`
            // so the clones are allocation-free.
            let cap = 5
                + route_kv_opt.is_some() as usize
                + custom_request_attributes.len()
                + custom_response_attributes.len();
            let mut label_superset = Vec::with_capacity(cap);
            label_superset.push(protocol_name_kv);
            label_superset.push(protocol_version_kv);
            label_superset.push(url_scheme_kv.clone());
            label_superset.push(method_kv.clone());
            label_superset.push(status_code_kv);
            if let Some(route_kv) = route_kv_opt {
                label_superset.push(route_kv);
            }
            // Move (not clone) the custom attribute Vecs into the label set.
            label_superset.extend(custom_request_attributes);
            label_superset.extend(custom_response_attributes);

            layer_state
                .server_request_duration
                .record(duration_start.elapsed().as_secs_f64(), &label_superset);

            if let Some(req_content_length) = req_body_size {
                layer_state
                    .server_request_body_size
                    .record(req_content_length, &label_superset);
            }

            if let Some(resp_content_length) = response.body().size_hint().exact() {
                layer_state
                    .server_response_body_size
                    .record(resp_content_length, &label_superset);
            }

            layer_state
                .server_active_requests
                .add(-1, &[url_scheme_kv, method_kv]);
        }
        Err(error) => {
            // Mark span as error
            span.set_status(Status::Error {
                description: format!("{:?}", error).into(),
            });

            // Still record duration metric (without status code).
            let label_superset = [
                protocol_name_kv,
                protocol_version_kv,
                url_scheme_kv.clone(),
                method_kv.clone(),
            ];

            layer_state
                .server_request_duration
                .record(duration_start.elapsed().as_secs_f64(), &label_superset);

            layer_state
                .server_active_requests
                .add(-1, &[url_scheme_kv, method_kv]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::http::extractors::{NoRouteExtractor, PathExtractor};
    use crate::Error;

    use http::{Request, Response, StatusCode};
    use opentelemetry::global::BoxedTracer;
    use opentelemetry::trace::Span;
    use opentelemetry::trace::TracerProvider;
    use opentelemetry::trace::{FutureExt, TraceContextExt, Tracer};
    use opentelemetry_http::HeaderInjector;
    use opentelemetry_sdk::metrics::SdkMeterProvider;
    use opentelemetry_sdk::metrics::{
        data::{AggregatedMetrics, MetricData},
        InMemoryMetricExporter, PeriodicReader,
    };
    use opentelemetry_sdk::propagation::TraceContextPropagator;
    use opentelemetry_sdk::trace::{InMemorySpanExporterBuilder, SdkTracerProvider};
    use std::result::Result;
    use std::sync::Mutex;
    use std::time::Duration;
    use tower::{ServiceBuilder, ServiceExt};

    #[cfg(feature = "axum")]
    use crate::http::extractors::AxumMatchedPathExtractor;

    #[tokio::test(flavor = "current_thread")]
    async fn test_tracing_with_in_memory_tracer() {
        let trace_exporter = InMemorySpanExporterBuilder::new().build();
        let tracer_provider = SdkTracerProvider::builder()
            .with_simple_exporter(trace_exporter.clone())
            .build();

        let tracer = Arc::new(BoxedTracer::new(Box::new(
            tracer_provider.tracer("test_tracer"),
        )));

        let layer = LayerBuilder::builder()
            .with_route_extractor(PathExtractor)
            .with_tracer_provider(tracer_provider.clone())
            .build()
            .unwrap();

        let mut service = ServiceBuilder::new()
            .layer(layer)
            .service(tower::service_fn(echo));

        // Create a parent span and set it as the current context
        let parent_span = tracer.start("parent_operation");
        let cx = OtelContext::current_with_span(parent_span);

        let request_body = "test".to_string();
        let request = http::Request::builder()
            .uri("http://example.com/api/users/123")
            .header("Content-Length", request_body.len().to_string())
            .header("User-Agent", "tower-test-client/1.0")
            .body(request_body)
            .unwrap();

        // Execute the service call within the parent span context
        let _response = async { service.ready().await.unwrap().call(request).await.unwrap() }
            .with_context(cx)
            .await;

        tracer_provider.force_flush().unwrap();

        let spans = trace_exporter.get_finished_spans().unwrap();
        assert_eq!(
            spans.len(),
            2,
            "Expected exactly two spans to be recorded (parent + HTTP)"
        );

        // Find the HTTP span (should be the child)
        let http_span = spans
            .iter()
            .find(|span| span.name == "GET /api/users/123")
            .expect("Should find HTTP span");

        // Find the parent span
        let parent_span = spans
            .iter()
            .find(|span| span.name == "parent_operation")
            .expect("Should find parent span");

        // Verify the HTTP span has the correct parent
        assert_eq!(
            http_span.parent_span_id,
            parent_span.span_context.span_id(),
            "HTTP span should have parent span as parent"
        );

        // Verify they share the same trace ID
        assert_eq!(
            http_span.span_context.trace_id(),
            parent_span.span_context.trace_id(),
            "Parent and child spans should share the same trace ID"
        );

        assert_eq!(
            http_span.name, "GET /api/users/123",
            "Span name should match the request"
        );
        // Build expected attributes
        let expected_attributes = vec![
            KeyValue::new(semconv::attribute::HTTP_REQUEST_METHOD, "GET".to_string()),
            KeyValue::new(semconv::attribute::URL_SCHEME, "http".to_string()),
            KeyValue::new(semconv::attribute::URL_PATH, "/api/users/123".to_string()),
            KeyValue::new(
                semconv::attribute::URL_FULL,
                "http://example.com/api/users/123".to_string(),
            ),
            KeyValue::new(
                semconv::attribute::USER_AGENT_ORIGINAL,
                "tower-test-client/1.0".to_string(),
            ),
            KeyValue::new(semconv::attribute::HTTP_ROUTE, "/api/users/123".to_string()),
            KeyValue::new(semconv::attribute::HTTP_RESPONSE_STATUS_CODE, 200),
        ];

        assert_eq!(http_span.attributes, expected_attributes);
    }

    async fn echo(req: http::Request<String>) -> Result<http::Response<String>, Error> {
        Ok(http::Response::new(req.into_body()))
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_metrics_labels() {
        let exporter = InMemoryMetricExporter::default();
        let reader = PeriodicReader::builder(exporter.clone())
            .with_interval(Duration::from_millis(100))
            .build();
        let meter_provider = SdkMeterProvider::builder().with_reader(reader).build();

        let layer = LayerBuilder::builder()
            .with_meter_provider(meter_provider.clone())
            .build()
            .unwrap();

        let service = tower::service_fn(|_req: Request<String>| async {
            Ok::<_, std::convert::Infallible>(
                Response::builder()
                    .status(StatusCode::OK)
                    .body(String::from("Hello, World!"))
                    .unwrap(),
            )
        });

        let mut service = layer.layer(service);

        let request = Request::builder()
            .method("GET")
            .uri("https://example.com/test")
            .body("test body".to_string())
            .unwrap();

        let _response = service.call(request).await.unwrap();

        tokio::time::sleep(Duration::from_millis(500)).await;

        let metrics = exporter.get_finished_metrics().unwrap();
        assert!(!metrics.is_empty());

        let resource_metrics = &metrics[0];
        let scope_metrics = resource_metrics
            .scope_metrics()
            .next()
            .expect("Should have scope metrics");

        let duration_metric = scope_metrics
            .metrics()
            .find(|m| m.name() == semconv::metric::HTTP_SERVER_REQUEST_DURATION)
            .expect("Duration metric should exist");

        if let AggregatedMetrics::F64(MetricData::Histogram(histogram)) = duration_metric.data() {
            let data_point = histogram
                .data_points()
                .next()
                .expect("Should have data point");
            let attributes: Vec<_> = data_point.attributes().collect();

            // Duration metric should have 5 attributes: protocol_name, protocol_version, url_scheme, method, status_code
            assert_eq!(
                attributes.len(),
                5,
                "Duration metric should have exactly 5 attributes"
            );

            let protocol_name = attributes
                .iter()
                .find(|kv| kv.key.as_str() == semconv::attribute::NETWORK_PROTOCOL_NAME)
                .expect("Protocol name should be present");
            assert_eq!(protocol_name.value.as_str(), "http");

            let protocol_version = attributes
                .iter()
                .find(|kv| kv.key.as_str() == semconv::attribute::NETWORK_PROTOCOL_VERSION)
                .expect("Protocol version should be present");
            assert_eq!(protocol_version.value.as_str(), "1.1");

            let url_scheme = attributes
                .iter()
                .find(|kv| kv.key.as_str() == semconv::attribute::URL_SCHEME)
                .expect("URL scheme should be present");
            assert_eq!(url_scheme.value.as_str(), "https");

            let method = attributes
                .iter()
                .find(|kv| kv.key.as_str() == semconv::attribute::HTTP_REQUEST_METHOD)
                .expect("HTTP method should be present");
            assert_eq!(method.value.as_str(), "GET");

            let status_code = attributes
                .iter()
                .find(|kv| kv.key.as_str() == semconv::attribute::HTTP_RESPONSE_STATUS_CODE)
                .expect("Status code should be present");
            if let opentelemetry::Value::I64(code) = &status_code.value {
                assert_eq!(*code, 200);
            } else {
                panic!("Expected i64 status code");
            }
        } else {
            panic!("Expected histogram data for duration metric");
        }

        let request_body_size_metric = scope_metrics
            .metrics()
            .find(|m| m.name() == semconv::metric::HTTP_SERVER_REQUEST_BODY_SIZE);

        if let Some(metric) = request_body_size_metric {
            if let AggregatedMetrics::F64(MetricData::Histogram(histogram)) = metric.data() {
                let data_point = histogram
                    .data_points()
                    .next()
                    .expect("Should have data point");
                let attributes: Vec<_> = data_point.attributes().collect();

                assert_eq!(
                    attributes.len(),
                    5,
                    "Request body size metric should have exactly 5 attributes"
                );

                let method = attributes
                    .iter()
                    .find(|kv| kv.key.as_str() == semconv::attribute::HTTP_REQUEST_METHOD)
                    .expect("HTTP method should be present in request body size");
                assert_eq!(method.value.as_str(), "GET");
            }
        }

        // Test response body size metric
        let response_body_size_metric = scope_metrics
            .metrics()
            .find(|m| m.name() == semconv::metric::HTTP_SERVER_RESPONSE_BODY_SIZE);

        if let Some(metric) = response_body_size_metric {
            if let AggregatedMetrics::F64(MetricData::Histogram(histogram)) = metric.data() {
                let data_point = histogram
                    .data_points()
                    .next()
                    .expect("Should have data point");
                let attributes: Vec<_> = data_point.attributes().collect();

                assert_eq!(
                    attributes.len(),
                    5,
                    "Response body size metric should have exactly 5 attributes"
                );

                let method = attributes
                    .iter()
                    .find(|kv| kv.key.as_str() == semconv::attribute::HTTP_REQUEST_METHOD)
                    .expect("HTTP method should be present in response body size");
                assert_eq!(method.value.as_str(), "GET");
            }
        }

        // Test active requests metric
        let active_requests_metric = scope_metrics
            .metrics()
            .find(|m| m.name() == semconv::metric::HTTP_SERVER_ACTIVE_REQUESTS);

        if let Some(metric) = active_requests_metric {
            if let AggregatedMetrics::I64(MetricData::Sum(sum)) = metric.data() {
                let data_point = sum.data_points().next().expect("Should have data point");
                let attributes: Vec<_> = data_point.attributes().collect();

                assert_eq!(
                    attributes.len(),
                    2,
                    "Active requests metric should have exactly 2 attributes"
                );

                let method = attributes
                    .iter()
                    .find(|kv| kv.key.as_str() == semconv::attribute::HTTP_REQUEST_METHOD)
                    .expect("HTTP method should be present in active requests");
                assert_eq!(method.value.as_str(), "GET");

                let url_scheme = attributes
                    .iter()
                    .find(|kv| kv.key.as_str() == semconv::attribute::URL_SCHEME)
                    .expect("URL scheme should be present in active requests");
                assert_eq!(url_scheme.value.as_str(), "https");
            }
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_context_available_in_handler() {
        let trace_exporter = InMemorySpanExporterBuilder::new().build();
        let tracer_provider = SdkTracerProvider::builder()
            .with_simple_exporter(trace_exporter.clone())
            .build();

        let _tracer = Arc::new(BoxedTracer::new(Box::new(
            tracer_provider.tracer("test_tracer"),
        )));

        let layer = LayerBuilder::builder()
            .with_route_extractor(PathExtractor)
            .with_tracer_provider(tracer_provider.clone())
            .build()
            .unwrap();

        let service = tower::service_fn(|_req: Request<String>| async {
            let cx = OtelContext::current();
            let span = cx.span();

            let span_context = span.span_context();
            assert!(span_context.is_valid(), "Span context should be valid");

            Ok::<_, std::convert::Infallible>(
                Response::builder()
                    .status(StatusCode::OK)
                    .body(String::from("OK"))
                    .unwrap(),
            )
        });

        let mut service = layer.layer(service);

        let request = Request::builder()
            .method("GET")
            .uri("http://example.com/test")
            .body("test".to_string())
            .unwrap();

        let _response = service.call(request).await.unwrap();

        tracer_provider.force_flush().unwrap();

        let spans = trace_exporter.get_finished_spans().unwrap();
        assert_eq!(spans.len(), 1, "Expected one HTTP span");
        assert_eq!(spans[0].name, "GET /test");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_method_only_span_name() {
        let trace_exporter = InMemorySpanExporterBuilder::new().build();
        let tracer_provider = SdkTracerProvider::builder()
            .with_simple_exporter(trace_exporter.clone())
            .build();

        let _tracer = Arc::new(BoxedTracer::new(Box::new(
            tracer_provider.tracer("test_tracer"),
        )));

        let layer = LayerBuilder::builder()
            .with_route_extractor(NoRouteExtractor)
            .with_tracer_provider(tracer_provider.clone())
            .build()
            .unwrap();

        let service = tower::service_fn(|_req: Request<String>| async {
            Ok::<_, std::convert::Infallible>(
                Response::builder()
                    .status(StatusCode::OK)
                    .body(String::from("OK"))
                    .unwrap(),
            )
        });

        let mut service = layer.layer(service);

        let request = Request::builder()
            .method("POST")
            .uri("http://example.com/users/123/orders?include=items")
            .body("test".to_string())
            .unwrap();

        let _response = service.call(request).await.unwrap();

        tracer_provider.force_flush().unwrap();

        let spans = trace_exporter.get_finished_spans().unwrap();
        assert_eq!(spans.len(), 1, "Expected one HTTP span");
        assert_eq!(spans[0].name, "POST");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_path_span_name_strips_query_params() {
        let trace_exporter = InMemorySpanExporterBuilder::new().build();
        let tracer_provider = SdkTracerProvider::builder()
            .with_simple_exporter(trace_exporter.clone())
            .build();

        let _tracer = Arc::new(BoxedTracer::new(Box::new(
            tracer_provider.tracer("test_tracer"),
        )));

        let layer = LayerBuilder::builder()
            .with_route_extractor(PathExtractor)
            .with_tracer_provider(tracer_provider.clone())
            .build()
            .unwrap();

        let service = tower::service_fn(|_req: Request<String>| async {
            Ok::<_, std::convert::Infallible>(
                Response::builder()
                    .status(StatusCode::OK)
                    .body(String::from("OK"))
                    .unwrap(),
            )
        });

        let mut service = layer.layer(service);

        let request = Request::builder()
            .method("GET")
            .uri("http://example.com/users?page=1&limit=10")
            .body("test".to_string())
            .unwrap();

        let _response = service.call(request).await.unwrap();

        tracer_provider.force_flush().unwrap();

        let spans = trace_exporter.get_finished_spans().unwrap();
        assert_eq!(spans.len(), 1, "Expected one HTTP span");
        assert_eq!(spans[0].name, "GET /users");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_custom_span_name_extractor() {
        let trace_exporter = InMemorySpanExporterBuilder::new().build();
        let tracer_provider = SdkTracerProvider::builder()
            .with_simple_exporter(trace_exporter.clone())
            .build();

        let _tracer = Arc::new(BoxedTracer::new(Box::new(
            tracer_provider.tracer("test_tracer"),
        )));

        let layer = LayerBuilder::builder()
            .with_route_extractor_fn(|req: &Request<String>| {
                let path = req.uri().path();
                let normalized = path
                    .split('/')
                    .map(|segment| {
                        if segment.parse::<u64>().is_ok() {
                            "{id}"
                        } else {
                            segment
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("/");
                Some(normalized)
            })
            .with_tracer_provider(tracer_provider.clone())
            .build()
            .unwrap();

        let service = tower::service_fn(|_req: Request<String>| async {
            Ok::<_, std::convert::Infallible>(
                Response::builder()
                    .status(StatusCode::OK)
                    .body(String::from("OK"))
                    .unwrap(),
            )
        });

        let mut service = layer.layer(service);

        let request = Request::builder()
            .method("GET")
            .uri("http://example.com/users/12345/orders/67890")
            .body("test".to_string())
            .unwrap();

        let _response = service.call(request).await.unwrap();

        tracer_provider.force_flush().unwrap();

        let spans = trace_exporter.get_finished_spans().unwrap();
        assert_eq!(spans.len(), 1, "Expected one HTTP span");
        assert_eq!(spans[0].name, "GET /users/{id}/orders/{id}");
    }

    #[cfg(feature = "axum")]
    #[tokio::test(flavor = "current_thread")]
    async fn test_axum_matched_path_extractor_fallback() {
        let trace_exporter = InMemorySpanExporterBuilder::new().build();
        let tracer_provider = SdkTracerProvider::builder()
            .with_simple_exporter(trace_exporter.clone())
            .build();

        let _tracer = Arc::new(BoxedTracer::new(Box::new(
            tracer_provider.tracer("test_tracer"),
        )));

        let layer = LayerBuilder::builder()
            .with_route_extractor(AxumMatchedPathExtractor)
            .with_tracer_provider(tracer_provider.clone())
            .build()
            .unwrap();

        let service = tower::service_fn(|_req: Request<String>| async {
            Ok::<_, std::convert::Infallible>(
                Response::builder()
                    .status(StatusCode::OK)
                    .body(String::from("OK"))
                    .unwrap(),
            )
        });

        let mut service = layer.layer(service);

        let request = Request::builder()
            .method("GET")
            .uri("http://example.com/users/123")
            .body("test".to_string())
            .unwrap();

        let _response = service.call(request).await.unwrap();

        tracer_provider.force_flush().unwrap();

        let spans = trace_exporter.get_finished_spans().unwrap();
        assert_eq!(spans.len(), 1, "Expected one HTTP span");
        assert_eq!(spans[0].name, "GET");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_with_tracing_false_produces_no_spans() {
        let trace_exporter = InMemorySpanExporterBuilder::new().build();
        let tracer_provider = SdkTracerProvider::builder()
            .with_simple_exporter(trace_exporter.clone())
            .build();

        let layer = LayerBuilder::builder()
            .with_route_extractor(PathExtractor)
            .with_tracer_provider(tracer_provider.clone())
            .with_tracing(false)
            .build()
            .unwrap();

        let service = tower::service_fn(|_req: Request<String>| async {
            Ok::<_, std::convert::Infallible>(
                Response::builder()
                    .status(StatusCode::OK)
                    .body(String::from("OK"))
                    .unwrap(),
            )
        });

        let mut service = layer.layer(service);

        let request = Request::builder()
            .method("GET")
            .uri("http://example.com/test")
            .body("test".to_string())
            .unwrap();

        let _response = service.call(request).await.unwrap();

        tracer_provider.force_flush().unwrap();

        let spans = trace_exporter.get_finished_spans().unwrap();
        assert!(
            spans.is_empty(),
            "Expected no spans when tracing is disabled"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_with_metrics_false_produces_no_metrics() {
        let exporter = InMemoryMetricExporter::default();
        let reader = PeriodicReader::builder(exporter.clone())
            .with_interval(Duration::from_millis(100))
            .build();
        let meter_provider = SdkMeterProvider::builder().with_reader(reader).build();

        let layer = LayerBuilder::builder()
            .with_meter_provider(meter_provider.clone())
            .with_metrics(false)
            .build()
            .unwrap();

        let service = tower::service_fn(|_req: Request<String>| async {
            Ok::<_, std::convert::Infallible>(
                Response::builder()
                    .status(StatusCode::OK)
                    .body(String::from("OK"))
                    .unwrap(),
            )
        });

        let mut service = layer.layer(service);

        let request = Request::builder()
            .method("GET")
            .uri("https://example.com/test")
            .body("test body".to_string())
            .unwrap();

        let _response = service.call(request).await.unwrap();

        tokio::time::sleep(Duration::from_millis(500)).await;

        let metrics = exporter.get_finished_metrics().unwrap();
        assert!(
            metrics.is_empty(),
            "Expected no metrics when metrics is disabled"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_with_tracing_false_still_propagates_context() {
        let trace_exporter = InMemorySpanExporterBuilder::new().build();
        let tracer_provider = SdkTracerProvider::builder()
            .with_simple_exporter(trace_exporter.clone())
            .build();

        let tracer = Arc::new(BoxedTracer::new(Box::new(
            tracer_provider.tracer("test_tracer"),
        )));

        // Set a real global propagator so the layer can extract context from headers.
        global::set_text_map_propagator(TraceContextPropagator::new());

        let layer = LayerBuilder::builder()
            .with_route_extractor(PathExtractor)
            .with_tracer_provider(tracer_provider.clone())
            .with_tracing(false)
            .build()
            .unwrap();

        // Create a parent span and inject its context into the request headers.
        let parent_span = tracer.start("parent_operation");
        let parent_span_id = parent_span.span_context().span_id();
        let parent_cx = OtelContext::current_with_span(parent_span);

        let mut request = http::Request::builder()
            .method("GET")
            .uri("http://example.com/test")
            .body("test".to_string())
            .unwrap();
        global::get_text_map_propagator(|propagator| {
            propagator.inject_context(&parent_cx, &mut HeaderInjector(request.headers_mut()));
        });

        let expected_span_id = Arc::new(Mutex::new(parent_span_id));
        let expected_span_id_clone = expected_span_id.clone();

        let service = tower::service_fn(|_req: Request<String>| async {
            // Even with tracing disabled, the extracted parent context must
            // be available as the current context inside the handler.
            let cx = OtelContext::current();
            let span = cx.span();
            let span_context = span.span_context();
            assert_eq!(
                span_context.span_id(),
                *expected_span_id_clone.lock().unwrap(),
                "Handler should see the same parent span ID"
            );

            Ok::<_, std::convert::Infallible>(
                Response::builder()
                    .status(StatusCode::OK)
                    .body(String::from("OK"))
                    .unwrap(),
            )
        });

        let mut service = layer.layer(service);

        let _response = service.call(request).await.unwrap();

        tracer_provider.force_flush().unwrap();

        // No spans should be recorded by the layer (tracing disabled).
        let spans = trace_exporter.get_finished_spans().unwrap();
        assert!(
            spans.is_empty(),
            "Expected no spans when tracing is disabled"
        );
    }
}
