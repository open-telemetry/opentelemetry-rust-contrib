//! HTTP client instrumentation layer.
//!
//! Produces a `SpanKind::Client` span and the standard HTTP client metrics for
//! every outgoing request, and injects the current trace context into the
//! outgoing request headers so downstream servers can continue the trace.

use std::borrow::Cow;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Instant;

use opentelemetry::global::BoxedTracer;
use opentelemetry::metrics::{Histogram, Meter};
use opentelemetry::trace::{SpanKind, TraceContextExt, Tracer};
use opentelemetry::Context as OtelContext;
use opentelemetry::KeyValue;
use opentelemetry_semantic_conventions as semconv;
use pin_project_lite::pin_project;
use tower_layer::Layer as TowerLayer;
use tower_service::Service as TowerService;

use crate::common::attributes::{
    method_kv, split_and_format_protocol_version, url_scheme_kv, HTTP_RESPONSE_STATUS_CODE_LABEL,
    HTTP_ROUTE_LABEL, NETWORK_PROTOCOL_NAME_LABEL, NETWORK_PROTOCOL_VERSION_LABEL,
    SERVER_ADDRESS_LABEL, SERVER_PORT_LABEL,
};
use crate::common::{instrumentation, propagation, status};
use crate::http::extractors::{
    DefaultRouteExtractor, NoOpExtractor, RequestAttributeExtractor, ResponseAttributeExtractor,
    RouteExtractor,
};
use crate::Result;

const HTTP_CLIENT_DURATION_METRIC: &str = semconv::metric::HTTP_CLIENT_REQUEST_DURATION;
const HTTP_CLIENT_DURATION_UNIT: &str = "s";

const OTEL_DEFAULT_HTTP_CLIENT_DURATION_BOUNDS: [f64; 14] = [
    0.005, 0.01, 0.025, 0.05, 0.075, 0.1, 0.25, 0.5, 0.75, 1.0, 2.5, 5.0, 7.5, 10.0,
];

const HTTP_CLIENT_REQUEST_BODY_SIZE_METRIC: &str = semconv::metric::HTTP_CLIENT_REQUEST_BODY_SIZE;
const HTTP_CLIENT_REQUEST_BODY_SIZE_UNIT: &str = "By";

const HTTP_CLIENT_RESPONSE_BODY_SIZE_METRIC: &str = semconv::metric::HTTP_CLIENT_RESPONSE_BODY_SIZE;
const HTTP_CLIENT_RESPONSE_BODY_SIZE_UNIT: &str = "By";

/// State scoped to the entire middleware [`Layer`].
struct LayerState {
    client_request_duration: Histogram<f64>,
    client_request_body_size: Histogram<u64>,
    client_response_body_size: Histogram<u64>,
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
/// [`tower_layer::Layer`] which applies OpenTelemetry HTTP client metrics and tracing.
pub struct Layer<RouteExt = DefaultRouteExtractor, ReqExt = NoOpExtractor, ResExt = NoOpExtractor> {
    state: Arc<LayerState>,
    route_extractor: RouteExt,
    request_extractor: ReqExt,
    response_extractor: ResExt,
    tracer: Arc<BoxedTracer>,
}

impl Layer {
    /// Create a new HTTP client layer with default configuration using global providers.
    pub fn new() -> Self {
        LayerBuilder::builder().build().unwrap()
    }
}

impl Default for Layer {
    fn default() -> Self {
        Self::new()
    }
}

/// Builder for the HTTP client [`Layer`].
pub struct LayerBuilder<
    RouteExt = DefaultRouteExtractor,
    ReqExt = NoOpExtractor,
    ResExt = NoOpExtractor,
> {
    meter: Option<Meter>,
    req_dur_bounds: Option<Vec<f64>>,
    route_extractor: RouteExt,
    request_extractor: ReqExt,
    response_extractor: ResExt,
}

impl LayerBuilder {
    pub fn builder() -> Self {
        LayerBuilder {
            meter: None,
            req_dur_bounds: Some(Vec::from(OTEL_DEFAULT_HTTP_CLIENT_DURATION_BOUNDS)),
            route_extractor: DefaultRouteExtractor::default(),
            request_extractor: NoOpExtractor,
            response_extractor: NoOpExtractor,
        }
    }
}

impl<RouteExt, ReqExt, ResExt> LayerBuilder<RouteExt, ReqExt, ResExt> {
    /// Set a custom route extractor.
    ///
    /// The extracted route is used for the span name (`"{method} {route}"`) and
    /// the `http.route` attribute. For clients this is typically a request
    /// target template rather than the concrete path; choose a low-cardinality
    /// value (see [`RouteExtractor`]).
    pub fn with_route_extractor<NewRoute>(
        self,
        extractor: NewRoute,
    ) -> LayerBuilder<NewRoute, ReqExt, ResExt> {
        LayerBuilder {
            meter: self.meter,
            req_dur_bounds: self.req_dur_bounds,
            route_extractor: extractor,
            request_extractor: self.request_extractor,
            response_extractor: self.response_extractor,
        }
    }

    /// Convenience method to set a function-based route extractor.
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
            meter: self.meter,
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
            meter: self.meter,
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
            .unwrap_or_else(|| Vec::from(OTEL_DEFAULT_HTTP_CLIENT_DURATION_BOUNDS));

        let tracer = instrumentation::tracer();

        let meter: Meter = self.meter.unwrap_or_else(instrumentation::meter);

        Ok(Layer {
            state: Arc::from(Self::make_state(meter, req_dur_bounds)),
            route_extractor: self.route_extractor,
            request_extractor: self.request_extractor,
            response_extractor: self.response_extractor,
            tracer,
        })
    }

    /// Override the meter used for metrics collection (test-only).
    #[cfg(test)]
    fn with_meter(mut self, meter: Meter) -> Self {
        self.meter = Some(meter);
        self
    }

    fn make_state(meter: Meter, req_dur_bounds: Vec<f64>) -> LayerState {
        LayerState {
            client_request_duration: meter
                .f64_histogram(Cow::from(HTTP_CLIENT_DURATION_METRIC))
                .with_description("Duration of HTTP client requests.")
                .with_unit(Cow::from(HTTP_CLIENT_DURATION_UNIT))
                .with_boundaries(req_dur_bounds)
                .build(),
            client_request_body_size: meter
                .u64_histogram(HTTP_CLIENT_REQUEST_BODY_SIZE_METRIC)
                .with_description("Size of HTTP client request bodies.")
                .with_unit(HTTP_CLIENT_REQUEST_BODY_SIZE_UNIT)
                .build(),
            client_response_body_size: meter
                .u64_histogram(HTTP_CLIENT_RESPONSE_BODY_SIZE_METRIC)
                .with_description("Size of HTTP client response bodies.")
                .with_unit(HTTP_CLIENT_RESPONSE_BODY_SIZE_UNIT)
                .build(),
        }
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

/// Request data captured before the inner client call, needed to record metrics
/// and finalize the span once the response arrives.
struct RequestData {
    duration_start: Instant,
    req_body_size: Option<u64>,

    protocol_name_kv: KeyValue,
    protocol_version_kv: KeyValue,
    url_scheme_kv: KeyValue,
    method_kv: KeyValue,
    route_kv_opt: Option<KeyValue>,
    server_address_kv_opt: Option<KeyValue>,
    server_port_kv_opt: Option<KeyValue>,

    custom_request_attributes: Vec<KeyValue>,
}

struct RequestFinalization<ResExt> {
    request_data: RequestData,
    layer_state: Arc<LayerState>,
    response_extractor: ResExt,
}

pin_project! {
    /// Future returned by the client [`Service`].
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
        if let Some(fin) = this.finalization.take() {
            finalize_request(
                &result,
                &fin.request_data,
                &fin.layer_state,
                &fin.response_extractor,
            );
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

    fn call(&mut self, mut req: http::Request<ReqBody>) -> Self::Future {
        let duration_start = Instant::now();

        let content_length = req
            .headers()
            .get(http::header::CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok()?.parse::<u64>().ok());

        let (protocol, version) = split_and_format_protocol_version(req.version());
        let protocol_name_kv = KeyValue::new(NETWORK_PROTOCOL_NAME_LABEL, protocol);
        let protocol_version_kv = KeyValue::new(NETWORK_PROTOCOL_VERSION_LABEL, version);

        let url_scheme_kv = url_scheme_kv(req.uri());

        let method = req.method().as_str().to_owned();
        let method_kv = method_kv(req.method());

        let server_address_kv_opt = req
            .uri()
            .host()
            .map(|host| KeyValue::new(SERVER_ADDRESS_LABEL, host.to_owned()));
        let server_port_kv_opt = req
            .uri()
            .port_u16()
            .map(|port| KeyValue::new(SERVER_PORT_LABEL, i64::from(port)));

        let route = self.route_extractor.extract_route(&req);
        let route_kv_opt = route
            .as_ref()
            .map(|r| KeyValue::new(HTTP_ROUTE_LABEL, r.clone()));

        let span_name = match &route {
            Some(r) => format!("{} {}", method, r),
            None => method.clone(),
        };

        let custom_request_attributes = self.request_extractor.extract_attributes(&req);

        let mut span_attributes = vec![
            KeyValue::new(semconv::trace::HTTP_REQUEST_METHOD, method.clone()),
            KeyValue::new(semconv::trace::URL_FULL, req.uri().to_string()),
            url_scheme_kv.clone(),
        ];
        if let Some(server_address_kv) = &server_address_kv_opt {
            span_attributes.push(server_address_kv.clone());
        }
        if let Some(server_port_kv) = &server_port_kv_opt {
            span_attributes.push(server_port_kv.clone());
        }
        if let Some(r) = &route {
            span_attributes.push(KeyValue::new(HTTP_ROUTE_LABEL, r.clone()));
        }
        span_attributes.extend(custom_request_attributes.clone());

        // The client span is a child of whatever context is currently active.
        let parent_cx = OtelContext::current();
        let span = self
            .tracer
            .span_builder(span_name)
            .with_kind(SpanKind::Client)
            .with_attributes(span_attributes)
            .start_with_context(self.tracer.as_ref(), &parent_cx);

        let cx = parent_cx.with_span(span);

        // Inject the client span context into the outgoing request headers so the
        // downstream server can continue the trace.
        propagation::inject(&cx, req.headers_mut());

        let request_data = RequestData {
            duration_start,
            req_body_size: content_length,
            protocol_name_kv,
            protocol_version_kv,
            url_scheme_kv,
            method_kv,
            route_kv_opt,
            server_address_kv_opt,
            server_port_kv_opt,
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

fn finalize_request<ResBody, E, ResExt>(
    result: &std::result::Result<http::Response<ResBody>, E>,
    request_data: &RequestData,
    layer_state: &Arc<LayerState>,
    response_extractor: &ResExt,
) where
    ResBody: http_body::Body,
    ResExt: ResponseAttributeExtractor<ResBody>,
    E: std::fmt::Debug,
{
    let cx = OtelContext::current();
    let span = cx.span();

    let mut base_labels = vec![
        request_data.protocol_name_kv.clone(),
        request_data.protocol_version_kv.clone(),
        request_data.url_scheme_kv.clone(),
        request_data.method_kv.clone(),
    ];
    if let Some(route_kv) = &request_data.route_kv_opt {
        base_labels.push(route_kv.clone());
    }
    if let Some(server_address_kv) = &request_data.server_address_kv_opt {
        base_labels.push(server_address_kv.clone());
    }
    if let Some(server_port_kv) = &request_data.server_port_kv_opt {
        base_labels.push(server_port_kv.clone());
    }

    match result {
        Ok(response) => {
            let http_status = response.status();

            let mut label_superset = base_labels;
            label_superset.push(KeyValue::new(
                HTTP_RESPONSE_STATUS_CODE_LABEL,
                i64::from(http_status.as_u16()),
            ));
            label_superset.extend(request_data.custom_request_attributes.clone());

            let custom_response_attributes = response_extractor.extract_attributes(response);
            label_superset.extend(custom_response_attributes.clone());

            span.set_attribute(KeyValue::new(
                semconv::trace::HTTP_RESPONSE_STATUS_CODE,
                http_status.as_u16() as i64,
            ));

            for attr in &custom_response_attributes {
                span.set_attribute(attr.clone());
            }

            if let Some(span_status) = status::client_status(http_status) {
                span.set_status(span_status);
            }

            layer_state.client_request_duration.record(
                request_data.duration_start.elapsed().as_secs_f64(),
                &label_superset,
            );

            if let Some(req_content_length) = request_data.req_body_size {
                layer_state
                    .client_request_body_size
                    .record(req_content_length, &label_superset);
            }

            if let Some(resp_content_length) = response.body().size_hint().exact() {
                layer_state
                    .client_response_body_size
                    .record(resp_content_length, &label_superset);
            }
        }
        Err(error) => {
            span.set_status(opentelemetry::trace::Status::Error {
                description: format!("{:?}", error).into(),
            });

            layer_state.client_request_duration.record(
                request_data.duration_start.elapsed().as_secs_f64(),
                &base_labels,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::http::extractors::PathExtractor;

    use crate::common::attributes::HTTP_REQUEST_METHOD_LABEL;
    use http::{Request, Response, StatusCode};
    use opentelemetry::global::BoxedTracer;
    use opentelemetry::metrics::MeterProvider;
    use opentelemetry::trace::{FutureExt, SpanKind, TraceContextExt, Tracer, TracerProvider};
    use opentelemetry_sdk::metrics::{
        data::{AggregatedMetrics, MetricData},
        InMemoryMetricExporter, PeriodicReader, SdkMeterProvider,
    };
    use opentelemetry_sdk::propagation::TraceContextPropagator;
    use opentelemetry_sdk::trace::{InMemorySpanExporterBuilder, SdkTracerProvider};
    use std::time::Duration;
    use tower::{ServiceBuilder, ServiceExt};

    #[tokio::test(flavor = "current_thread")]
    async fn test_client_span_is_child_with_client_kind() {
        let trace_exporter = InMemorySpanExporterBuilder::new().build();
        let tracer_provider = SdkTracerProvider::builder()
            .with_simple_exporter(trace_exporter.clone())
            .build();
        let tracer = Arc::new(BoxedTracer::new(Box::new(
            tracer_provider.tracer("test_tracer"),
        )));

        let mut layer = LayerBuilder::builder()
            .with_route_extractor(PathExtractor)
            .build()
            .unwrap();
        layer.tracer = tracer.clone();

        let mut service = ServiceBuilder::new()
            .layer(layer)
            .service(tower::service_fn(|_req: Request<String>| async {
                Ok::<_, std::convert::Infallible>(
                    Response::builder()
                        .status(StatusCode::OK)
                        .body(String::from("OK"))
                        .unwrap(),
                )
            }));

        let parent_span = tracer.start("parent_operation");
        let cx = OtelContext::current_with_span(parent_span);

        let request = Request::builder()
            .method("GET")
            .uri("http://example.com/api/users/123")
            .body("test".to_string())
            .unwrap();

        let _response = async { service.ready().await.unwrap().call(request).await.unwrap() }
            .with_context(cx)
            .await;

        tracer_provider.force_flush().unwrap();

        let spans = trace_exporter.get_finished_spans().unwrap();
        assert_eq!(spans.len(), 2, "Expected parent + client span");

        let client_span = spans
            .iter()
            .find(|span| span.name == "GET /api/users/123")
            .expect("Should find client span");
        let parent_span = spans
            .iter()
            .find(|span| span.name == "parent_operation")
            .expect("Should find parent span");

        assert_eq!(client_span.span_kind, SpanKind::Client);
        assert_eq!(
            client_span.parent_span_id,
            parent_span.span_context.span_id()
        );
        assert_eq!(
            client_span.span_context.trace_id(),
            parent_span.span_context.trace_id()
        );

        let expected_attributes = vec![
            KeyValue::new(semconv::trace::HTTP_REQUEST_METHOD, "GET".to_string()),
            KeyValue::new(
                semconv::trace::URL_FULL,
                "http://example.com/api/users/123".to_string(),
            ),
            KeyValue::new(semconv::trace::URL_SCHEME, "http".to_string()),
            KeyValue::new(
                semconv::attribute::SERVER_ADDRESS,
                "example.com".to_string(),
            ),
            KeyValue::new(semconv::trace::HTTP_ROUTE, "/api/users/123".to_string()),
            KeyValue::new(semconv::trace::HTTP_RESPONSE_STATUS_CODE, 200),
        ];
        assert_eq!(client_span.attributes, expected_attributes);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_client_injects_context_into_headers() {
        opentelemetry::global::set_text_map_propagator(TraceContextPropagator::new());

        let tracer_provider = SdkTracerProvider::builder().build();
        let tracer = Arc::new(BoxedTracer::new(Box::new(
            tracer_provider.tracer("test_tracer"),
        )));

        let mut layer = LayerBuilder::builder().build().unwrap();
        layer.tracer = tracer.clone();

        let mut service = ServiceBuilder::new()
            .layer(layer)
            .service(tower::service_fn(|req: Request<String>| async move {
                assert!(
                    req.headers().contains_key("traceparent"),
                    "outgoing request should carry an injected traceparent header"
                );
                Ok::<_, std::convert::Infallible>(
                    Response::builder()
                        .status(StatusCode::OK)
                        .body(String::from("OK"))
                        .unwrap(),
                )
            }));

        let parent_span = tracer.start("parent_operation");
        let cx = OtelContext::current_with_span(parent_span);

        let request = Request::builder()
            .method("GET")
            .uri("http://example.com/api")
            .body("test".to_string())
            .unwrap();

        let response = async { service.ready().await.unwrap().call(request).await.unwrap() }
            .with_context(cx)
            .await;

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_client_metrics() {
        let exporter = InMemoryMetricExporter::default();
        let reader = PeriodicReader::builder(exporter.clone())
            .with_interval(Duration::from_millis(100))
            .build();
        let meter_provider = SdkMeterProvider::builder().with_reader(reader).build();
        let meter = meter_provider.meter("test");

        let layer = LayerBuilder::builder().with_meter(meter).build().unwrap();

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

        let scope_metrics = metrics[0]
            .scope_metrics()
            .next()
            .expect("Should have scope metrics");

        let duration_metric = scope_metrics
            .metrics()
            .find(|m| m.name() == HTTP_CLIENT_DURATION_METRIC)
            .expect("Client duration metric should exist");

        if let AggregatedMetrics::F64(MetricData::Histogram(histogram)) = duration_metric.data() {
            let data_point = histogram
                .data_points()
                .next()
                .expect("Should have data point");
            let attributes: Vec<_> = data_point.attributes().collect();

            let method = attributes
                .iter()
                .find(|kv| kv.key.as_str() == HTTP_REQUEST_METHOD_LABEL)
                .expect("HTTP method should be present");
            assert_eq!(method.value.as_str(), "GET");

            let server_address = attributes
                .iter()
                .find(|kv| kv.key.as_str() == SERVER_ADDRESS_LABEL)
                .expect("server.address should be present");
            assert_eq!(server_address.value.as_str(), "example.com");

            let status_code = attributes
                .iter()
                .find(|kv| kv.key.as_str() == HTTP_RESPONSE_STATUS_CODE_LABEL)
                .expect("Status code should be present");
            if let opentelemetry::Value::I64(code) = &status_code.value {
                assert_eq!(*code, 200);
            } else {
                panic!("Expected i64 status code");
            }
        } else {
            panic!("Expected histogram data for client duration metric");
        }
    }
}
