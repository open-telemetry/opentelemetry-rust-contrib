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

use opentelemetry::global::{self, BoxedTracer};
use opentelemetry::metrics::{Histogram, Meter, MeterProvider, NoopMeterProvider};
use opentelemetry::trace::noop::NoopTracerProvider;
use opentelemetry::trace::{SpanKind, Status, TraceContextExt, Tracer, TracerProvider};
use opentelemetry::Context as OtelContext;
use opentelemetry::KeyValue;
use opentelemetry_http::HeaderInjector;
use opentelemetry_semantic_conventions as semconv;
use opentelemetry_semantic_conventions::attribute::ERROR_TYPE;
use pin_project_lite::pin_project;
use tower_layer::Layer as TowerLayer;
use tower_service::Service as TowerService;

use crate::common::attributes::{
    error_type_kv, method_kv, redact_url_full, split_and_format_protocol_version, url_scheme_kv,
    DEFAULT_SENSITIVE_QUERY_PARAMETERS,
};
use crate::common::instrumentation_scope;
use crate::http::extractors::{
    NoOpExtractor, NoRouteExtractor, RequestAttributeExtractor, ResponseAttributeExtractor,
    RouteExtractor,
};
use crate::Result;

const HTTP_CLIENT_DURATION_UNIT: &str = "s";

const OTEL_DEFAULT_HTTP_CLIENT_DURATION_BOUNDS: [f64; 14] = [
    0.005, 0.01, 0.025, 0.05, 0.075, 0.1, 0.25, 0.5, 0.75, 1.0, 2.5, 5.0, 7.5, 10.0,
];

const HTTP_CLIENT_REQUEST_BODY_SIZE_UNIT: &str = "By";

const HTTP_CLIENT_RESPONSE_BODY_SIZE_UNIT: &str = "By";

/// State scoped to the entire middleware [`Layer`].
struct LayerState {
    client_request_duration: Histogram<f64>,
    client_request_body_size: Histogram<u64>,
    client_response_body_size: Histogram<u64>,
    sensitive_query_parameters: Box<[Cow<'static, str>]>,
}

#[derive(Clone)]
/// [`tower_service::Service`] produced by [`Layer`].
pub struct Service<S, RouteExt = NoRouteExtractor, ReqExt = NoOpExtractor, ResExt = NoOpExtractor> {
    state: Arc<LayerState>,
    route_extractor: RouteExt,
    request_extractor: ReqExt,
    response_extractor: ResExt,
    inner_service: S,
    tracer: Arc<BoxedTracer>,
}

#[derive(Clone)]
/// [`tower_layer::Layer`] which applies OpenTelemetry HTTP client metrics and tracing.
pub struct Layer<RouteExt = NoRouteExtractor, ReqExt = NoOpExtractor, ResExt = NoOpExtractor> {
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
pub struct LayerBuilder<RouteExt = NoRouteExtractor, ReqExt = NoOpExtractor, ResExt = NoOpExtractor>
{
    tracer: Option<Arc<BoxedTracer>>,
    meter: Option<Meter>,
    req_dur_bounds: Option<Vec<f64>>,
    tracing_enabled: bool,
    metrics_enabled: bool,
    sensitive_query_parameters: Option<Vec<Cow<'static, str>>>,
    route_extractor: RouteExt,
    request_extractor: ReqExt,
    response_extractor: ResExt,
}

impl LayerBuilder {
    pub fn builder() -> Self {
        LayerBuilder {
            tracer: None,
            meter: None,
            req_dur_bounds: Some(Vec::from(OTEL_DEFAULT_HTTP_CLIENT_DURATION_BOUNDS)),
            tracing_enabled: true,
            metrics_enabled: true,
            sensitive_query_parameters: None,
            route_extractor: NoRouteExtractor,
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
            tracer: self.tracer,
            meter: self.meter,
            req_dur_bounds: self.req_dur_bounds,
            tracing_enabled: self.tracing_enabled,
            metrics_enabled: self.metrics_enabled,
            sensitive_query_parameters: self.sensitive_query_parameters,
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
            tracer: self.tracer,
            meter: self.meter,
            req_dur_bounds: self.req_dur_bounds,
            tracing_enabled: self.tracing_enabled,
            metrics_enabled: self.metrics_enabled,
            sensitive_query_parameters: self.sensitive_query_parameters,
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
            req_dur_bounds: self.req_dur_bounds,
            tracing_enabled: self.tracing_enabled,
            metrics_enabled: self.metrics_enabled,
            sensitive_query_parameters: self.sensitive_query_parameters,
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

        let sensitive_query_parameters: Box<[Cow<'static, str>]> =
            match self.sensitive_query_parameters {
                Some(parameters) => parameters.into(),
                None => DEFAULT_SENSITIVE_QUERY_PARAMETERS
                    .iter()
                    .map(|parameter| Cow::Borrowed(*parameter))
                    .collect(),
            };

        Ok(Layer {
            state: Arc::from(make_state(
                meter,
                req_dur_bounds,
                sensitive_query_parameters,
            )),
            route_extractor: self.route_extractor,
            request_extractor: self.request_extractor,
            response_extractor: self.response_extractor,
            tracer,
        })
    }

    /// Set the query parameter keys whose values the layer redacts in
    /// `url.full`.
    ///
    /// The list replaces the default one, which holds the keys that the
    /// semantic conventions name. It does not extend it. An empty list disables
    /// redaction. Keys are matched case-sensitively. The user information of
    /// `url.full` is always redacted.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let layer = LayerBuilder::builder()
    ///     .with_sensitive_query_parameters(["sig", "token"])
    ///     .build()
    ///     .unwrap();
    /// ```
    pub fn with_sensitive_query_parameters<I, P>(mut self, parameters: I) -> Self
    where
        I: IntoIterator<Item = P>,
        P: AsRef<str>,
    {
        self.sensitive_query_parameters = Some(
            parameters
                .into_iter()
                .map(|parameter| Cow::Owned(parameter.as_ref().to_owned()))
                .collect(),
        );
        self
    }

    /// Enable or disable trace collection for this layer.
    ///
    /// Tracing is enabled by default. When disabled, the layer records no
    /// client spans, but context is still injected into outgoing request
    /// headers so downstream servers keep the trace intact.
    pub fn with_tracing(mut self, enabled: bool) -> Self {
        self.tracing_enabled = enabled;
        self
    }

    /// Enable or disable metrics collection for this layer.
    ///
    /// Metrics are enabled by default. When disabled, the layer records no
    /// HTTP client metrics.
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

fn make_state(
    meter: Meter,
    req_dur_bounds: Vec<f64>,
    sensitive_query_parameters: Box<[Cow<'static, str>]>,
) -> LayerState {
    LayerState {
        client_request_duration: meter
            .f64_histogram(Cow::from(semconv::metric::HTTP_CLIENT_REQUEST_DURATION))
            .with_description("Duration of HTTP client requests.")
            .with_unit(Cow::from(HTTP_CLIENT_DURATION_UNIT))
            .with_boundaries(req_dur_bounds)
            .build(),
        client_request_body_size: meter
            .u64_histogram(semconv::metric::HTTP_CLIENT_REQUEST_BODY_SIZE)
            .with_description("Size of HTTP client request bodies.")
            .with_unit(HTTP_CLIENT_REQUEST_BODY_SIZE_UNIT)
            .build(),
        client_response_body_size: meter
            .u64_histogram(semconv::metric::HTTP_CLIENT_RESPONSE_BODY_SIZE)
            .with_description("Size of HTTP client response bodies.")
            .with_unit(HTTP_CLIENT_RESPONSE_BODY_SIZE_UNIT)
            .build(),
        sensitive_query_parameters,
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
        let protocol_name_kv = KeyValue::new(semconv::attribute::NETWORK_PROTOCOL_NAME, protocol);
        let protocol_version_kv =
            KeyValue::new(semconv::attribute::NETWORK_PROTOCOL_VERSION, version);

        let url_scheme_kv = url_scheme_kv(req.uri());

        let method = req.method().as_str().to_owned();
        let method_kv = method_kv(req.method());

        let server_address_kv_opt = req
            .uri()
            .host()
            .map(|host| KeyValue::new(semconv::attribute::SERVER_ADDRESS, host.to_owned()));
        let server_port_kv_opt = req
            .uri()
            .port_u16()
            .map(|port| KeyValue::new(semconv::attribute::SERVER_PORT, i64::from(port)));

        let route = self.route_extractor.extract_route(&req);
        let route_kv_opt = route
            .as_ref()
            .map(|r| KeyValue::new(semconv::attribute::HTTP_ROUTE, r.clone()));

        let span_name = match &route {
            Some(r) => format!("{} {}", method, r),
            None => method.clone(),
        };

        let custom_request_attributes = self.request_extractor.extract_attributes(&req);

        let mut span_attributes = vec![
            KeyValue::new(semconv::attribute::HTTP_REQUEST_METHOD, method.clone()),
            KeyValue::new(
                semconv::attribute::URL_FULL,
                redact_url_full(req.uri(), &self.state.sensitive_query_parameters),
            ),
            url_scheme_kv.clone(),
        ];
        if let Some(server_address_kv) = &server_address_kv_opt {
            span_attributes.push(server_address_kv.clone());
        }
        if let Some(server_port_kv) = &server_port_kv_opt {
            span_attributes.push(server_port_kv.clone());
        }
        if let Some(r) = &route {
            span_attributes.push(KeyValue::new(semconv::attribute::HTTP_ROUTE, r.clone()));
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
        global::get_text_map_propagator(|propagator| {
            propagator.inject_context(&cx, &mut HeaderInjector(req.headers_mut()))
        });

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
        server_address_kv_opt,
        server_port_kv_opt,
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

            let custom_response_attributes = response_extractor.extract_attributes(response);

            span.set_attribute(status_code_kv.clone());
            for attr in &custom_response_attributes {
                span.set_attribute(attr.clone());
            }

            // From the semantic convention: For HTTP status codes in the 4xx range
            // span status MUST be left unset in case of SpanKind.SERVER and SHOULD
            // be set to Error in case of SpanKind.CLIENT.
            // For HTTP status codes in the 5xx range, as well as any other code the
            // client failed to interpret, span status SHOULD be set to Error.
            let error_type_kv = if http_status.is_client_error() || http_status.is_server_error() {
                let error_type_kv = error_type_kv(http_status);
                span.set_attribute(error_type_kv.clone());
                // From the semantic convention:
                // Don’t set the span status description if the reason can be
                // inferred from http.response.status_code.
                span.set_status(Status::error(""));
                Some(error_type_kv)
            } else {
                None
            };

            let cap = 5
                + error_type_kv.is_some() as usize
                + route_kv_opt.is_some() as usize
                + server_address_kv_opt.is_some() as usize
                + server_port_kv_opt.is_some() as usize
                + custom_request_attributes.len()
                + custom_response_attributes.len();
            let mut label_superset = Vec::with_capacity(cap);
            label_superset.push(protocol_name_kv);
            label_superset.push(protocol_version_kv);
            label_superset.push(url_scheme_kv);
            label_superset.push(method_kv);
            label_superset.push(status_code_kv);
            if let Some(route_kv) = route_kv_opt {
                label_superset.push(route_kv);
            }
            if let Some(server_address_kv) = server_address_kv_opt {
                label_superset.push(server_address_kv);
            }
            if let Some(server_port_kv) = server_port_kv_opt {
                label_superset.push(server_port_kv);
            }
            if let Some(error_type_kv) = error_type_kv {
                label_superset.push(error_type_kv);
            }
            label_superset.extend(custom_request_attributes);
            label_superset.extend(custom_response_attributes);

            layer_state
                .client_request_duration
                .record(duration_start.elapsed().as_secs_f64(), &label_superset);

            if let Some(req_content_length) = req_body_size {
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
            // From the semantic convention: If the request fails with an error
            // before response status code was sent or received, error.type
            // SHOULD be set to exception type (its fully-qualified class name,
            // if applicable) or a component-specific low cardinality error identifier.
            let error_type_kv = KeyValue::new(ERROR_TYPE, std::any::type_name::<E>());

            span.set_attribute(error_type_kv.clone());
            span.set_status(Status::error(format!("{:?}", error)));

            let mut label_superset = Vec::with_capacity(
                5 + route_kv_opt.is_some() as usize
                    + server_address_kv_opt.is_some() as usize
                    + server_port_kv_opt.is_some() as usize,
            );
            label_superset.push(protocol_name_kv);
            label_superset.push(protocol_version_kv);
            label_superset.push(url_scheme_kv);
            label_superset.push(method_kv);
            if let Some(route_kv) = route_kv_opt {
                label_superset.push(route_kv);
            }
            if let Some(server_address_kv) = server_address_kv_opt {
                label_superset.push(server_address_kv);
            }
            if let Some(server_port_kv) = server_port_kv_opt {
                label_superset.push(server_port_kv);
            }
            label_superset.push(error_type_kv);

            layer_state
                .client_request_duration
                .record(duration_start.elapsed().as_secs_f64(), &label_superset);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::http::extractors::PathExtractor;

    use http::{Request, Response, StatusCode};
    use opentelemetry::trace::{FutureExt, Span, SpanKind, TraceContextExt, Tracer};
    use opentelemetry_sdk::metrics::{
        data::{AggregatedMetrics, MetricData},
        InMemoryMetricExporter, PeriodicReader, SdkMeterProvider,
    };
    use opentelemetry_sdk::propagation::TraceContextPropagator;
    use opentelemetry_sdk::trace::{InMemorySpanExporterBuilder, SdkTracerProvider};
    use std::time::Duration;
    use tower::{ServiceBuilder, ServiceExt};

    #[derive(Debug, Clone, Copy)]
    struct TestError;

    fn ok_service() -> impl TowerService<
        Request<String>,
        Response = Response<String>,
        Error = std::convert::Infallible,
    > + Clone {
        tower::service_fn(|_req: Request<String>| async {
            Ok::<_, std::convert::Infallible>(
                Response::builder()
                    .status(StatusCode::OK)
                    .body(String::from("OK"))
                    .unwrap(),
            )
        })
    }

    fn outcome_service(
        outcome: std::result::Result<StatusCode, TestError>,
    ) -> impl TowerService<Request<String>, Response = Response<String>, Error = TestError> + Clone
    {
        tower::service_fn(move |_req: Request<String>| async move {
            outcome.map(|status| {
                Response::builder()
                    .status(status)
                    .body(String::from("body"))
                    .unwrap()
            })
        })
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_client_span_is_child_with_client_kind() {
        struct TestCase {
            name: &'static str,
            target: &'static str,
            outcome: std::result::Result<StatusCode, TestError>,
            expected_span_name: &'static str,
            expected_attributes: Vec<KeyValue>,
            expected_span_status: Status,
        }

        let test_cases = [
            TestCase {
                name: "plain URL",
                target: "http://example.com/api/users/123",
                outcome: Ok(StatusCode::OK),
                expected_span_name: "GET /api/users/123",
                expected_attributes: vec![
                    KeyValue::new(semconv::attribute::HTTP_REQUEST_METHOD, "GET".to_string()),
                    KeyValue::new(
                        semconv::attribute::URL_FULL,
                        "http://example.com/api/users/123".to_string(),
                    ),
                    KeyValue::new(semconv::attribute::URL_SCHEME, "http".to_string()),
                    KeyValue::new(
                        semconv::attribute::SERVER_ADDRESS,
                        "example.com".to_string(),
                    ),
                    KeyValue::new(semconv::attribute::HTTP_ROUTE, "/api/users/123".to_string()),
                    KeyValue::new(semconv::attribute::HTTP_RESPONSE_STATUS_CODE, 200),
                ],
                expected_span_status: Status::Unset,
            },
            TestCase {
                name: "URL with a query component",
                target: "http://example.com/api/users/123?fields=name&verbose=true",
                outcome: Ok(StatusCode::OK),
                expected_span_name: "GET /api/users/123",
                expected_attributes: vec![
                    KeyValue::new(semconv::attribute::HTTP_REQUEST_METHOD, "GET".to_string()),
                    KeyValue::new(
                        semconv::attribute::URL_FULL,
                        "http://example.com/api/users/123?fields=name&verbose=true".to_string(),
                    ),
                    KeyValue::new(semconv::attribute::URL_SCHEME, "http".to_string()),
                    KeyValue::new(
                        semconv::attribute::SERVER_ADDRESS,
                        "example.com".to_string(),
                    ),
                    KeyValue::new(semconv::attribute::HTTP_ROUTE, "/api/users/123".to_string()),
                    KeyValue::new(semconv::attribute::HTTP_RESPONSE_STATUS_CODE, 200),
                ],
                expected_span_status: Status::Unset,
            },
            TestCase {
                name: "URL with a sensitive query parameter",
                target: "https://example.com:8443/api/users/123?fields=name&sig=secret",
                outcome: Ok(StatusCode::OK),
                expected_span_name: "GET /api/users/123",
                expected_attributes: vec![
                    KeyValue::new(semconv::attribute::HTTP_REQUEST_METHOD, "GET".to_string()),
                    KeyValue::new(
                        semconv::attribute::URL_FULL,
                        "https://example.com:8443/api/users/123?fields=name&sig=REDACTED"
                            .to_string(),
                    ),
                    KeyValue::new(semconv::attribute::URL_SCHEME, "https".to_string()),
                    KeyValue::new(
                        semconv::attribute::SERVER_ADDRESS,
                        "example.com".to_string(),
                    ),
                    KeyValue::new(semconv::attribute::SERVER_PORT, 8443),
                    KeyValue::new(semconv::attribute::HTTP_ROUTE, "/api/users/123".to_string()),
                    KeyValue::new(semconv::attribute::HTTP_RESPONSE_STATUS_CODE, 200),
                ],
                expected_span_status: Status::Unset,
            },
            TestCase {
                name: "URL with user information",
                target: "http://user:password@example.com/api/users/123",
                outcome: Ok(StatusCode::OK),
                expected_span_name: "GET /api/users/123",
                expected_attributes: vec![
                    KeyValue::new(semconv::attribute::HTTP_REQUEST_METHOD, "GET".to_string()),
                    KeyValue::new(
                        semconv::attribute::URL_FULL,
                        "http://REDACTED:REDACTED@example.com/api/users/123".to_string(),
                    ),
                    KeyValue::new(semconv::attribute::URL_SCHEME, "http".to_string()),
                    KeyValue::new(
                        semconv::attribute::SERVER_ADDRESS,
                        "example.com".to_string(),
                    ),
                    KeyValue::new(semconv::attribute::HTTP_ROUTE, "/api/users/123".to_string()),
                    KeyValue::new(semconv::attribute::HTTP_RESPONSE_STATUS_CODE, 200),
                ],
                expected_span_status: Status::Unset,
            },
            TestCase {
                name: "URL with an empty query component",
                target: "http://example.com/api/users/123?",
                outcome: Ok(StatusCode::OK),
                expected_span_name: "GET /api/users/123",
                expected_attributes: vec![
                    KeyValue::new(semconv::attribute::HTTP_REQUEST_METHOD, "GET".to_string()),
                    KeyValue::new(
                        semconv::attribute::URL_FULL,
                        "http://example.com/api/users/123?".to_string(),
                    ),
                    KeyValue::new(semconv::attribute::URL_SCHEME, "http".to_string()),
                    KeyValue::new(
                        semconv::attribute::SERVER_ADDRESS,
                        "example.com".to_string(),
                    ),
                    KeyValue::new(semconv::attribute::HTTP_ROUTE, "/api/users/123".to_string()),
                    KeyValue::new(semconv::attribute::HTTP_RESPONSE_STATUS_CODE, 200),
                ],
                expected_span_status: Status::Unset,
            },
            TestCase {
                name: "a client error response carries error.type",
                target: "http://example.com/api/users/123",
                outcome: Ok(StatusCode::NOT_FOUND),
                expected_span_name: "GET /api/users/123",
                expected_attributes: vec![
                    KeyValue::new(semconv::attribute::HTTP_REQUEST_METHOD, "GET".to_string()),
                    KeyValue::new(
                        semconv::attribute::URL_FULL,
                        "http://example.com/api/users/123".to_string(),
                    ),
                    KeyValue::new(semconv::attribute::URL_SCHEME, "http".to_string()),
                    KeyValue::new(
                        semconv::attribute::SERVER_ADDRESS,
                        "example.com".to_string(),
                    ),
                    KeyValue::new(semconv::attribute::HTTP_ROUTE, "/api/users/123".to_string()),
                    KeyValue::new(semconv::attribute::HTTP_RESPONSE_STATUS_CODE, 404),
                    KeyValue::new(ERROR_TYPE, "404"),
                ],
                expected_span_status: Status::error(""),
            },
            TestCase {
                name: "a server error response carries error.type",
                target: "http://example.com/api/users/123",
                outcome: Ok(StatusCode::INTERNAL_SERVER_ERROR),
                expected_span_name: "GET /api/users/123",
                expected_attributes: vec![
                    KeyValue::new(semconv::attribute::HTTP_REQUEST_METHOD, "GET".to_string()),
                    KeyValue::new(
                        semconv::attribute::URL_FULL,
                        "http://example.com/api/users/123".to_string(),
                    ),
                    KeyValue::new(semconv::attribute::URL_SCHEME, "http".to_string()),
                    KeyValue::new(
                        semconv::attribute::SERVER_ADDRESS,
                        "example.com".to_string(),
                    ),
                    KeyValue::new(semconv::attribute::HTTP_ROUTE, "/api/users/123".to_string()),
                    KeyValue::new(semconv::attribute::HTTP_RESPONSE_STATUS_CODE, 500),
                    KeyValue::new(ERROR_TYPE, "500"),
                ],
                expected_span_status: Status::error(""),
            },
            TestCase {
                name: "an uncommon error response carries error.type",
                target: "http://example.com/api/users/123",
                outcome: Ok(StatusCode::from_u16(499).unwrap()),
                expected_span_name: "GET /api/users/123",
                expected_attributes: vec![
                    KeyValue::new(semconv::attribute::HTTP_REQUEST_METHOD, "GET".to_string()),
                    KeyValue::new(
                        semconv::attribute::URL_FULL,
                        "http://example.com/api/users/123".to_string(),
                    ),
                    KeyValue::new(semconv::attribute::URL_SCHEME, "http".to_string()),
                    KeyValue::new(
                        semconv::attribute::SERVER_ADDRESS,
                        "example.com".to_string(),
                    ),
                    KeyValue::new(semconv::attribute::HTTP_ROUTE, "/api/users/123".to_string()),
                    KeyValue::new(semconv::attribute::HTTP_RESPONSE_STATUS_CODE, 499),
                    KeyValue::new(ERROR_TYPE, "499"),
                ],
                expected_span_status: Status::error(""),
            },
            TestCase {
                name: "a redirect response carries no error.type",
                target: "http://example.com/api/users/123",
                outcome: Ok(StatusCode::MOVED_PERMANENTLY),
                expected_span_name: "GET /api/users/123",
                expected_attributes: vec![
                    KeyValue::new(semconv::attribute::HTTP_REQUEST_METHOD, "GET".to_string()),
                    KeyValue::new(
                        semconv::attribute::URL_FULL,
                        "http://example.com/api/users/123".to_string(),
                    ),
                    KeyValue::new(semconv::attribute::URL_SCHEME, "http".to_string()),
                    KeyValue::new(
                        semconv::attribute::SERVER_ADDRESS,
                        "example.com".to_string(),
                    ),
                    KeyValue::new(semconv::attribute::HTTP_ROUTE, "/api/users/123".to_string()),
                    KeyValue::new(semconv::attribute::HTTP_RESPONSE_STATUS_CODE, 301),
                ],
                expected_span_status: Status::Unset,
            },
            TestCase {
                name: "an inner-service error carries the error type name",
                target: "http://example.com/api/users/123",
                outcome: Err(TestError),
                expected_span_name: "GET /api/users/123",
                expected_attributes: vec![
                    KeyValue::new(semconv::attribute::HTTP_REQUEST_METHOD, "GET".to_string()),
                    KeyValue::new(
                        semconv::attribute::URL_FULL,
                        "http://example.com/api/users/123".to_string(),
                    ),
                    KeyValue::new(semconv::attribute::URL_SCHEME, "http".to_string()),
                    KeyValue::new(
                        semconv::attribute::SERVER_ADDRESS,
                        "example.com".to_string(),
                    ),
                    KeyValue::new(semconv::attribute::HTTP_ROUTE, "/api/users/123".to_string()),
                    KeyValue::new(ERROR_TYPE, std::any::type_name::<TestError>()),
                ],
                expected_span_status: Status::error(format!("{:?}", TestError)),
            },
        ];

        for test_case in test_cases {
            let trace_exporter = InMemorySpanExporterBuilder::new().build();
            let tracer_provider = SdkTracerProvider::builder()
                .with_simple_exporter(trace_exporter.clone())
                .build();
            let tracer = tracer_provider.tracer("test_tracer");

            let layer = LayerBuilder::builder()
                .with_route_extractor(PathExtractor)
                .with_tracer_provider(tracer_provider.clone())
                .build()
                .unwrap();

            let mut service = ServiceBuilder::new()
                .layer(layer)
                .service(outcome_service(test_case.outcome));

            let parent_span = tracer.start("parent_operation");
            let cx = OtelContext::current_with_span(parent_span);

            let request = Request::builder()
                .method("GET")
                .uri(test_case.target)
                .body("test".to_string())
                .unwrap();

            let _response = async { service.ready().await.unwrap().call(request).await }
                .with_context(cx)
                .await;

            tracer_provider.force_flush().unwrap();

            let spans = trace_exporter.get_finished_spans().unwrap();
            assert_eq!(
                spans.len(),
                2,
                "{}: expected parent + client span",
                test_case.name
            );

            let client_span = spans
                .iter()
                .find(|span| span.name == test_case.expected_span_name)
                .expect("Should find client span");
            let parent_span = spans
                .iter()
                .find(|span| span.name == "parent_operation")
                .expect("Should find parent span");

            assert_eq!(
                (
                    client_span.name.as_ref(),
                    client_span.span_kind.clone(),
                    client_span.parent_span_id,
                    client_span.span_context.trace_id(),
                    client_span.attributes.clone(),
                    client_span.status.clone()
                ),
                (
                    test_case.expected_span_name,
                    SpanKind::Client,
                    parent_span.span_context.span_id(),
                    parent_span.span_context.trace_id(),
                    test_case.expected_attributes,
                    test_case.expected_span_status
                ),
                "{}",
                test_case.name
            );
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn with_sensitive_query_parameters_replaces_the_default_list() {
        struct TestCase {
            name: &'static str,
            sensitive: Option<Vec<&'static str>>,
            expected_url_full: &'static str,
        }

        let test_cases = [
            TestCase {
                name: "the default list redacts the keys of the conventions",
                sensitive: None,
                expected_url_full: "http://example.com/api?token=abc&sig=REDACTED",
            },
            TestCase {
                name: "a custom list replaces the default one",
                sensitive: Some(vec!["token"]),
                expected_url_full: "http://example.com/api?token=REDACTED&sig=abc",
            },
            TestCase {
                name: "an empty list disables redaction",
                sensitive: Some(Vec::new()),
                expected_url_full: "http://example.com/api?token=abc&sig=abc",
            },
        ];

        for test_case in test_cases {
            let trace_exporter = InMemorySpanExporterBuilder::new().build();
            let tracer_provider = SdkTracerProvider::builder()
                .with_simple_exporter(trace_exporter.clone())
                .build();
            let builder = LayerBuilder::builder().with_tracer_provider(tracer_provider.clone());
            let builder = match test_case.sensitive {
                Some(sensitive) => builder.with_sensitive_query_parameters(sensitive),
                None => builder,
            };
            let mut service = builder.build().unwrap().layer(ok_service());

            let request = Request::builder()
                .method("GET")
                .uri("http://example.com/api?token=abc&sig=abc")
                .body(String::from("body"))
                .unwrap();
            let _response = service.call(request).await.unwrap();

            tracer_provider.force_flush().unwrap();
            let spans = trace_exporter.get_finished_spans().unwrap();
            assert_eq!(spans.len(), 1, "{}", test_case.name);

            let result = spans[0]
                .attributes
                .iter()
                .find(|kv| kv.key.as_str() == semconv::attribute::URL_FULL)
                .expect("url.full should be present")
                .clone();

            assert_eq!(
                result,
                KeyValue::new(
                    semconv::attribute::URL_FULL,
                    test_case.expected_url_full.to_string()
                ),
                "{}",
                test_case.name
            );
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_client_injects_context_into_headers() {
        struct TestCase {
            name: &'static str,
            tracing_enabled: bool,
            expect_parent_span_id: bool,
        }

        let test_cases = [
            TestCase {
                name: "tracing enabled injects the client span",
                tracing_enabled: true,
                expect_parent_span_id: false,
            },
            TestCase {
                name: "tracing disabled still injects the parent span",
                tracing_enabled: false,
                expect_parent_span_id: true,
            },
        ];

        global::set_text_map_propagator(TraceContextPropagator::new());

        for test_case in test_cases {
            let tracer_provider = SdkTracerProvider::builder().build();
            let tracer = tracer_provider.tracer("test_tracer");

            let layer = LayerBuilder::builder()
                .with_tracer_provider(tracer_provider.clone())
                .with_tracing(test_case.tracing_enabled)
                .build()
                .unwrap();

            let injected = Arc::new(std::sync::Mutex::new(None::<String>));
            let injected_clone = injected.clone();
            let mut service = ServiceBuilder::new()
                .layer(layer)
                .service(tower::service_fn(move |req: Request<String>| {
                    let injected = injected_clone.clone();
                    async move {
                        *injected.lock().unwrap() = req
                            .headers()
                            .get("traceparent")
                            .and_then(|value| value.to_str().ok())
                            .map(str::to_owned);
                        Ok::<_, std::convert::Infallible>(
                            Response::builder()
                                .status(StatusCode::OK)
                                .body(String::from("OK"))
                                .unwrap(),
                        )
                    }
                }));

            let parent_span = tracer.start("parent_operation");
            let parent_span_context = parent_span.span_context().clone();
            let cx = OtelContext::current_with_span(parent_span);

            let request = Request::builder()
                .method("GET")
                .uri("http://example.com/api")
                .body("test".to_string())
                .unwrap();

            let _response = async { service.ready().await.unwrap().call(request).await.unwrap() }
                .with_context(cx)
                .await;

            let traceparent = injected
                .lock()
                .unwrap()
                .clone()
                .expect("outgoing request should carry an injected traceparent header");
            let parts: Vec<&str> = traceparent.split('-').collect();

            assert_eq!(
                (
                    parts[1],
                    parts[2] == parent_span_context.span_id().to_string()
                ),
                (
                    parent_span_context.trace_id().to_string().as_str(),
                    test_case.expect_parent_span_id
                ),
                "{}",
                test_case.name
            );
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_client_metrics() {
        struct TestCase {
            name: &'static str,
            outcome: std::result::Result<StatusCode, TestError>,
            expected_attributes: Vec<KeyValue>,
        }

        let test_cases = [
            TestCase {
                name: "a successful response",
                outcome: Ok(StatusCode::OK),
                expected_attributes: vec![
                    KeyValue::new(semconv::attribute::HTTP_REQUEST_METHOD, "GET"),
                    KeyValue::new(semconv::attribute::HTTP_RESPONSE_STATUS_CODE, 200),
                    KeyValue::new(semconv::attribute::NETWORK_PROTOCOL_NAME, "http"),
                    KeyValue::new(semconv::attribute::NETWORK_PROTOCOL_VERSION, "1.1"),
                    KeyValue::new(semconv::attribute::SERVER_ADDRESS, "example.com"),
                    KeyValue::new(semconv::attribute::URL_SCHEME, "https"),
                ],
            },
            TestCase {
                name: "a client error response carries error.type",
                outcome: Ok(StatusCode::NOT_FOUND),
                expected_attributes: vec![
                    KeyValue::new(ERROR_TYPE, "404"),
                    KeyValue::new(semconv::attribute::HTTP_REQUEST_METHOD, "GET"),
                    KeyValue::new(semconv::attribute::HTTP_RESPONSE_STATUS_CODE, 404),
                    KeyValue::new(semconv::attribute::NETWORK_PROTOCOL_NAME, "http"),
                    KeyValue::new(semconv::attribute::NETWORK_PROTOCOL_VERSION, "1.1"),
                    KeyValue::new(semconv::attribute::SERVER_ADDRESS, "example.com"),
                    KeyValue::new(semconv::attribute::URL_SCHEME, "https"),
                ],
            },
            TestCase {
                name: "an inner-service error carries the error type name",
                outcome: Err(TestError),
                expected_attributes: vec![
                    KeyValue::new(ERROR_TYPE, std::any::type_name::<TestError>()),
                    KeyValue::new(semconv::attribute::HTTP_REQUEST_METHOD, "GET"),
                    KeyValue::new(semconv::attribute::NETWORK_PROTOCOL_NAME, "http"),
                    KeyValue::new(semconv::attribute::NETWORK_PROTOCOL_VERSION, "1.1"),
                    KeyValue::new(semconv::attribute::SERVER_ADDRESS, "example.com"),
                    KeyValue::new(semconv::attribute::URL_SCHEME, "https"),
                ],
            },
        ];

        for test_case in test_cases {
            let exporter = InMemoryMetricExporter::default();
            let reader = PeriodicReader::builder(exporter.clone())
                .with_interval(Duration::from_millis(100))
                .build();
            let meter_provider = SdkMeterProvider::builder().with_reader(reader).build();

            let layer = LayerBuilder::builder()
                .with_meter_provider(meter_provider.clone())
                .build()
                .unwrap();

            let mut service = layer.layer(outcome_service(test_case.outcome));

            let request = Request::builder()
                .method("GET")
                .uri("https://example.com/test")
                .body("test body".to_string())
                .unwrap();

            let _response = service.call(request).await;

            meter_provider.force_flush().unwrap();

            let metrics = exporter.get_finished_metrics().unwrap();
            let duration_metric = metrics
                .iter()
                .flat_map(|resource_metrics| resource_metrics.scope_metrics())
                .flat_map(|scope_metrics| scope_metrics.metrics())
                .find(|metric| metric.name() == semconv::metric::HTTP_CLIENT_REQUEST_DURATION)
                .expect("Client duration metric should exist");

            let AggregatedMetrics::F64(MetricData::Histogram(histogram)) = duration_metric.data()
            else {
                panic!("Expected histogram data for client duration metric");
            };
            let mut result: Vec<KeyValue> = histogram
                .data_points()
                .next()
                .expect("Should have data point")
                .attributes()
                .cloned()
                .collect();
            result.sort_by(|a, b| a.key.as_str().cmp(b.key.as_str()));

            assert_eq!(result, test_case.expected_attributes, "{}", test_case.name);
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_with_tracing_disabled_records_no_client_span() {
        let trace_exporter = InMemorySpanExporterBuilder::new().build();
        let tracer_provider = SdkTracerProvider::builder()
            .with_simple_exporter(trace_exporter.clone())
            .build();
        let tracer = tracer_provider.tracer("test_tracer");

        let layer = LayerBuilder::builder()
            .with_tracer_provider(tracer_provider.clone())
            .with_tracing(false)
            .build()
            .unwrap();

        let mut service = ServiceBuilder::new().layer(layer).service(ok_service());

        // A surrounding span keeps context/propagation intact even with the
        // layer's own tracing disabled.
        let parent_span = tracer.start("parent_operation");
        let cx = OtelContext::current_with_span(parent_span);

        let request = Request::builder()
            .method("GET")
            .uri("http://example.com/api")
            .body("test".to_string())
            .unwrap();

        let _response = async { service.ready().await.unwrap().call(request).await.unwrap() }
            .with_context(cx)
            .await;

        tracer_provider.force_flush().unwrap();

        let spans = trace_exporter.get_finished_spans().unwrap();
        let span_names: Vec<&str> = spans.iter().map(|span| span.name.as_ref()).collect();
        assert_eq!(span_names, vec!["parent_operation"]);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_default_no_route_extractor_low_cardinality_span_names() {
        let trace_exporter = InMemorySpanExporterBuilder::new().build();
        let tracer_provider = SdkTracerProvider::builder()
            .with_simple_exporter(trace_exporter.clone())
            .build();

        // Default builder uses NoRouteExtractor, which returns None for every
        // request. Span names should be method-only (e.g. "GET") regardless of
        // the request path, and no http.route attribute should be set.
        let layer = LayerBuilder::builder()
            .with_tracer_provider(tracer_provider.clone())
            .build()
            .unwrap();

        let mut service = ServiceBuilder::new().layer(layer).service(ok_service());

        let request = Request::builder()
            .method("GET")
            .uri("http://example.com/api/users/123")
            .body("test".to_string())
            .unwrap();
        service.ready().await.unwrap().call(request).await.unwrap();

        tracer_provider.force_flush().unwrap();

        let spans = trace_exporter.get_finished_spans().unwrap();
        assert_eq!(spans.len(), 1);

        let span = &spans[0];
        assert_eq!(
            span.name, "GET",
            "span name should be method-only, got: {}",
            span.name
        );
        assert!(
            span.attributes
                .iter()
                .all(|kv| kv.key.as_str() != semconv::attribute::HTTP_ROUTE),
            "no http.route attribute should be set when using the default NoRouteExtractor"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_with_metrics_disabled_records_no_metrics() {
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

        let mut service = layer.layer(ok_service());

        let request = Request::builder()
            .method("GET")
            .uri("https://example.com/test")
            .body("test body".to_string())
            .unwrap();

        let _response = service.call(request).await.unwrap();

        meter_provider.force_flush().unwrap();

        let metrics = exporter.get_finished_metrics().unwrap();
        assert!(metrics.is_empty());
    }
}
