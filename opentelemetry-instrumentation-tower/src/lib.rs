#![doc = include_str!("../README.md")]
//!
//! [`Layer`]: tower_layer::Layer
//! [`Service`]: tower_service::Service
//! [`Future`]: tower_service::Future

use std::borrow::Cow;
use std::future::Future;
use std::pin::Pin;
use std::string::String;
use std::sync::Arc;
use std::task::Poll::Ready;
use std::task::{Context, Poll};
use std::time::Instant;
use std::{fmt, result};

#[cfg(feature = "axum")]
use axum::extract::MatchedPath;
use futures_util::ready;
use opentelemetry::metrics::{Histogram, Meter, UpDownCounter};
use opentelemetry::KeyValue;
use pin_project_lite::pin_project;
use tower_layer::Layer;
use tower_service::Service;

const HTTP_SERVER_DURATION_METRIC: &str = "http.server.request.duration";
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
const HTTP_SERVER_ACTIVE_REQUESTS_METRIC: &str = "http.server.active_requests";
const HTTP_SERVER_ACTIVE_REQUESTS_UNIT: &str = "{request}";

const HTTP_SERVER_REQUEST_BODY_SIZE_METRIC: &str = "http.server.request.body.size";
const HTTP_SERVER_REQUEST_BODY_SIZE_UNIT: &str = "By";

const HTTP_SERVER_RESPONSE_BODY_SIZE_METRIC: &str = "http.server.response.body.size";
const HTTP_SERVER_RESPONSE_BODY_SIZE_UNIT: &str = "By";

const NETWORK_PROTOCOL_NAME_LABEL: &str = "network.protocol.name";
const NETWORK_PROTOCOL_VERSION_LABEL: &str = "network.protocol.version";
const URL_SCHEME_LABEL: &str = "url.scheme";

const HTTP_REQUEST_METHOD_LABEL: &str = "http.request.method";
#[allow(dead_code)] // cargo check is not smart
const HTTP_ROUTE_LABEL: &str = "http.route";
const HTTP_RESPONSE_STATUS_CODE_LABEL: &str = "http.response.status_code";

/// State scoped to the entire middleware Layer.
///
/// For now the only global state we hold onto is the metrics instruments.
/// The OTEL SDKs do support calling for the global meter provider instead of holding a reference
/// but it seems ideal to avoid extra access to the global meter, which sits behind a RWLock.
struct HTTPMetricsLayerState {
    pub server_request_duration: Histogram<f64>,
    pub server_active_requests: UpDownCounter<i64>,
    pub server_request_body_size: Histogram<u64>,
    pub server_response_body_size: Histogram<u64>,
}

#[derive(Clone)]
/// [`Service`] used by [`HTTPMetricsLayer`]
pub struct HTTPMetricsService<S> {
    pub(crate) state: Arc<HTTPMetricsLayerState>,
    inner_service: S,
}

#[derive(Clone)]
/// [`Layer`] which applies the OTEL HTTP server metrics middleware
pub struct HTTPMetricsLayer {
    state: Arc<HTTPMetricsLayerState>,
}

pub struct HTTPMetricsLayerBuilder {
    meter: Option<Meter>,
    req_dur_bounds: Option<Vec<f64>>,
}

/// Error typedef to implement `std::error::Error` for `tower_otel_http_metrics`
pub struct Error {
    #[allow(dead_code)]
    inner: ErrorKind,
}

/// `Result` typedef to use with the `tower_otel_http_metrics::Error` type
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
        f.debug_tuple("tower_otel_http_metrics::Error").finish()
    }
}

impl HTTPMetricsLayerBuilder {
    pub fn builder() -> Self {
        HTTPMetricsLayerBuilder {
            meter: None,
            req_dur_bounds: Some(LIBRARY_DEFAULT_HTTP_SERVER_DURATION_BOUNDARIES.to_vec()),
        }
    }

    pub fn build(self) -> Result<HTTPMetricsLayer> {
        let req_dur_bounds = self
            .req_dur_bounds
            .unwrap_or_else(|| LIBRARY_DEFAULT_HTTP_SERVER_DURATION_BOUNDARIES.to_vec());
        match self.meter {
            Some(meter) => Ok(HTTPMetricsLayer {
                state: Arc::from(HTTPMetricsLayerBuilder::make_state(meter, req_dur_bounds)),
            }),
            None => Err(Error {
                inner: ErrorKind::Config(String::from("no meter provided")),
            }),
        }
    }

    pub fn with_meter(mut self, meter: Meter) -> Self {
        self.meter = Some(meter);
        self
    }

    fn make_state(meter: Meter, req_dur_bounds: Vec<f64>) -> HTTPMetricsLayerState {
        HTTPMetricsLayerState {
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

impl<S> Layer<S> for HTTPMetricsLayer {
    type Service = HTTPMetricsService<S>;

    fn layer(&self, service: S) -> Self::Service {
        HTTPMetricsService {
            state: self.state.clone(),
            inner_service: service,
        }
    }
}

/// ResponseFutureMetricsState holds request-scoped data for metrics and their attributes.
///
/// ResponseFutureMetricsState lives inside the response future, as it needs to hold data
/// initialized or extracted from the request before it is forwarded to the inner Service.
/// The rest of the data (e.g. status code, error) can be extracted from the response
/// or calculated with respect to the data held here (e.g., duration = now - duration start).
#[derive(Clone)]
struct ResponseFutureMetricsState {
    // fields for the metric values
    // https://opentelemetry.io/docs/specs/semconv/http/http-metrics/#metric-httpserverrequestduration
    duration_start: Instant,
    // https://opentelemetry.io/docs/specs/semconv/http/http-metrics/#metric-httpserverrequestbodysize
    req_body_size: Option<u64>,

    // fields for metric labels
    protocol_name_kv: KeyValue,
    protocol_version_kv: KeyValue,
    url_scheme_kv: KeyValue,
    method_kv: KeyValue,
    route_kv_opt: Option<KeyValue>,
}

pin_project! {
    /// Response [`Future`] for [`HTTPMetricsService`].
    pub struct HTTPMetricsResponseFuture<F> {
        #[pin]
        inner_response_future: F,
        layer_state: Arc<HTTPMetricsLayerState>,
        metrics_state: ResponseFutureMetricsState,
    }
}

impl<S, ReqBody, ResBody: http_body::Body> Service<http::Request<ReqBody>> for HTTPMetricsService<S>
where
    S: Service<http::Request<ReqBody>, Response = http::Response<ResBody>>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = HTTPMetricsResponseFuture<S::Future>;

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
        let method_kv = KeyValue::new(HTTP_REQUEST_METHOD_LABEL, method);

        #[allow(unused_mut)]
        let mut route_kv_opt = None;
        #[cfg(feature = "axum")]
        if let Some(matched_path) = req.extensions().get::<MatchedPath>() {
            route_kv_opt = Some(KeyValue::new(
                HTTP_ROUTE_LABEL,
                matched_path.as_str().to_owned(),
            ));
        };

        self.state
            .server_active_requests
            .add(1, &[url_scheme_kv.clone(), method_kv.clone()]);

        HTTPMetricsResponseFuture {
            inner_response_future: self.inner_service.call(req),
            layer_state: self.state.clone(),
            metrics_state: ResponseFutureMetricsState {
                duration_start,
                req_body_size: content_length,

                protocol_name_kv,
                protocol_version_kv,
                url_scheme_kv,
                method_kv,
                route_kv_opt,
            },
        }
    }
}

impl<F, ResBody: http_body::Body, E> Future for HTTPMetricsResponseFuture<F>
where
    F: Future<Output = result::Result<http::Response<ResBody>, E>>,
{
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        let response = ready!(this.inner_response_future.poll(cx))?;
        let status = response.status();

        // all OTEL `http.server...` metrics use this entire label set,
        // except `http.server.active_requests`, which only uses a subset
        let mut label_superset = vec![
            this.metrics_state.protocol_name_kv.clone(),
            this.metrics_state.protocol_version_kv.clone(),
            this.metrics_state.url_scheme_kv.clone(),
            this.metrics_state.method_kv.clone(),
            KeyValue::new(HTTP_RESPONSE_STATUS_CODE_LABEL, i64::from(status.as_u16())),
        ];
        if let Some(route_kv) = this.metrics_state.route_kv_opt.clone() {
            label_superset.push(route_kv);
        }

        this.layer_state.server_request_duration.record(
            this.metrics_state.duration_start.elapsed().as_secs_f64(),
            &label_superset,
        );

        if let Some(req_content_length) = this.metrics_state.req_body_size {
            this.layer_state
                .server_request_body_size
                .record(req_content_length, &label_superset);
        }

        // use same approach for `http.server.response.body.size` as hyper does to set content-length
        if let Some(resp_content_length) = response.body().size_hint().exact() {
            this.layer_state
                .server_response_body_size
                .record(resp_content_length, &label_superset);
        }

        this.layer_state.server_active_requests.add(
            -1,
            &[
                this.metrics_state.url_scheme_kv.clone(),
                this.metrics_state.method_kv.clone(),
            ],
        );

        Ready(Ok(response))
    }
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
