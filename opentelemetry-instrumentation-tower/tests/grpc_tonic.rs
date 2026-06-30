use std::net::SocketAddr;
use std::time::Duration;

use opentelemetry::trace::Status as SpanStatus;
use opentelemetry::KeyValue;
use opentelemetry_instrumentation_tower::grpc::GRPCLayerBuilder;
use opentelemetry_proto::tonic::collector::trace::v1::{
    trace_service_client::TraceServiceClient,
    trace_service_server::{TraceService, TraceServiceServer},
    ExportTraceServiceRequest, ExportTraceServiceResponse,
};
use opentelemetry_sdk::metrics::{
    data::{AggregatedMetrics, MetricData},
    InMemoryMetricExporter, PeriodicReader, SdkMeterProvider,
};
use opentelemetry_sdk::trace::{InMemorySpanExporterBuilder, SdkTracerProvider};
use opentelemetry_semantic_conventions as semconv;
use tokio::sync::oneshot;
use tokio_stream::wrappers::TcpListenerStream;
use tonic::{Request, Response, Status};

const OTLP_TRACE_METHOD: &str = "opentelemetry.proto.collector.trace.v1.TraceService/Export";

#[derive(Clone, Copy)]
enum ServerBehavior {
    Ok,
    InternalError,
}

#[derive(Clone, Copy)]
struct MockTraceService {
    behavior: ServerBehavior,
}

#[tonic::async_trait]
impl TraceService for MockTraceService {
    async fn export(
        &self,
        _request: Request<ExportTraceServiceRequest>,
    ) -> Result<Response<ExportTraceServiceResponse>, Status> {
        match self.behavior {
            ServerBehavior::Ok => Ok(Response::new(ExportTraceServiceResponse {
                partial_success: None,
            })),
            ServerBehavior::InternalError => Err(Status::internal("boom")),
        }
    }
}

async fn spawn_trace_server(
    behavior: ServerBehavior,
    tracer_provider: SdkTracerProvider,
    meter_provider: SdkMeterProvider,
) -> (SocketAddr, oneshot::Sender<()>, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let incoming = TcpListenerStream::new(listener);
    let (shutdown_tx, shutdown_rx) = oneshot::channel();

    let layer = GRPCLayerBuilder::builder()
        .with_tracer_provider(tracer_provider)
        .with_meter_provider(meter_provider)
        .build();
    let service = TraceServiceServer::new(MockTraceService { behavior });

    let handle = tokio::spawn(async move {
        tonic::transport::Server::builder()
            .layer(layer)
            .add_service(service)
            .serve_with_incoming_shutdown(incoming, async {
                let _ = shutdown_rx.await;
            })
            .await
            .unwrap();
    });

    (addr, shutdown_tx, handle)
}

#[tokio::test(flavor = "multi_thread")]
async fn records_real_tonic_unary_success() {
    let trace_exporter = InMemorySpanExporterBuilder::new().build();
    let tracer_provider = SdkTracerProvider::builder()
        .with_simple_exporter(trace_exporter.clone())
        .build();
    let metric_exporter = InMemoryMetricExporter::default();
    let reader = PeriodicReader::builder(metric_exporter.clone())
        .with_interval(Duration::from_millis(100))
        .build();
    let meter_provider = SdkMeterProvider::builder().with_reader(reader).build();

    let (addr, shutdown_tx, server_handle) = spawn_trace_server(
        ServerBehavior::Ok,
        tracer_provider.clone(),
        meter_provider.clone(),
    )
    .await;

    let mut client = TraceServiceClient::connect(format!("http://{addr}"))
        .await
        .unwrap();
    client
        .export(ExportTraceServiceRequest {
            resource_spans: vec![],
        })
        .await
        .unwrap();

    tracer_provider.force_flush().unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;

    let spans = trace_exporter.get_finished_spans().unwrap();
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].name, OTLP_TRACE_METHOD);
    assert_eq!(spans[0].status, SpanStatus::Unset);
    assert_eq!(
        spans[0].attributes,
        vec![
            KeyValue::new(semconv::attribute::RPC_SYSTEM_NAME, "grpc"),
            KeyValue::new(semconv::attribute::RPC_METHOD, OTLP_TRACE_METHOD),
            KeyValue::new(semconv::attribute::RPC_RESPONSE_STATUS_CODE, "OK"),
        ]
    );

    let metrics = metric_exporter.get_finished_metrics().unwrap();
    let duration_metric = metrics
        .iter()
        .flat_map(|resource_metrics| resource_metrics.scope_metrics())
        .flat_map(|scope_metrics| scope_metrics.metrics())
        .find(|metric| metric.name() == semconv::metric::RPC_SERVER_CALL_DURATION)
        .expect("duration metric should exist");

    if let AggregatedMetrics::F64(MetricData::Histogram(histogram)) = duration_metric.data() {
        let data_point = histogram
            .data_points()
            .next()
            .expect("duration metric should have a data point");
        let attributes: Vec<_> = data_point.attributes().collect();
        assert!(attributes
            .iter()
            .any(|kv| kv.key.as_str() == semconv::attribute::RPC_SYSTEM_NAME
                && kv.value.as_str() == "grpc"));
        assert!(attributes
            .iter()
            .any(|kv| kv.key.as_str() == semconv::attribute::RPC_METHOD
                && kv.value.as_str() == OTLP_TRACE_METHOD));
        assert!(attributes.iter().any(|kv| kv.key.as_str()
            == semconv::attribute::RPC_RESPONSE_STATUS_CODE
            && kv.value.as_str() == "OK"));
    } else {
        panic!("expected histogram data for duration metric");
    }

    shutdown_tx.send(()).unwrap();
    server_handle.await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn records_real_tonic_unary_error_status() {
    let trace_exporter = InMemorySpanExporterBuilder::new().build();
    let tracer_provider = SdkTracerProvider::builder()
        .with_simple_exporter(trace_exporter.clone())
        .build();
    let metric_exporter = InMemoryMetricExporter::default();
    let reader = PeriodicReader::builder(metric_exporter)
        .with_interval(Duration::from_millis(100))
        .build();
    let meter_provider = SdkMeterProvider::builder().with_reader(reader).build();

    let (addr, shutdown_tx, server_handle) = spawn_trace_server(
        ServerBehavior::InternalError,
        tracer_provider.clone(),
        meter_provider,
    )
    .await;

    let mut client = TraceServiceClient::connect(format!("http://{addr}"))
        .await
        .unwrap();
    let error = client
        .export(ExportTraceServiceRequest {
            resource_spans: vec![],
        })
        .await
        .unwrap_err();
    assert_eq!(error.code(), tonic::Code::Internal);

    tracer_provider.force_flush().unwrap();

    let spans = trace_exporter.get_finished_spans().unwrap();
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].name, OTLP_TRACE_METHOD);
    assert!(spans[0]
        .attributes
        .contains(&KeyValue::new(semconv::attribute::RPC_SYSTEM_NAME, "grpc")));
    assert!(spans[0].attributes.contains(&KeyValue::new(
        semconv::attribute::RPC_METHOD,
        OTLP_TRACE_METHOD
    )));
    assert!(spans[0].attributes.contains(&KeyValue::new(
        semconv::attribute::RPC_RESPONSE_STATUS_CODE,
        "INTERNAL"
    )));
    assert!(matches!(spans[0].status, SpanStatus::Error { .. }));
    assert!(spans[0]
        .attributes
        .contains(&KeyValue::new(semconv::attribute::ERROR_TYPE, "INTERNAL")));

    shutdown_tx.send(()).unwrap();
    server_handle.await.unwrap();
}
