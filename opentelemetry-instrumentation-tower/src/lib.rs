use std::borrow::Cow;
use std::future::Future;
use std::pin::Pin;
use std::string::String;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Instant;
use std::{fmt, result};

#[cfg(feature = "axum")]
use axum::extract::MatchedPath;
use opentelemetry::global::{self, BoxedTracer};
use opentelemetry::metrics::Meter;
use opentelemetry::metrics::{Histogram, UpDownCounter};
use opentelemetry::trace::{FutureExt as OtelFutureExt, SpanKind, Status, TraceContextExt, Tracer};
use opentelemetry::Context as OtelContext;
use opentelemetry::KeyValue;
use opentelemetry_http::HeaderExtractor;
use opentelemetry_semantic_conventions as semconv;
use tower_layer::Layer;
use tower_service::Service;

const HTTP_SERVER_DURATION_METRIC: &str = semconv::metric::HTTP_SERVER_REQUEST_DURATION;
const HTTP_SERVER_DURATION_UNIT: &str = "s";

const _OTEL_DEFAULT_HTTP_SERVER_DURATION_BOUNDARIES: [f64; 14] = [
    0.005, 0.01, 0.025, 0.05, 0.075, 0.1, 0.25, 0.5, 0.75, 1.0, 2.5, 5.0, 7.5, 10.0,
];

// OTEL default does not capture duration over 10s - a poor choice for an arbitrary http server;
// we want to capture longer requests with some rough granularity on the upper end.
// These choices are adapted from various recommendations in
// https://github.com/open-telemetry/semantic-conventions/issues/336.
const LIBRARY_DEFAULT_HTTP_SERVER_DURATION_BOUNDARIES: [f64; 14] = [
    0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0, 120.0, 300.0,
];
const HTTP_SERVER_ACTIVE_REQUESTS_METRIC: &str = semconv::metric::HTTP_SERVER_ACTIVE_REQUESTS;
const HTTP_SERVER_ACTIVE_REQUESTS_UNIT: &str = "{request}";

const HTTP_SERVER_REQUEST_BODY_SIZE_METRIC: &str = semconv::metric::HTTP_SERVER_REQUEST_BODY_SIZE;
const HTTP_SERVER_REQUEST_BODY_SIZE_UNIT: &str = "By";

const HTTP_SERVER_RESPONSE_BODY_SIZE_METRIC: &str = semconv::metric::HTTP_SERVER_RESPONSE_BODY_SIZE;
const HTTP_SERVER_RESPONSE_BODY_SIZE_UNIT: &str = "By";

const NETWORK_PROTOCOL_NAME_LABEL: &str = semconv::attribute::NETWORK_PROTOCOL_NAME;
const NETWORK_PROTOCOL_VERSION_LABEL: &str = semconv::attribute::NETWORK_PROTOCOL_VERSION;
const URL_SCHEME_LABEL: &str = semconv::attribute::URL_SCHEME;

const HTTP_REQUEST_METHOD_LABEL: &str = semconv::attribute::HTTP_REQUEST_METHOD;
#[cfg(feature = "axum")]
const HTTP_ROUTE_LABEL: &str = semconv::attribute::HTTP_ROUTE;
const HTTP_RESPONSE_STATUS_CODE_LABEL: &str = semconv::attribute::HTTP_RESPONSE_STATUS_CODE;

/// Trait for extracting custom attributes from HTTP requests
pub trait RequestAttributeExtractor<B>: Clone + Send + Sync + 'static {
    fn extract_attributes(&self, req: &http::Request<B>) -> Vec<KeyValue>;
}

/// Trait for extracting custom attributes from HTTP responses
pub trait ResponseAttributeExtractor<B>: Clone + Send + Sync + 'static {
    fn extract_attributes(&self, res: &http::Response<B>) -> Vec<KeyValue>;
}

/// Default implementation that extracts no attributes
#[derive(Clone)]
pub struct NoOpExtractor;

impl<B> RequestAttributeExtractor<B> for NoOpExtractor {
    fn extract_attributes(&self, _req: &http::Request<B>) -> Vec<KeyValue> {
        vec![]
    }
}

impl<B> ResponseAttributeExtractor<B> for NoOpExtractor {
    fn extract_attributes(&self, _res: &http::Response<B>) -> Vec<KeyValue> {
        vec![]
    }
}

/// A function-based request attribute extractor
#[derive(Clone)]
pub struct FnRequestExtractor<F> {
    extractor: F,
}

impl<F> FnRequestExtractor<F> {
    pub fn new(extractor: F) -> Self {
        Self { extractor }
    }
}

impl<F, B> RequestAttributeExtractor<B> for FnRequestExtractor<F>
where
    F: Fn(&http::Request<B>) -> Vec<KeyValue> + Clone + Send + Sync + 'static,
{
    fn extract_attributes(&self, req: &http::Request<B>) -> Vec<KeyValue> {
        (self.extractor)(req)
    }
}

/// A function-based response attribute extractor
#[derive(Clone)]
pub struct FnResponseExtractor<F> {
    extractor: F,
}

impl<F> FnResponseExtractor<F> {
    pub fn new(extractor: F) -> Self {
        Self { extractor }
    }
}

impl<F, B> ResponseAttributeExtractor<B> for FnResponseExtractor<F>
where
    F: Fn(&http::Response<B>) -> Vec<KeyValue> + Clone + Send + Sync + 'static,
{
    fn extract_attributes(&self, res: &http::Response<B>) -> Vec<KeyValue> {
        (self.extractor)(res)
    }
}

/// State scoped to the entire middleware Layer.
struct HTTPLayerState {
    pub server_request_duration: Histogram<f64>,
    pub server_active_requests: UpDownCounter<i64>,
    pub server_request_body_size: Histogram<u64>,
    pub server_response_body_size: Histogram<u64>,
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

#[derive(Clone)]
/// [`Service`] used by [`HTTPLayer`]
pub struct HTTPService<S, ReqExt = NoOpExtractor, ResExt = NoOpExtractor> {
    pub(crate) state: Arc<HTTPLayerState>,
    request_extractor: ReqExt,
    response_extractor: ResExt,
    inner_service: S,
    tracer: Arc<BoxedTracer>,
}

#[derive(Clone)]
/// [`Layer`] which applies the OTEL HTTP server metrics and tracing middleware
pub struct HTTPLayer<ReqExt = NoOpExtractor, ResExt = NoOpExtractor> {
    state: Arc<HTTPLayerState>,
    request_extractor: ReqExt,
    response_extractor: ResExt,
    tracer: Arc<BoxedTracer>,
}

impl HTTPLayer {
    /// Create a new HTTP layer with default configuration using global providers
    pub fn new() -> Self {
        HTTPLayerBuilder::builder().build().unwrap()
    }
}

impl Default for HTTPLayer {
    fn default() -> Self {
        Self::new()
    }
}

pub struct HTTPLayerBuilder<ReqExt = NoOpExtractor, ResExt = NoOpExtractor> {
    meter: Option<Meter>,
    req_dur_bounds: Option<Vec<f64>>,
    request_extractor: ReqExt,
    response_extractor: ResExt,
}

/// Error typedef to implement `std::error::Error` for `opentelemetry_instrumentation_tower`
pub struct Error {
    #[allow(dead_code)]
    inner: ErrorKind,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self.inner {
            ErrorKind::Other(ref s) => write!(f, "{s}"),
            ErrorKind::Config(ref s) => write!(f, "config error: {s}"),
        }
    }
}

impl std::error::Error for Error {}

/// `Result` typedef to use with the `opentelemetry_instrumentation_tower::Error` type
pub type Result<T> = result::Result<T, Error>;

enum ErrorKind {
    #[allow(dead_code)]
    /// Uncategorized
    Other(String),
    #[allow(dead_code)]
    /// Invalid configuration
    Config(String),
}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_tuple("opentelemetry_instrumentation_tower::Error")
            .finish()
    }
}

impl HTTPLayerBuilder {
    pub fn builder() -> Self {
        HTTPLayerBuilder {
            meter: None,
            req_dur_bounds: Some(LIBRARY_DEFAULT_HTTP_SERVER_DURATION_BOUNDARIES.to_vec()),
            request_extractor: NoOpExtractor,
            response_extractor: NoOpExtractor,
        }
    }
}

impl<ReqExt, ResExt> HTTPLayerBuilder<ReqExt, ResExt> {
    /// Set a request attribute extractor
    pub fn with_request_extractor<NewReqExt, B>(
        self,
        extractor: NewReqExt,
    ) -> HTTPLayerBuilder<NewReqExt, ResExt>
    where
        NewReqExt: RequestAttributeExtractor<B>,
    {
        HTTPLayerBuilder {
            meter: self.meter,
            req_dur_bounds: self.req_dur_bounds,
            request_extractor: extractor,
            response_extractor: self.response_extractor,
        }
    }

    /// Set a response attribute extractor
    pub fn with_response_extractor<NewResExt, B>(
        self,
        extractor: NewResExt,
    ) -> HTTPLayerBuilder<ReqExt, NewResExt>
    where
        NewResExt: ResponseAttributeExtractor<B>,
    {
        HTTPLayerBuilder {
            meter: self.meter,
            req_dur_bounds: self.req_dur_bounds,
            request_extractor: self.request_extractor,
            response_extractor: extractor,
        }
    }

    /// Convenience method to set a function-based request extractor
    pub fn with_request_extractor_fn<F, B>(
        self,
        f: F,
    ) -> HTTPLayerBuilder<FnRequestExtractor<F>, ResExt>
    where
        F: Fn(&http::Request<B>) -> Vec<KeyValue> + Clone + Send + Sync + 'static,
    {
        self.with_request_extractor(FnRequestExtractor::new(f))
    }

    /// Convenience method to set a function-based response extractor
    pub fn with_response_extractor_fn<F, B>(
        self,
        f: F,
    ) -> HTTPLayerBuilder<ReqExt, FnResponseExtractor<F>>
    where
        F: Fn(&http::Response<B>) -> Vec<KeyValue> + Clone + Send + Sync + 'static,
    {
        self.with_response_extractor(FnResponseExtractor::new(f))
    }

    pub fn build(self) -> Result<HTTPLayer<ReqExt, ResExt>> {
        let req_dur_bounds = self
            .req_dur_bounds
            .unwrap_or_else(|| LIBRARY_DEFAULT_HTTP_SERVER_DURATION_BOUNDARIES.to_vec());

        let tracer = Arc::new(global::tracer("opentelemetry-instrumentation-tower"));

        let meter: Meter = self
            .meter
            .unwrap_or_else(|| global::meter("opentelemetry-instrumentation-tower"));

        Ok(HTTPLayer {
            state: Arc::from(Self::make_state(meter, req_dur_bounds)),
            request_extractor: self.request_extractor,
            response_extractor: self.response_extractor,
            tracer,
        })
    }

    pub fn with_meter(mut self, meter: Meter) -> Self {
        self.meter = Some(meter);
        self
    }

    pub fn with_request_duration_bounds(mut self, bounds: Vec<f64>) -> Self {
        self.req_dur_bounds = Some(bounds);
        self
    }

    fn make_state(meter: Meter, req_dur_bounds: Vec<f64>) -> HTTPLayerState {
        HTTPLayerState {
            server_request_duration: meter
                .f64_histogram(Cow::from(HTTP_SERVER_DURATION_METRIC))
                .with_description("Duration of HTTP server requests.")
                .with_unit(Cow::from(HTTP_SERVER_DURATION_UNIT))
                .with_boundaries(req_dur_bounds)
                .build(),
            server_active_requests: meter
                .i64_up_down_counter(Cow::from(HTTP_SERVER_ACTIVE_REQUESTS_METRIC))
                .with_description("Number of active HTTP server requests.")
                .with_unit(Cow::from(HTTP_SERVER_ACTIVE_REQUESTS_UNIT))
                .build(),
            server_request_body_size: meter
                .u64_histogram(HTTP_SERVER_REQUEST_BODY_SIZE_METRIC)
                .with_description("Size of HTTP server request bodies.")
                .with_unit(HTTP_SERVER_REQUEST_BODY_SIZE_UNIT)
                .build(),
            server_response_body_size: meter
                .u64_histogram(HTTP_SERVER_RESPONSE_BODY_SIZE_METRIC)
                .with_description("Size of HTTP server response bodies.")
                .with_unit(HTTP_SERVER_RESPONSE_BODY_SIZE_UNIT)
                .build(),
        }
    }
}

impl<S, ReqExt, ResExt> Layer<S> for HTTPLayer<ReqExt, ResExt>
where
    ReqExt: Clone,
    ResExt: Clone,
{
    type Service = HTTPService<S, ReqExt, ResExt>;

    fn layer(&self, service: S) -> Self::Service {
        HTTPService {
            state: self.state.clone(),
            request_extractor: self.request_extractor.clone(),
            response_extractor: self.response_extractor.clone(),
            inner_service: service,
            tracer: self.tracer.clone(),
        }
    }
}

impl<S, ReqBody, ResBody, ReqExt, ResExt> Service<http::Request<ReqBody>>
    for HTTPService<S, ReqExt, ResExt>
where
    S: Service<http::Request<ReqBody>, Response = http::Response<ResBody>>,
    S::Future: Send + 'static,
    S::Error: std::fmt::Debug,
    ResBody: http_body::Body,
    ReqExt: RequestAttributeExtractor<ReqBody>,
    ResExt: ResponseAttributeExtractor<ResBody>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = result::Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<result::Result<(), Self::Error>> {
        self.inner_service.poll_ready(cx)
    }

    fn call(&mut self, req: http::Request<ReqBody>) -> Self::Future {
        let duration_start = Instant::now();

        let headers = req.headers();
        let content_length = headers
            .get(http::header::CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok()?.parse::<u64>().ok());

        let (protocol, version) = split_and_format_protocol_version(req.version());
        let protocol_name_kv = KeyValue::new(NETWORK_PROTOCOL_NAME_LABEL, protocol);
        let protocol_version_kv = KeyValue::new(NETWORK_PROTOCOL_VERSION_LABEL, version);

        let scheme = req.uri().scheme_str().unwrap_or("").to_string();
        let url_scheme_kv = KeyValue::new(URL_SCHEME_LABEL, scheme);

        let method = req.method().as_str().to_owned();
        let method_kv = KeyValue::new(HTTP_REQUEST_METHOD_LABEL, method.clone());

        #[cfg(feature = "axum")]
        let route_kv_opt = req
            .extensions()
            .get::<MatchedPath>()
            .map(|matched_path| KeyValue::new(HTTP_ROUTE_LABEL, matched_path.as_str().to_owned()));

        #[cfg(not(feature = "axum"))]
        let route_kv_opt = None;

        // Extract custom request attributes
        let custom_request_attributes = self.request_extractor.extract_attributes(&req);

        // Extract the context from the incoming request headers
        let parent_cx = global::get_text_map_propagator(|propagator| {
            propagator.extract(&HeaderExtractor(req.headers()))
        });

        let mut span_attributes = vec![
            KeyValue::new(semconv::trace::HTTP_REQUEST_METHOD, method.clone()),
            url_scheme_kv.clone(),
            KeyValue::new(semconv::attribute::URL_PATH, req.uri().path().to_string()),
            KeyValue::new(semconv::trace::URL_FULL, req.uri().to_string()),
        ];

        if let Some(user_agent) = req
            .headers()
            .get("user-agent")
            .and_then(|v| v.to_str().ok())
        {
            span_attributes.push(KeyValue::new(
                semconv::trace::USER_AGENT_ORIGINAL,
                user_agent.to_string(),
            ));
        }

        span_attributes.extend(custom_request_attributes.clone());

        let span_name = format!("{} {}", method, req.uri().path());

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

        Box::pin(
            async move {
                let result = inner_future.await;
                finalize_request(&result, &request_data, &layer_state, &response_extractor);
                result
            }
            .with_context(cx),
        )
    }
}

/// Finalizes the request by updating the span and recording metrics after the response is received.
fn finalize_request<ResBody, E, ResExt>(
    result: &result::Result<http::Response<ResBody>, E>,
    request_data: &RequestData,
    layer_state: &Arc<HTTPLayerState>,
    response_extractor: &ResExt,
) where
    ResBody: http_body::Body,
    ResExt: ResponseAttributeExtractor<ResBody>,
    E: std::fmt::Debug,
{
    let cx = OtelContext::current();
    let span = cx.span();

    match result {
        Ok(response) => {
            let status = response.status();

            // Build base label set
            let mut label_superset = vec![
                request_data.protocol_name_kv.clone(),
                request_data.protocol_version_kv.clone(),
                request_data.url_scheme_kv.clone(),
                request_data.method_kv.clone(),
                KeyValue::new(HTTP_RESPONSE_STATUS_CODE_LABEL, i64::from(status.as_u16())),
            ];

            if let Some(route_kv) = &request_data.route_kv_opt {
                label_superset.push(route_kv.clone());
            }

            // Add custom request attributes
            label_superset.extend(request_data.custom_request_attributes.clone());

            // Extract and add custom response attributes
            let custom_response_attributes = response_extractor.extract_attributes(response);
            label_superset.extend(custom_response_attributes.clone());

            // Update span
            span.set_attribute(KeyValue::new(
                semconv::trace::HTTP_RESPONSE_STATUS_CODE,
                status.as_u16() as i64,
            ));

            // Add custom response attributes to span
            for attr in &custom_response_attributes {
                span.set_attribute(attr.clone());
            }

            // Set span status based on HTTP status code
            if status.is_server_error() {
                span.set_status(Status::Error {
                    description: format!("HTTP {}", status.as_u16()).into(),
                });
            }

            // Record metrics
            layer_state.server_request_duration.record(
                request_data.duration_start.elapsed().as_secs_f64(),
                &label_superset,
            );

            if let Some(req_content_length) = request_data.req_body_size {
                layer_state
                    .server_request_body_size
                    .record(req_content_length, &label_superset);
            }

            if let Some(resp_content_length) = response.body().size_hint().exact() {
                layer_state
                    .server_response_body_size
                    .record(resp_content_length, &label_superset);
            }
        }
        Err(error) => {
            // Mark span as error
            span.set_status(Status::Error {
                description: format!("{:?}", error).into(),
            });

            // Still record duration metric with error label
            let label_superset = vec![
                request_data.protocol_name_kv.clone(),
                request_data.protocol_version_kv.clone(),
                request_data.url_scheme_kv.clone(),
                request_data.method_kv.clone(),
            ];

            layer_state.server_request_duration.record(
                request_data.duration_start.elapsed().as_secs_f64(),
                &label_superset,
            );
        }
    }

    // Always decrement active requests counter
    layer_state.server_active_requests.add(
        -1,
        &[
            request_data.url_scheme_kv.clone(),
            request_data.method_kv.clone(),
        ],
    );
}

fn split_and_format_protocol_version(http_version: http::Version) -> (String, String) {
    let version_str = match http_version {
        http::Version::HTTP_09 => "0.9",
        http::Version::HTTP_10 => "1.0",
        http::Version::HTTP_11 => "1.1",
        http::Version::HTTP_2 => "2.0",
        http::Version::HTTP_3 => "3.0",
        _ => "",
    };
    (String::from("http"), String::from(version_str))
}

#[cfg(test)]
mod tests {
    // Tests use optional provider overrides instead of global providers to avoid interference.
    use super::*;

    use http::{Request, Response, StatusCode};
    use opentelemetry::metrics::MeterProvider;
    use opentelemetry::trace::TracerProvider;
    use opentelemetry::trace::{FutureExt, TraceContextExt, Tracer};
    use opentelemetry_sdk::metrics::SdkMeterProvider;
    use opentelemetry_sdk::metrics::{
        data::{AggregatedMetrics, MetricData},
        InMemoryMetricExporter, PeriodicReader,
    };
    use opentelemetry_sdk::trace::{InMemorySpanExporterBuilder, SdkTracerProvider};
    use std::result::Result;
    use std::time::Duration;
    use tower::{Service, ServiceBuilder, ServiceExt};

    #[tokio::test(flavor = "current_thread")]
    async fn test_tracing_with_in_memory_tracer() {
        let trace_exporter = InMemorySpanExporterBuilder::new().build();
        let tracer_provider = SdkTracerProvider::builder()
            .with_simple_exporter(trace_exporter.clone())
            .build();

        let tracer = Arc::new(BoxedTracer::new(Box::new(
            tracer_provider.tracer("test_tracer"),
        )));

        let mut layer = HTTPLayerBuilder::builder().build().unwrap();
        layer.tracer = tracer.clone();

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
            KeyValue::new(semconv::trace::HTTP_REQUEST_METHOD, "GET".to_string()),
            KeyValue::new(semconv::trace::URL_SCHEME, "http".to_string()),
            KeyValue::new(semconv::trace::URL_PATH, "/api/users/123".to_string()),
            KeyValue::new(
                semconv::trace::URL_FULL,
                "http://example.com/api/users/123".to_string(),
            ),
            KeyValue::new(
                semconv::trace::USER_AGENT_ORIGINAL,
                "tower-test-client/1.0".to_string(),
            ),
            KeyValue::new(semconv::trace::HTTP_RESPONSE_STATUS_CODE, 200),
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
        let meter = meter_provider.meter("test");

        let layer = HTTPLayerBuilder::builder()
            .with_meter(meter)
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
            .find(|m| m.name() == HTTP_SERVER_DURATION_METRIC)
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
                .find(|kv| kv.key.as_str() == NETWORK_PROTOCOL_NAME_LABEL)
                .expect("Protocol name should be present");
            assert_eq!(protocol_name.value.as_str(), "http");

            let protocol_version = attributes
                .iter()
                .find(|kv| kv.key.as_str() == NETWORK_PROTOCOL_VERSION_LABEL)
                .expect("Protocol version should be present");
            assert_eq!(protocol_version.value.as_str(), "1.1");

            let url_scheme = attributes
                .iter()
                .find(|kv| kv.key.as_str() == URL_SCHEME_LABEL)
                .expect("URL scheme should be present");
            assert_eq!(url_scheme.value.as_str(), "https");

            let method = attributes
                .iter()
                .find(|kv| kv.key.as_str() == HTTP_REQUEST_METHOD_LABEL)
                .expect("HTTP method should be present");
            assert_eq!(method.value.as_str(), "GET");

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
            panic!("Expected histogram data for duration metric");
        }

        let request_body_size_metric = scope_metrics
            .metrics()
            .find(|m| m.name() == HTTP_SERVER_REQUEST_BODY_SIZE_METRIC);

        if let Some(metric) = request_body_size_metric {
            if let AggregatedMetrics::F64(MetricData::Histogram(histogram)) = metric.data() {
                let data_point = histogram
                    .data_points()
                    .next()
                    .expect("Should have data point");
                let attributes: Vec<_> = data_point.attributes().collect();

                // Request body size metric should have 5 attributes: protocol_name, protocol_version, url_scheme, method, status_code
                assert_eq!(
                    attributes.len(),
                    5,
                    "Request body size metric should have exactly 5 attributes"
                );

                let protocol_name = attributes
                    .iter()
                    .find(|kv| kv.key.as_str() == NETWORK_PROTOCOL_NAME_LABEL)
                    .expect("Protocol name should be present in request body size");
                assert_eq!(protocol_name.value.as_str(), "https");

                let protocol_version = attributes
                    .iter()
                    .find(|kv| kv.key.as_str() == NETWORK_PROTOCOL_VERSION_LABEL)
                    .expect("Protocol version should be present in request body size");
                assert_eq!(protocol_version.value.as_str(), "1.1");

                let url_scheme = attributes
                    .iter()
                    .find(|kv| kv.key.as_str() == URL_SCHEME_LABEL)
                    .expect("URL scheme should be present in request body size");
                assert_eq!(url_scheme.value.as_str(), "https");

                let method = attributes
                    .iter()
                    .find(|kv| kv.key.as_str() == HTTP_REQUEST_METHOD_LABEL)
                    .expect("HTTP method should be present in request body size");
                assert_eq!(method.value.as_str(), "GET");

                let status_code = attributes
                    .iter()
                    .find(|kv| kv.key.as_str() == HTTP_RESPONSE_STATUS_CODE_LABEL)
                    .expect("Status code should be present in request body size");
                if let opentelemetry::Value::I64(code) = &status_code.value {
                    assert_eq!(*code, 200);
                } else {
                    panic!("Expected i64 status code");
                }
            }
        }

        // Test response body size metric
        let response_body_size_metric = scope_metrics
            .metrics()
            .find(|m| m.name() == HTTP_SERVER_RESPONSE_BODY_SIZE_METRIC);

        if let Some(metric) = response_body_size_metric {
            if let AggregatedMetrics::F64(MetricData::Histogram(histogram)) = metric.data() {
                let data_point = histogram
                    .data_points()
                    .next()
                    .expect("Should have data point");
                let attributes: Vec<_> = data_point.attributes().collect();

                // Response body size metric should have 5 attributes: protocol_name, protocol_version, url_scheme, method, status_code
                assert_eq!(
                    attributes.len(),
                    5,
                    "Response body size metric should have exactly 5 attributes"
                );

                let protocol_name = attributes
                    .iter()
                    .find(|kv| kv.key.as_str() == NETWORK_PROTOCOL_NAME_LABEL)
                    .expect("Protocol name should be present in response body size");
                assert_eq!(protocol_name.value.as_str(), "http");

                let protocol_version = attributes
                    .iter()
                    .find(|kv| kv.key.as_str() == NETWORK_PROTOCOL_VERSION_LABEL)
                    .expect("Protocol version should be present in response body size");
                assert_eq!(protocol_version.value.as_str(), "1.1");

                let url_scheme = attributes
                    .iter()
                    .find(|kv| kv.key.as_str() == URL_SCHEME_LABEL)
                    .expect("URL scheme should be present in response body size");
                assert_eq!(url_scheme.value.as_str(), "https");

                let method = attributes
                    .iter()
                    .find(|kv| kv.key.as_str() == HTTP_REQUEST_METHOD_LABEL)
                    .expect("HTTP method should be present in response body size");
                assert_eq!(method.value.as_str(), "GET");

                let status_code = attributes
                    .iter()
                    .find(|kv| kv.key.as_str() == HTTP_RESPONSE_STATUS_CODE_LABEL)
                    .expect("Status code should be present in response body size");
                if let opentelemetry::Value::I64(code) = &status_code.value {
                    assert_eq!(*code, 200);
                } else {
                    panic!("Expected i64 status code");
                }
            }
        }

        // Test active requests metric
        let active_requests_metric = scope_metrics
            .metrics()
            .find(|m| m.name() == HTTP_SERVER_ACTIVE_REQUESTS_METRIC);

        if let Some(metric) = active_requests_metric {
            if let AggregatedMetrics::I64(MetricData::Sum(sum)) = metric.data() {
                let data_point = sum.data_points().next().expect("Should have data point");
                let attributes: Vec<_> = data_point.attributes().collect();

                // Active requests metric should have 2 attributes: method, url_scheme
                assert_eq!(
                    attributes.len(),
                    2,
                    "Active requests metric should have exactly 2 attributes"
                );

                let method = attributes
                    .iter()
                    .find(|kv| kv.key.as_str() == HTTP_REQUEST_METHOD_LABEL)
                    .expect("HTTP method should be present in active requests");
                assert_eq!(method.value.as_str(), "GET");

                let url_scheme = attributes
                    .iter()
                    .find(|kv| kv.key.as_str() == URL_SCHEME_LABEL)
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

        let tracer = Arc::new(BoxedTracer::new(Box::new(
            tracer_provider.tracer("test_tracer"),
        )));

        let mut layer = HTTPLayerBuilder::builder().build().unwrap();
        layer.tracer = tracer.clone();

        let service = tower::service_fn(|_req: Request<String>| async {
            // Access the current context - this should have the HTTP span
            let cx = OtelContext::current();
            let span = cx.span();

            // Verify we can get span context (means context is attached)
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
}
