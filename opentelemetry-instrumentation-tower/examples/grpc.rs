use std::convert::Infallible;

use http::{Request, Response, StatusCode, Version};
use http_body_util::Empty;
use opentelemetry_instrumentation_tower::grpc::GRPCLayerBuilder;
use tower::{Service, ServiceBuilder, ServiceExt};

async fn grpc_handler(
    _req: Request<Empty<&'static [u8]>>,
) -> Result<Response<Empty<&'static [u8]>>, Infallible> {
    Ok(Response::builder()
        .status(StatusCode::OK)
        .version(Version::HTTP_2)
        .header("content-type", "application/grpc")
        .header("grpc-status", "0")
        .body(Empty::new())
        .unwrap())
}

#[tokio::main]
async fn main() {
    let layer = GRPCLayerBuilder::builder().build();
    let mut service = ServiceBuilder::new()
        .layer(layer)
        .service(tower::service_fn(grpc_handler));

    let request = Request::builder()
        .method("POST")
        .uri("http://example.com/example.Greeter/SayHello")
        .version(Version::HTTP_2)
        .header("content-type", "application/grpc")
        .header("te", "trailers")
        .body(Empty::new())
        .unwrap();

    let _response = service.ready().await.unwrap().call(request).await.unwrap();
}
