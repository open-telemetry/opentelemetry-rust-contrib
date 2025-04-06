use std::{borrow::Cow, rc::Rc, task::Poll};

use actix_web::{
    dev::{Service, ServiceRequest, ServiceResponse, Transform},
    http::header::HeaderMap,
    Error,
};
use futures_util::future::{ok, FutureExt as _, LocalBoxFuture, Ready};
use opentelemetry::{
    global::{self},
    propagation::Extractor,
    trace::{FutureExt as OtelFutureExt, SpanKind, Status, TraceContextExt, Tracer},
    KeyValue,
};
use opentelemetry_semantic_conventions::trace::HTTP_RESPONSE_STATUS_CODE;

use super::{get_scope, route_formatter::RouteFormatter};
use crate::util::trace_attributes_from_request;

/// Request tracing middleware.
///
/// # Examples:
///
/// ```no_run
/// use actix_web::{web, App, HttpServer};
/// use opentelemetry::global;
/// use opentelemetry_instrumentation_actix_web::RequestTracing;
/// use opentelemetry_sdk::trace::SdkTracerProvider;
///
/// async fn index() -> &'static str {
///     "Hello world!"
/// }
///
/// #[actix_web::main]
/// async fn main() -> std::io::Result<()> {
///     // Install an OpenTelemetry trace pipeline.
///     // Swap for https://docs.rs/opentelemetry-jaeger or other compatible
///     // exporter to send trace information to your collector.
///     let exporter = opentelemetry_stdout::SpanExporter::default();
///
///     // Configure your tracer provider with your exporter(s)
///     let provider = SdkTracerProvider::builder()
///         .with_simple_exporter(exporter)
///         .build();
///     global::set_tracer_provider(provider);
///
///     HttpServer::new(|| {
///         App::new()
///             .wrap(RequestTracing::new())
///             .service(web::resource("/").to(index))
///     })
///     .bind("127.0.0.1:8080")?
///     .run()
///     .await
/// }
///```
#[derive(Default, Debug)]
pub struct RequestTracing {
    route_formatter: Option<Rc<dyn RouteFormatter + 'static>>,
}

impl RequestTracing {
    /// Actix web middleware to trace each request in an OpenTelemetry span.
    pub fn new() -> RequestTracing {
        RequestTracing::default()
    }

    /// Actix web middleware to trace each request in an OpenTelemetry span with
    /// formatted routes.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use actix_web::{web, App, HttpServer};
    /// use opentelemetry_instrumentation_actix_web::{RouteFormatter, RequestTracing};
    ///
    /// # #[actix_web::main]
    /// # async fn main() -> std::io::Result<()> {
    ///
    ///
    /// #[derive(Debug)]
    /// struct MyLowercaseFormatter;
    ///
    /// impl RouteFormatter for MyLowercaseFormatter {
    ///     fn format(&self, path: &str) -> String {
    ///         path.to_lowercase()
    ///     }
    /// }
    ///
    /// // report /users/{id} as /users/:id
    /// HttpServer::new(move || {
    ///     App::new()
    ///         .wrap(RequestTracing::with_formatter(MyLowercaseFormatter))
    ///         .service(web::resource("/users/{id}").to(|| async { "ok" }))
    /// })
    /// .bind("127.0.0.1:8080")?
    /// .run()
    /// .await
    /// # }
    /// ```
    pub fn with_formatter<T: RouteFormatter + 'static>(route_formatter: T) -> Self {
        RequestTracing {
            route_formatter: Some(Rc::new(route_formatter)),
        }
    }
}

impl<S, B> Transform<S, ServiceRequest> for RequestTracing
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Transform = RequestTracingMiddleware<S>;
    type InitError = ();
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ok(RequestTracingMiddleware::new(
            global::tracer_with_scope(get_scope()),
            service,
            self.route_formatter.clone(),
        ))
    }
}

/// Request tracing middleware
#[derive(Debug)]
pub struct RequestTracingMiddleware<S> {
    tracer: global::BoxedTracer,
    service: S,
    route_formatter: Option<Rc<dyn RouteFormatter>>,
}

impl<S, B> RequestTracingMiddleware<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    B: 'static,
{
    fn new(
        tracer: global::BoxedTracer,
        service: S,
        route_formatter: Option<Rc<dyn RouteFormatter>>,
    ) -> Self {
        RequestTracingMiddleware {
            tracer,
            service,
            route_formatter,
        }
    }
}

impl<S, B> Service<ServiceRequest> for RequestTracingMiddleware<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&self, cx: &mut std::task::Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.service.poll_ready(cx)
    }

    fn call(&self, mut req: ServiceRequest) -> Self::Future {
        let parent_context = global::get_text_map_propagator(|propagator| {
            propagator.extract(&RequestHeaderCarrier::new(req.headers_mut()))
        });
        let mut http_route: Cow<'static, str> = req
            .match_pattern()
            .map(Into::into)
            .unwrap_or_else(|| "default".into());
        if let Some(formatter) = &self.route_formatter {
            http_route = formatter.format(&http_route).into();
        }

        let mut builder = self.tracer.span_builder(http_route.clone());
        builder.span_kind = Some(SpanKind::Server);
        builder.attributes = Some(trace_attributes_from_request(&req, &http_route));

        let span = self.tracer.build_with_context(builder, &parent_context);
        let cx = parent_context.with_span(span);

        #[cfg(feature = "sync-middleware")]
        let attachment = cx.clone().attach();

        let fut = self
            .service
            .call(req)
            .with_context(cx.clone())
            .map(move |res| match res {
                Ok(ok_res) => {
                    let span = cx.span();
                    span.set_attribute(KeyValue::new(
                        HTTP_RESPONSE_STATUS_CODE,
                        ok_res.status().as_u16() as i64,
                    ));
                    if ok_res.status().is_server_error() {
                        span.set_status(Status::error(
                            ok_res
                                .status()
                                .canonical_reason()
                                .map(ToString::to_string)
                                .unwrap_or_default(),
                        ));
                    };
                    span.end();
                    Ok(ok_res)
                }
                Err(err) => {
                    let span = cx.span();
                    span.set_status(Status::error(format!("{:?}", err)));
                    span.end();
                    Err(err)
                }
            });

        #[cfg(feature = "sync-middleware")]
        drop(attachment);

        Box::pin(fut)
    }
}

struct RequestHeaderCarrier<'a> {
    headers: &'a HeaderMap,
}

impl<'a> RequestHeaderCarrier<'a> {
    fn new(headers: &'a HeaderMap) -> Self {
        RequestHeaderCarrier { headers }
    }
}

impl Extractor for RequestHeaderCarrier<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        self.headers.get(key).and_then(|v| v.to_str().ok())
    }

    fn keys(&self) -> Vec<&str> {
        self.headers.keys().map(|header| header.as_str()).collect()
    }
}
