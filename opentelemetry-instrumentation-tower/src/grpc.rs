use std::borrow::Cow;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Instant;
use std::{fmt, result};

use opentelemetry::global::{self, BoxedTracer, ObjectSafeTracer};
use opentelemetry::metrics::{Histogram, Meter, MeterProvider};
use opentelemetry::trace::{SpanKind, Status, TraceContextExt, Tracer, TracerProvider};
use opentelemetry::{Context as OtelContext, InstrumentationScope, KeyValue};
use opentelemetry_http::HeaderExtractor;
use opentelemetry_semantic_conventions as semconv;
use pin_project_lite::pin_project;
use tower_layer::Layer;
use tower_service::Service;

const INSTRUMENTATION_NAME: &str = "opentelemetry-instrumentation-tower";

const RPC_SERVER_DURATION_UNIT: &str = "s";

const OTEL_DEFAULT_RPC_SERVER_DURATION_BOUNDS: [f64; 14] = [
    0.005, 0.01, 0.025, 0.05, 0.075, 0.1, 0.25, 0.5, 0.75, 1.0, 2.5, 5.0, 7.5, 10.0,
];

/// Extracts a fully-qualified gRPC method from an HTTP request.
pub trait GRPCMethodExtractor<B>: Clone + Send + Sync + 'static {
    /// Returns the `rpc.method` value when available.
    fn extract_method(&self, req: &http::Request<B>) -> Option<String>;
}

/// Extracts the gRPC method from paths shaped like `/package.Service/Method`.
#[derive(Clone, Default)]
pub struct DefaultGRPCMethodExtractor;

impl<B> GRPCMethodExtractor<B> for DefaultGRPCMethodExtractor {
    fn extract_method(&self, req: &http::Request<B>) -> Option<String> {
        parse_grpc_path(req.uri().path()).map(str::to_owned)
    }
}

/// A function-based gRPC method extractor.
#[derive(Clone)]
pub struct FnGRPCMethodExtractor<F> {
    extractor: F,
}

impl<F> FnGRPCMethodExtractor<F> {
    pub fn new(extractor: F) -> Self {
        Self { extractor }
    }
}

impl<F, B> GRPCMethodExtractor<B> for FnGRPCMethodExtractor<F>
where
    F: Fn(&http::Request<B>) -> Option<String> + Clone + Send + Sync + 'static,
{
    fn extract_method(&self, req: &http::Request<B>) -> Option<String> {
        (self.extractor)(req)
    }
}

/// Trait for extracting custom attributes from gRPC requests.
pub trait GRPCRequestAttributeExtractor<B>: Clone + Send + Sync + 'static {
    fn extract_attributes(&self, req: &http::Request<B>) -> Vec<KeyValue>;
}

/// Trait for extracting custom attributes from gRPC responses.
pub trait GRPCResponseAttributeExtractor<B>: Clone + Send + Sync + 'static {
    fn extract_attributes(&self, res: &http::Response<B>) -> Vec<KeyValue>;
}

/// Default implementation that extracts no attributes.
#[derive(Clone)]
pub struct NoOpGRPCExtractor;

impl<B> GRPCRequestAttributeExtractor<B> for NoOpGRPCExtractor {
    fn extract_attributes(&self, _req: &http::Request<B>) -> Vec<KeyValue> {
        vec![]
    }
}

impl<B> GRPCResponseAttributeExtractor<B> for NoOpGRPCExtractor {
    fn extract_attributes(&self, _res: &http::Response<B>) -> Vec<KeyValue> {
        vec![]
    }
}

/// A function-based gRPC request attribute extractor.
#[derive(Clone)]
pub struct FnGRPCRequestExtractor<F> {
    extractor: F,
}

impl<F> FnGRPCRequestExtractor<F> {
    pub fn new(extractor: F) -> Self {
        Self { extractor }
    }
}

impl<F, B> GRPCRequestAttributeExtractor<B> for FnGRPCRequestExtractor<F>
where
    F: Fn(&http::Request<B>) -> Vec<KeyValue> + Clone + Send + Sync + 'static,
{
    fn extract_attributes(&self, req: &http::Request<B>) -> Vec<KeyValue> {
        (self.extractor)(req)
    }
}

/// A function-based gRPC response attribute extractor.
#[derive(Clone)]
pub struct FnGRPCResponseExtractor<F> {
    extractor: F,
}

impl<F> FnGRPCResponseExtractor<F> {
    pub fn new(extractor: F) -> Self {
        Self { extractor }
    }
}

impl<F, B> GRPCResponseAttributeExtractor<B> for FnGRPCResponseExtractor<F>
where
    F: Fn(&http::Response<B>) -> Vec<KeyValue> + Clone + Send + Sync + 'static,
{
    fn extract_attributes(&self, res: &http::Response<B>) -> Vec<KeyValue> {
        (self.extractor)(res)
    }
}

#[derive(Clone)]
struct GRPCLayerState {
    server_duration: Histogram<f64>,
}

#[derive(Clone)]
/// [`Service`] used by [`GRPCLayer`].
pub struct GRPCService<
    S,
    MethodExt = DefaultGRPCMethodExtractor,
    ReqExt = NoOpGRPCExtractor,
    ResExt = NoOpGRPCExtractor,
> {
    state: Arc<GRPCLayerState>,
    method_extractor: MethodExt,
    request_extractor: ReqExt,
    response_extractor: ResExt,
    inner_service: S,
    tracer: Arc<BoxedTracer>,
}

#[derive(Clone)]
/// [`Layer`] which applies OpenTelemetry gRPC server metrics and tracing middleware.
pub struct GRPCLayer<
    MethodExt = DefaultGRPCMethodExtractor,
    ReqExt = NoOpGRPCExtractor,
    ResExt = NoOpGRPCExtractor,
> {
    state: Arc<GRPCLayerState>,
    method_extractor: MethodExt,
    request_extractor: ReqExt,
    response_extractor: ResExt,
    tracer: Arc<BoxedTracer>,
}

impl GRPCLayer {
    /// Create a new gRPC layer with default configuration using global providers.
    pub fn new() -> Self {
        GRPCLayerBuilder::builder().build()
    }
}

impl Default for GRPCLayer {
    fn default() -> Self {
        Self::new()
    }
}

/// Builder for [`GRPCLayer`].
pub struct GRPCLayerBuilder<
    MethodExt = DefaultGRPCMethodExtractor,
    ReqExt = NoOpGRPCExtractor,
    ResExt = NoOpGRPCExtractor,
> {
    tracer: Option<Arc<BoxedTracer>>,
    meter: Option<Meter>,
    duration_bounds: Option<Vec<f64>>,
    method_extractor: MethodExt,
    request_extractor: ReqExt,
    response_extractor: ResExt,
}

impl GRPCLayerBuilder {
    pub fn builder() -> Self {
        Self {
            tracer: None,
            meter: None,
            duration_bounds: Some(Vec::from(OTEL_DEFAULT_RPC_SERVER_DURATION_BOUNDS)),
            method_extractor: DefaultGRPCMethodExtractor,
            request_extractor: NoOpGRPCExtractor,
            response_extractor: NoOpGRPCExtractor,
        }
    }
}

impl<MethodExt, ReqExt, ResExt> GRPCLayerBuilder<MethodExt, ReqExt, ResExt> {
    /// Set a custom gRPC method extractor.
    pub fn with_method_extractor<NewMethodExt>(
        self,
        extractor: NewMethodExt,
    ) -> GRPCLayerBuilder<NewMethodExt, ReqExt, ResExt> {
        GRPCLayerBuilder {
            tracer: self.tracer,
            meter: self.meter,
            duration_bounds: self.duration_bounds,
            method_extractor: extractor,
            request_extractor: self.request_extractor,
            response_extractor: self.response_extractor,
        }
    }

    /// Convenience method to set a function-based gRPC method extractor.
    pub fn with_method_extractor_fn<F, B>(
        self,
        f: F,
    ) -> GRPCLayerBuilder<FnGRPCMethodExtractor<F>, ReqExt, ResExt>
    where
        F: Fn(&http::Request<B>) -> Option<String> + Clone + Send + Sync + 'static,
    {
        self.with_method_extractor(FnGRPCMethodExtractor::new(f))
    }

    /// Set a request attribute extractor.
    pub fn with_request_extractor<NewReqExt, B>(
        self,
        extractor: NewReqExt,
    ) -> GRPCLayerBuilder<MethodExt, NewReqExt, ResExt>
    where
        NewReqExt: GRPCRequestAttributeExtractor<B>,
    {
        GRPCLayerBuilder {
            tracer: self.tracer,
            meter: self.meter,
            duration_bounds: self.duration_bounds,
            method_extractor: self.method_extractor,
            request_extractor: extractor,
            response_extractor: self.response_extractor,
        }
    }

    /// Convenience method to set a function-based request attribute extractor.
    pub fn with_request_extractor_fn<F, B>(
        self,
        f: F,
    ) -> GRPCLayerBuilder<MethodExt, FnGRPCRequestExtractor<F>, ResExt>
    where
        F: Fn(&http::Request<B>) -> Vec<KeyValue> + Clone + Send + Sync + 'static,
    {
        self.with_request_extractor(FnGRPCRequestExtractor::new(f))
    }

    /// Set a response attribute extractor.
    pub fn with_response_extractor<NewResExt, B>(
        self,
        extractor: NewResExt,
    ) -> GRPCLayerBuilder<MethodExt, ReqExt, NewResExt>
    where
        NewResExt: GRPCResponseAttributeExtractor<B>,
    {
        GRPCLayerBuilder {
            tracer: self.tracer,
            meter: self.meter,
            duration_bounds: self.duration_bounds,
            method_extractor: self.method_extractor,
            request_extractor: self.request_extractor,
            response_extractor: extractor,
        }
    }

    /// Convenience method to set a function-based response attribute extractor.
    pub fn with_response_extractor_fn<F, B>(
        self,
        f: F,
    ) -> GRPCLayerBuilder<MethodExt, ReqExt, FnGRPCResponseExtractor<F>>
    where
        F: Fn(&http::Response<B>) -> Vec<KeyValue> + Clone + Send + Sync + 'static,
    {
        self.with_response_extractor(FnGRPCResponseExtractor::new(f))
    }

    /// Override the tracer provider used for trace collection.
    pub fn with_tracer_provider<P>(mut self, tracer_provider: P) -> Self
    where
        P: TracerProvider,
        P::Tracer: ObjectSafeTracer + Send + Sync + 'static,
    {
        self.tracer = Some(Arc::new(BoxedTracer::new(Box::new(
            tracer_provider.tracer_with_scope(instrumentation_scope()),
        ))));
        self
    }

    /// Override the meter provider used for metrics collection.
    pub fn with_meter_provider(mut self, meter_provider: impl MeterProvider) -> Self {
        self.meter = Some(meter_provider.meter_with_scope(instrumentation_scope()));
        self
    }

    pub fn build(self) -> GRPCLayer<MethodExt, ReqExt, ResExt> {
        let tracer = self
            .tracer
            .unwrap_or_else(|| Arc::new(global::tracer_with_scope(instrumentation_scope())));
        let meter = self
            .meter
            .unwrap_or_else(|| global::meter_with_scope(instrumentation_scope()));
        let duration_bounds = self
            .duration_bounds
            .unwrap_or_else(|| Vec::from(OTEL_DEFAULT_RPC_SERVER_DURATION_BOUNDS));

        GRPCLayer {
            state: Arc::new(GRPCLayerState::new(meter, duration_bounds)),
            method_extractor: self.method_extractor,
            request_extractor: self.request_extractor,
            response_extractor: self.response_extractor,
            tracer,
        }
    }
}

impl GRPCLayerState {
    fn new(meter: Meter, duration_bounds: Vec<f64>) -> Self {
        Self {
            server_duration: meter
                .f64_histogram(Cow::from(semconv::metric::RPC_SERVER_CALL_DURATION))
                .with_description("Duration of inbound gRPC requests.")
                .with_unit(Cow::from(RPC_SERVER_DURATION_UNIT))
                .with_boundaries(duration_bounds)
                .build(),
        }
    }
}

impl<S, MethodExt, ReqExt, ResExt> Layer<S> for GRPCLayer<MethodExt, ReqExt, ResExt>
where
    MethodExt: Clone,
    ReqExt: Clone,
    ResExt: Clone,
{
    type Service = GRPCService<S, MethodExt, ReqExt, ResExt>;

    fn layer(&self, service: S) -> Self::Service {
        GRPCService {
            state: self.state.clone(),
            method_extractor: self.method_extractor.clone(),
            request_extractor: self.request_extractor.clone(),
            response_extractor: self.response_extractor.clone(),
            inner_service: service,
            tracer: self.tracer.clone(),
        }
    }
}

struct RequestData {
    duration_start: Instant,
    system_kv: KeyValue,
    method_kv: KeyValue,
    custom_request_attributes: Vec<KeyValue>,
}

struct RequestFinalization<ResExt> {
    request_data: RequestData,
    layer_state: Arc<GRPCLayerState>,
    response_extractor: ResExt,
}

pin_project! {
    /// Future type returned by [`GRPCService`].
    pub struct GRPCResponseFuture<F, ResExt> {
        #[pin]
        inner: F,
        otel_cx: OtelContext,
        finalization: Option<RequestFinalization<ResExt>>,
    }
}

impl<F, ResBody, E, ResExt> Future for GRPCResponseFuture<F, ResExt>
where
    F: Future<Output = result::Result<http::Response<ResBody>, E>>,
    E: fmt::Debug,
    ResExt: GRPCResponseAttributeExtractor<ResBody>,
{
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        let _guard = this.otel_cx.clone().attach();
        let result = std::task::ready!(this.inner.poll(cx));
        if let Some(fin) = this.finalization.take() {
            finalize_request(
                &result,
                fin.request_data,
                &fin.layer_state,
                &fin.response_extractor,
            );
        }
        Poll::Ready(result)
    }
}

impl<S, ReqBody, ResBody, MethodExt, ReqExt, ResExt> Service<http::Request<ReqBody>>
    for GRPCService<S, MethodExt, ReqExt, ResExt>
where
    S: Service<http::Request<ReqBody>, Response = http::Response<ResBody>>,
    S::Future: Send,
    S::Error: fmt::Debug,
    MethodExt: GRPCMethodExtractor<ReqBody>,
    ReqExt: GRPCRequestAttributeExtractor<ReqBody>,
    ResExt: GRPCResponseAttributeExtractor<ResBody>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = GRPCResponseFuture<S::Future, ResExt>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<result::Result<(), Self::Error>> {
        self.inner_service.poll_ready(cx)
    }

    fn call(&mut self, req: http::Request<ReqBody>) -> Self::Future {
        let duration_start = Instant::now();
        let parent_cx = global::get_text_map_propagator(|propagator| {
            propagator.extract(&HeaderExtractor(req.headers()))
        });

        let rpc_method = self
            .method_extractor
            .extract_method(&req)
            .unwrap_or_else(|| req.uri().path().trim_start_matches('/').to_owned());

        let system_kv = KeyValue::new(semconv::attribute::RPC_SYSTEM_NAME, "grpc");
        let method_kv = KeyValue::new(semconv::attribute::RPC_METHOD, rpc_method.clone());
        let custom_request_attributes = self.request_extractor.extract_attributes(&req);

        let mut span_attributes = Vec::with_capacity(2 + custom_request_attributes.len());
        span_attributes.push(system_kv.clone());
        if !rpc_method.is_empty() {
            span_attributes.push(method_kv.clone());
        }
        span_attributes.extend(custom_request_attributes.iter().cloned());

        let span_name = if rpc_method.is_empty() {
            req.uri().path().to_owned()
        } else {
            rpc_method.clone()
        };

        let span = self
            .tracer
            .span_builder(span_name)
            .with_kind(SpanKind::Server)
            .with_attributes(span_attributes)
            .start_with_context(self.tracer.as_ref(), &parent_cx);
        let otel_cx = parent_cx.with_span(span);

        let request_data = RequestData {
            duration_start,
            system_kv,
            method_kv,
            custom_request_attributes,
        };
        let layer_state = self.state.clone();
        let response_extractor = self.response_extractor.clone();
        let inner = self.inner_service.call(req);

        GRPCResponseFuture {
            inner,
            otel_cx,
            finalization: Some(RequestFinalization {
                request_data,
                layer_state,
                response_extractor,
            }),
        }
    }
}

fn finalize_request<ResBody, E, ResExt>(
    result: &result::Result<http::Response<ResBody>, E>,
    request_data: RequestData,
    layer_state: &Arc<GRPCLayerState>,
    response_extractor: &ResExt,
) where
    E: fmt::Debug,
    ResExt: GRPCResponseAttributeExtractor<ResBody>,
{
    let cx = OtelContext::current();
    let span = cx.span();

    match result {
        Ok(response) => {
            let rpc_status_code = rpc_status_code(response);
            let custom_response_attributes = response_extractor.extract_attributes(response);
            let failed = is_error_status(rpc_status_code);

            let mut label_superset = Vec::with_capacity(
                3 + usize::from(failed)
                    + request_data.custom_request_attributes.len()
                    + custom_response_attributes.len(),
            );
            label_superset.push(request_data.system_kv.clone());
            label_superset.push(request_data.method_kv.clone());
            label_superset.push(KeyValue::new(
                semconv::attribute::RPC_RESPONSE_STATUS_CODE,
                rpc_status_code,
            ));
            if failed {
                label_superset.push(KeyValue::new(
                    semconv::attribute::ERROR_TYPE,
                    rpc_status_code,
                ));
            }
            label_superset.extend(request_data.custom_request_attributes);
            label_superset.extend(custom_response_attributes.iter().cloned());

            span.set_attribute(KeyValue::new(
                semconv::attribute::RPC_RESPONSE_STATUS_CODE,
                rpc_status_code,
            ));
            for attr in custom_response_attributes {
                span.set_attribute(attr);
            }
            if failed {
                span.set_attribute(KeyValue::new(
                    semconv::attribute::ERROR_TYPE,
                    rpc_status_code,
                ));
                span.set_status(Status::Error {
                    description: format!("gRPC status {rpc_status_code}").into(),
                });
            }

            layer_state.server_duration.record(
                request_data.duration_start.elapsed().as_secs_f64(),
                &label_superset,
            );
        }
        Err(error) => {
            span.set_status(Status::Error {
                description: format!("{error:?}").into(),
            });

            let label_superset = [
                request_data.system_kv.clone(),
                request_data.method_kv.clone(),
                KeyValue::new(semconv::attribute::ERROR_TYPE, "_OTHER"),
            ];
            layer_state.server_duration.record(
                request_data.duration_start.elapsed().as_secs_f64(),
                &label_superset,
            );
        }
    }
}

fn rpc_status_code<B>(response: &http::Response<B>) -> &'static str {
    response
        .headers()
        .get("grpc-status")
        .and_then(|value| value.to_str().ok())
        .map(grpc_status_name)
        .unwrap_or_else(|| {
            if response.status().is_success() {
                "OK"
            } else {
                "UNKNOWN"
            }
        })
}

fn grpc_status_name(code: &str) -> &'static str {
    match code {
        "0" => "OK",
        "1" => "CANCELLED",
        "2" => "UNKNOWN",
        "3" => "INVALID_ARGUMENT",
        "4" => "DEADLINE_EXCEEDED",
        "5" => "NOT_FOUND",
        "6" => "ALREADY_EXISTS",
        "7" => "PERMISSION_DENIED",
        "8" => "RESOURCE_EXHAUSTED",
        "9" => "FAILED_PRECONDITION",
        "10" => "ABORTED",
        "11" => "OUT_OF_RANGE",
        "12" => "UNIMPLEMENTED",
        "13" => "INTERNAL",
        "14" => "UNAVAILABLE",
        "15" => "DATA_LOSS",
        "16" => "UNAUTHENTICATED",
        _ => "UNKNOWN",
    }
}

fn is_error_status(status: &str) -> bool {
    matches!(
        status,
        "UNKNOWN"
            | "DEADLINE_EXCEEDED"
            | "UNIMPLEMENTED"
            | "INTERNAL"
            | "UNAVAILABLE"
            | "DATA_LOSS"
    )
}

fn parse_grpc_path(path: &str) -> Option<&str> {
    let path = path.strip_prefix('/')?;
    let (service, method) = path.rsplit_once('/')?;
    if service.is_empty() || method.is_empty() {
        return None;
    }
    Some(path)
}

fn instrumentation_scope() -> InstrumentationScope {
    InstrumentationScope::builder(INSTRUMENTATION_NAME)
        .with_version(env!("CARGO_PKG_VERSION"))
        .with_schema_url(semconv::SCHEMA_URL)
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    use http::{Request, Response, StatusCode};
    use opentelemetry_sdk::metrics::{
        data::{AggregatedMetrics, MetricData},
        InMemoryMetricExporter, PeriodicReader, SdkMeterProvider,
    };
    use opentelemetry_sdk::trace::{InMemorySpanExporterBuilder, SdkTracerProvider};
    use std::time::Duration;
    use tower::Service;

    #[test]
    fn parses_grpc_path() {
        assert_eq!(
            parse_grpc_path("/opentelemetry.proto.collector.trace.v1.TraceService/Export"),
            Some("opentelemetry.proto.collector.trace.v1.TraceService/Export")
        );
        assert_eq!(parse_grpc_path("/Service/Method"), Some("Service/Method"));
        assert_eq!(parse_grpc_path("Service/Method"), None);
        assert_eq!(parse_grpc_path("/Service"), None);
        assert_eq!(parse_grpc_path("/Service/"), None);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_grpc_tracing_with_in_memory_tracer() {
        let trace_exporter = InMemorySpanExporterBuilder::new().build();
        let tracer_provider = SdkTracerProvider::builder()
            .with_simple_exporter(trace_exporter.clone())
            .build();

        let layer = GRPCLayerBuilder::builder()
            .with_tracer_provider(tracer_provider.clone())
            .build();
        let service = tower::service_fn(|_req: Request<String>| async {
            Ok::<_, std::convert::Infallible>(
                Response::builder()
                    .status(StatusCode::OK)
                    .header("grpc-status", "0")
                    .body(String::new())
                    .unwrap(),
            )
        });
        let mut service = layer.layer(service);

        let request = Request::builder()
            .method("POST")
            .uri("http://example.com/package.Service/GetThing")
            .body(String::new())
            .unwrap();

        let _response = service.call(request).await.unwrap();
        tracer_provider.force_flush().unwrap();

        let spans = trace_exporter.get_finished_spans().unwrap();
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].name, "package.Service/GetThing");
        assert_eq!(
            spans[0].attributes,
            vec![
                KeyValue::new(semconv::attribute::RPC_SYSTEM_NAME, "grpc"),
                KeyValue::new(semconv::attribute::RPC_METHOD, "package.Service/GetThing"),
                KeyValue::new(semconv::attribute::RPC_RESPONSE_STATUS_CODE, "OK"),
            ]
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_grpc_error_status_marks_span_error() {
        let trace_exporter = InMemorySpanExporterBuilder::new().build();
        let tracer_provider = SdkTracerProvider::builder()
            .with_simple_exporter(trace_exporter.clone())
            .build();

        let layer = GRPCLayerBuilder::builder()
            .with_tracer_provider(tracer_provider.clone())
            .build();
        let service = tower::service_fn(|_req: Request<String>| async {
            Ok::<_, std::convert::Infallible>(
                Response::builder()
                    .status(StatusCode::OK)
                    .header("grpc-status", "13")
                    .body(String::new())
                    .unwrap(),
            )
        });
        let mut service = layer.layer(service);

        let request = Request::builder()
            .method("POST")
            .uri("http://example.com/package.Service/GetThing")
            .body(String::new())
            .unwrap();

        let _response = service.call(request).await.unwrap();
        tracer_provider.force_flush().unwrap();

        let spans = trace_exporter.get_finished_spans().unwrap();
        assert_eq!(spans.len(), 1);
        assert!(matches!(spans[0].status, Status::Error { .. }));
        assert!(spans[0].attributes.contains(&KeyValue::new(
            semconv::attribute::RPC_RESPONSE_STATUS_CODE,
            "INTERNAL"
        )));
        assert!(spans[0]
            .attributes
            .contains(&KeyValue::new(semconv::attribute::ERROR_TYPE, "INTERNAL")));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_grpc_metrics_labels() {
        let exporter = InMemoryMetricExporter::default();
        let reader = PeriodicReader::builder(exporter.clone())
            .with_interval(Duration::from_millis(100))
            .build();
        let meter_provider = SdkMeterProvider::builder().with_reader(reader).build();
        let layer = GRPCLayerBuilder::builder()
            .with_meter_provider(meter_provider.clone())
            .build();

        let service = tower::service_fn(|_req: Request<String>| async {
            Ok::<_, std::convert::Infallible>(
                Response::builder()
                    .status(StatusCode::OK)
                    .header("grpc-status", "0")
                    .body(String::new())
                    .unwrap(),
            )
        });
        let mut service = layer.layer(service);

        let request = Request::builder()
            .method("POST")
            .uri("http://example.com/package.Service/GetThing")
            .body(String::new())
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
            .find(|m| m.name() == semconv::metric::RPC_SERVER_CALL_DURATION)
            .expect("Duration metric should exist");

        if let AggregatedMetrics::F64(MetricData::Histogram(histogram)) = duration_metric.data() {
            let data_point = histogram
                .data_points()
                .next()
                .expect("Should have data point");
            let attributes: Vec<_> = data_point.attributes().collect();
            assert_eq!(attributes.len(), 3);
            assert!(attributes
                .iter()
                .any(|kv| kv.key.as_str() == semconv::attribute::RPC_SYSTEM_NAME
                    && kv.value.as_str() == "grpc"));
            assert!(attributes
                .iter()
                .any(|kv| kv.key.as_str() == semconv::attribute::RPC_METHOD
                    && kv.value.as_str() == "package.Service/GetThing"));
            assert!(attributes.iter().any(|kv| kv.key.as_str()
                == semconv::attribute::RPC_RESPONSE_STATUS_CODE
                && kv.value.as_str() == "OK"));
        } else {
            panic!("Expected histogram data for duration metric");
        }
    }
}
