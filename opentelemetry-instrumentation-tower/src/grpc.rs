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
pub struct GRPCService<S, ReqExt = NoOpGRPCExtractor, ResExt = NoOpGRPCExtractor> {
    state: Arc<GRPCLayerState>,
    request_extractor: ReqExt,
    response_extractor: ResExt,
    inner_service: S,
    tracer: Arc<BoxedTracer>,
}

#[derive(Clone)]
/// [`Layer`] which applies OpenTelemetry gRPC server metrics and tracing middleware.
pub struct GRPCLayer<ReqExt = NoOpGRPCExtractor, ResExt = NoOpGRPCExtractor> {
    state: Arc<GRPCLayerState>,
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
pub struct GRPCLayerBuilder<ReqExt = NoOpGRPCExtractor, ResExt = NoOpGRPCExtractor> {
    tracer: Option<Arc<BoxedTracer>>,
    meter: Option<Meter>,
    duration_bounds: Option<Vec<f64>>,
    request_extractor: ReqExt,
    response_extractor: ResExt,
}

impl GRPCLayerBuilder {
    pub fn builder() -> Self {
        Self {
            tracer: None,
            meter: None,
            duration_bounds: Some(Vec::from(OTEL_DEFAULT_RPC_SERVER_DURATION_BOUNDS)),
            request_extractor: NoOpGRPCExtractor,
            response_extractor: NoOpGRPCExtractor,
        }
    }
}

impl<ReqExt, ResExt> GRPCLayerBuilder<ReqExt, ResExt> {
    /// Set a request attribute extractor.
    pub fn with_request_extractor<NewReqExt, B>(
        self,
        extractor: NewReqExt,
    ) -> GRPCLayerBuilder<NewReqExt, ResExt>
    where
        NewReqExt: GRPCRequestAttributeExtractor<B>,
    {
        GRPCLayerBuilder {
            tracer: self.tracer,
            meter: self.meter,
            duration_bounds: self.duration_bounds,
            request_extractor: extractor,
            response_extractor: self.response_extractor,
        }
    }

    /// Convenience method to set a function-based request attribute extractor.
    pub fn with_request_extractor_fn<F, B>(
        self,
        f: F,
    ) -> GRPCLayerBuilder<FnGRPCRequestExtractor<F>, ResExt>
    where
        F: Fn(&http::Request<B>) -> Vec<KeyValue> + Clone + Send + Sync + 'static,
    {
        self.with_request_extractor(FnGRPCRequestExtractor::new(f))
    }

    /// Set a response attribute extractor.
    pub fn with_response_extractor<NewResExt, B>(
        self,
        extractor: NewResExt,
    ) -> GRPCLayerBuilder<ReqExt, NewResExt>
    where
        NewResExt: GRPCResponseAttributeExtractor<B>,
    {
        GRPCLayerBuilder {
            tracer: self.tracer,
            meter: self.meter,
            duration_bounds: self.duration_bounds,
            request_extractor: self.request_extractor,
            response_extractor: extractor,
        }
    }

    /// Convenience method to set a function-based response attribute extractor.
    pub fn with_response_extractor_fn<F, B>(
        self,
        f: F,
    ) -> GRPCLayerBuilder<ReqExt, FnGRPCResponseExtractor<F>>
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

    pub fn build(self) -> GRPCLayer<ReqExt, ResExt> {
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

impl<S, ReqExt, ResExt> Layer<S> for GRPCLayer<ReqExt, ResExt>
where
    ReqExt: Clone,
    ResExt: Clone,
{
    type Service = GRPCService<S, ReqExt, ResExt>;

    fn layer(&self, service: S) -> Self::Service {
        GRPCService {
            state: self.state.clone(),
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

struct BodyFinalization {
    request_data: RequestData,
    layer_state: Arc<GRPCLayerState>,
    custom_response_attributes: Vec<KeyValue>,
    http_status: http::StatusCode,
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
    ResBody: http_body::Body,
    ResBody::Error: fmt::Debug,
    ResExt: GRPCResponseAttributeExtractor<ResBody>,
{
    type Output = result::Result<http::Response<GRPCResponseBody<ResBody>>, E>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        let _guard = this.otel_cx.clone().attach();
        let result = std::task::ready!(this.inner.poll(cx));
        match result {
            Ok(response) => {
                let fin = this
                    .finalization
                    .take()
                    .expect("gRPC response future polled after completion");
                let custom_response_attributes =
                    fin.response_extractor.extract_attributes(&response);
                let http_status = response.status();
                let rpc_status_code = rpc_status_code(response.headers(), http_status);
                let (parts, body) = response.into_parts();
                let body = GRPCResponseBody {
                    inner: body,
                    otel_cx: this.otel_cx.clone(),
                    finalization: Some(BodyFinalization {
                        request_data: fin.request_data,
                        layer_state: fin.layer_state,
                        custom_response_attributes,
                        http_status,
                    }),
                    rpc_status_code,
                };
                Poll::Ready(Ok(http::Response::from_parts(parts, body)))
            }
            Err(error) => {
                if let Some(fin) = this.finalization.take() {
                    finalize_error(&error, fin.request_data, &fin.layer_state);
                }
                Poll::Ready(Err(error))
            }
        }
    }
}

pin_project! {
    /// Response body wrapper that observes gRPC status from trailers.
    pub struct GRPCResponseBody<B> {
        #[pin]
        inner: B,
        otel_cx: OtelContext,
        finalization: Option<BodyFinalization>,
        rpc_status_code: &'static str,
    }

    impl<B> PinnedDrop for GRPCResponseBody<B> {
        fn drop(this: Pin<&mut Self>) {
            let this = this.project();
            if let Some(fin) = this.finalization.take() {
                let _guard = this.otel_cx.clone().attach();
                finalize_response(this.rpc_status_code, fin);
            }
        }
    }
}

impl<B> http_body::Body for GRPCResponseBody<B>
where
    B: http_body::Body,
    B::Error: fmt::Debug,
{
    type Data = B::Data;
    type Error = B::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<result::Result<http_body::Frame<Self::Data>, Self::Error>>> {
        let mut this = self.project();

        match std::task::ready!(this.inner.as_mut().poll_frame(cx)) {
            Some(Ok(frame)) => {
                if let Some(trailers) = frame.trailers_ref() {
                    if let Some(fin) = this.finalization.take() {
                        let rpc_status_code = rpc_status_code(trailers, fin.http_status);
                        *this.rpc_status_code = rpc_status_code;
                        let _guard = this.otel_cx.clone().attach();
                        finalize_response(rpc_status_code, fin);
                    }
                }
                Poll::Ready(Some(Ok(frame)))
            }
            Some(Err(error)) => {
                if let Some(fin) = this.finalization.take() {
                    let _guard = this.otel_cx.clone().attach();
                    finalize_body_error(&error, fin);
                }
                Poll::Ready(Some(Err(error)))
            }
            None => {
                if let Some(fin) = this.finalization.take() {
                    let _guard = this.otel_cx.clone().attach();
                    finalize_response(this.rpc_status_code, fin);
                }
                Poll::Ready(None)
            }
        }
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> http_body::SizeHint {
        self.inner.size_hint()
    }
}

impl<S, ReqBody, ResBody, ReqExt, ResExt> Service<http::Request<ReqBody>>
    for GRPCService<S, ReqExt, ResExt>
where
    S: Service<http::Request<ReqBody>, Response = http::Response<ResBody>>,
    S::Future: Send,
    S::Error: fmt::Debug,
    ReqExt: GRPCRequestAttributeExtractor<ReqBody>,
    ResExt: GRPCResponseAttributeExtractor<ResBody>,
    ResBody: http_body::Body,
    ResBody::Error: fmt::Debug,
{
    type Response = http::Response<GRPCResponseBody<ResBody>>;
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

        let rpc_method = parse_grpc_path(req.uri().path())
            .unwrap_or_else(|| req.uri().path().trim_start_matches('/'))
            .to_owned();

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

fn finalize_response(rpc_status_code: &'static str, fin: BodyFinalization) {
    let cx = OtelContext::current();
    let span = cx.span();
    let failed = is_error_status(rpc_status_code);

    let mut label_superset = Vec::with_capacity(
        3 + usize::from(failed)
            + fin.request_data.custom_request_attributes.len()
            + fin.custom_response_attributes.len(),
    );
    label_superset.push(fin.request_data.system_kv.clone());
    label_superset.push(fin.request_data.method_kv.clone());
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
    label_superset.extend(fin.request_data.custom_request_attributes);
    label_superset.extend(fin.custom_response_attributes.iter().cloned());

    span.set_attribute(KeyValue::new(
        semconv::attribute::RPC_RESPONSE_STATUS_CODE,
        rpc_status_code,
    ));
    for attr in fin.custom_response_attributes {
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

    fin.layer_state.server_duration.record(
        fin.request_data.duration_start.elapsed().as_secs_f64(),
        &label_superset,
    );
}

fn finalize_error(
    error: &impl fmt::Debug,
    request_data: RequestData,
    layer_state: &Arc<GRPCLayerState>,
) {
    let cx = OtelContext::current();
    let span = cx.span();
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

fn finalize_body_error(error: &impl fmt::Debug, fin: BodyFinalization) {
    finalize_error(&error, fin.request_data, &fin.layer_state);
}

fn rpc_status_code(headers: &http::HeaderMap, http_status: http::StatusCode) -> &'static str {
    headers
        .get("grpc-status")
        .and_then(|value| value.to_str().ok())
        .map(grpc_status_name)
        .unwrap_or_else(|| {
            if http_status.is_success() {
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

    use std::convert::Infallible;

    use http::{Request, Response, StatusCode, Version};
    use http_body_util::{BodyExt, Empty};
    use opentelemetry_sdk::metrics::{
        data::{AggregatedMetrics, MetricData},
        InMemoryMetricExporter, PeriodicReader, SdkMeterProvider,
    };
    use opentelemetry_sdk::trace::{InMemorySpanExporterBuilder, SdkTracerProvider};
    use std::time::Duration;
    use tower::Service;

    struct TrailerBody {
        trailers: Option<http::HeaderMap>,
    }

    impl TrailerBody {
        fn new(trailers: http::HeaderMap) -> Self {
            Self {
                trailers: Some(trailers),
            }
        }
    }

    impl http_body::Body for TrailerBody {
        type Data = &'static [u8];
        type Error = Infallible;

        fn poll_frame(
            mut self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<Option<result::Result<http_body::Frame<Self::Data>, Self::Error>>> {
            Poll::Ready(
                self.trailers
                    .take()
                    .map(|trailers| Ok(http_body::Frame::trailers(trailers))),
            )
        }
    }

    fn grpc_request() -> Request<Empty<&'static [u8]>> {
        Request::builder()
            .method("POST")
            .uri("http://example.com/package.Service/GetThing")
            .version(Version::HTTP_2)
            .header("content-type", "application/grpc")
            .header("te", "trailers")
            .body(Empty::new())
            .unwrap()
    }

    fn grpc_response<B>(grpc_status: &'static str, body: B) -> Response<B> {
        Response::builder()
            .status(StatusCode::OK)
            .version(Version::HTTP_2)
            .header("content-type", "application/grpc")
            .header("grpc-status", grpc_status)
            .body(body)
            .unwrap()
    }

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
        let service = tower::service_fn(|_req: Request<Empty<&'static [u8]>>| async {
            Ok::<_, std::convert::Infallible>(grpc_response("0", Empty::<&'static [u8]>::new()))
        });
        let mut service = layer.layer(service);

        let response = service.call(grpc_request()).await.unwrap();
        response.into_body().collect().await.unwrap();
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
        let service = tower::service_fn(|_req: Request<Empty<&'static [u8]>>| async {
            Ok::<_, std::convert::Infallible>(grpc_response("13", Empty::<&'static [u8]>::new()))
        });
        let mut service = layer.layer(service);

        let response = service.call(grpc_request()).await.unwrap();
        response.into_body().collect().await.unwrap();
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
    async fn test_grpc_trailer_status_overrides_header_status() {
        let trace_exporter = InMemorySpanExporterBuilder::new().build();
        let tracer_provider = SdkTracerProvider::builder()
            .with_simple_exporter(trace_exporter.clone())
            .build();

        let layer = GRPCLayerBuilder::builder()
            .with_tracer_provider(tracer_provider.clone())
            .build();
        let service = tower::service_fn(|_req: Request<Empty<&'static [u8]>>| async {
            let mut trailers = http::HeaderMap::new();
            trailers.insert("grpc-status", http::HeaderValue::from_static("13"));

            Ok::<_, std::convert::Infallible>(grpc_response("0", TrailerBody::new(trailers)))
        });
        let mut service = layer.layer(service);

        let response = service.call(grpc_request()).await.unwrap();
        response.into_body().collect().await.unwrap();
        tracer_provider.force_flush().unwrap();

        let spans = trace_exporter.get_finished_spans().unwrap();
        assert_eq!(spans.len(), 1);
        assert!(matches!(spans[0].status, Status::Error { .. }));
        assert!(spans[0].attributes.contains(&KeyValue::new(
            semconv::attribute::RPC_RESPONSE_STATUS_CODE,
            "INTERNAL"
        )));
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

        let service = tower::service_fn(|_req: Request<Empty<&'static [u8]>>| async {
            Ok::<_, std::convert::Infallible>(grpc_response("0", Empty::<&'static [u8]>::new()))
        });
        let mut service = layer.layer(service);

        let response = service.call(grpc_request()).await.unwrap();
        response.into_body().collect().await.unwrap();
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
