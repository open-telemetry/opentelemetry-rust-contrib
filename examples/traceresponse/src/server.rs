use http_body_util::{combinators::BoxBody, BodyExt, Full};
use hyper::{body::Incoming, service::service_fn, Request, Response};
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder;
use opentelemetry::{
    global,
    propagation::TextMapPropagator,
    trace::{SpanKind, TraceContextExt, Tracer},
    Context,
};
use opentelemetry_contrib::trace::propagator::trace_context_response::TraceContextResponsePropagator;
use opentelemetry_http::{Bytes, HeaderExtractor, HeaderInjector};
use opentelemetry_sdk::{propagation::TraceContextPropagator, trace::SdkTracerProvider};
use opentelemetry_stdout::SpanExporter;
use std::{convert::Infallible, net::SocketAddr};
use tokio::net::TcpListener;

async fn handle(
    req: Request<Incoming>,
) -> Result<Response<BoxBody<Bytes, hyper::Error>>, Infallible> {
    let parent_cx = global::get_text_map_propagator(|propagator| {
        propagator.extract(&HeaderExtractor(req.headers()))
    });
    let _cx_guard = parent_cx.attach();

    let tracer = global::tracer("example/server");
    let span = tracer
        .span_builder("say hello")
        .with_kind(SpanKind::Server)
        .start(&tracer);

    let cx = Context::current_with_span(span);

    cx.span().add_event("handling this...", Vec::new());

    let mut res = Response::new(
        Full::new(Bytes::from_static(b"Server is up and running!"))
            .map_err(|err| match err {})
            .boxed(),
    );
    let response_propagator: &dyn TextMapPropagator = &TraceContextResponsePropagator::new();
    response_propagator.inject_context(&cx, &mut HeaderInjector(res.headers_mut()));

    Ok(res)
}

fn init_traces() -> SdkTracerProvider {
    global::set_text_map_propagator(TraceContextPropagator::new());

    // Install stdout exporter pipeline to be able to retrieve the collected spans.
    // For the demonstration, use `Sampler::AlwaysOn` sampler to sample all traces. In a production
    // application, use `Sampler::ParentBased` or `Sampler::TraceIdRatioBased` with a desired ratio.
    SdkTracerProvider::builder()
        .with_simple_exporter(SpanExporter::default())
        .build()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
    let tracer_provider = init_traces();
    global::set_tracer_provider(tracer_provider.clone());

    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    let listener = TcpListener::bind(addr).await.unwrap();

    while let Ok((stream, _addr)) = listener.accept().await {
        if let Err(err) = Builder::new(TokioExecutor::new())
            .serve_connection(TokioIo::new(stream), service_fn(handle))
            .await
        {
            eprintln!("{err}");
        }
    }

    tracer_provider.shutdown()?;

    Ok(())
}
