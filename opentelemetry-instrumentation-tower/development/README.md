# Development

## Usage

### Docker Compose

#### Components
* HTTP server examples with demo code utilizing this metrics middleware
* K6 load generation script to create traffic to the example servers
* OTEL Collector to collect and forward metrics to receivers -
  by default the collector config will print metrics to its logs as well as forward to Mimir's OTEL endpoint.
* Mimir to ingest the metrics into Prometheus metrics format
  this runs in monolithic mode with no persistent volume to retain metrics after container remove.
- Grafana to query and the chart the metrics ingested by Mimir

#### Build and Run

In this directory, build and run one of the example servers along with the related components:
```shell
docker compose up --build -d example-axum-server k6-load-gen otel-collector mimir grafana
```

or

```shell
docker compose up --build -d example-hyper-server k6-load-gen otel-collector mimir grafana
```

The k6 load gen script will send traffic to the echo server's port 5000 over the docker network.
Port 5000 is also exposed for access from any http client on the local machine:

```shell
curl -i -X POST localhost:5000/

HTTP/1.1 200 OK
content-length: 12
date: Sun, 22 Jun 2025 22:59:34 GMT

hello world
```

## Examples
[Example code](../examples) is provided to demonstrate and test capabilities and configuration.

All example servers include some basic features to assist in verification on the metrics produced:
* randomized length of response body text in order to verify `http.server.response.body.size`
* randomized artificial response latency on some requests in order to verify `http.server.request.duration`.

### Axum Server
The [axum-http-service](../examples/axum-http-service) utilizes [Axum](https://github.com/tokio-rs/axum),
a popular, high-level async backend web framework compatible with the tower and tower-http ecosystem.

This example demonstrates the Axum-specific attribute extractors enabled by the `axum` feature flag,
as well as the support for custom request & response extractors to add metric attributes beyond the OTEL spec.
The example extractors utilize inherent aspects of HTTP requests & responses such as path length and query parameters,
as well as the `http` crate's [`Extensions`](https://docs.rs/http/latest/http/struct.Extensions.html),
which allow arbitrary key-value pairs to be attached to requests as they are passed down and back up the stack.
More can be found at ["What are Rust's HTTP extensions?"](https://blog.adamchalmers.com/what-are-extensions/).

### Hyper Server
The [hyper-http-service](../examples/hyper-http-service) utilizes [Hyper](https://github.com/hyperium/hyper),
a lower-level building-block HTTP library which forms the basis of Axum, Warp, and other popular Rust crates.

This example demonstrates the use of this middleware with Hyper and its Tower compatibility layer,
and is used during development to ensure that changes focused on Axum do not break or limit compatibility
with those lower-level interfaces.