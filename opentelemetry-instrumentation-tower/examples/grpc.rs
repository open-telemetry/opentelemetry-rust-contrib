use std::convert::Infallible;

use http::{Request, Response, StatusCode};
use opentelemetry_instrumentation_tower::grpc::GRPCLayerBuilder;
use tower::{Service, ServiceBuilder, ServiceExt};

async fn grpc_handler(_req: Request<String>) -> Result<Response<String>, Infallible> {
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("grpc-status", "0")
        .body(String::new())
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
        .body(String::new())
        .unwrap();

    let _response = service.ready().await.unwrap().call(request).await.unwrap();
}
