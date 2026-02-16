use std::{
    fs::File,
    hint::black_box,
    net::{SocketAddr, UdpSocket},
    path::Path,
    sync::mpsc::{sync_channel, Receiver, RecvError, RecvTimeoutError, SyncSender},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use criterion::{
    criterion_group, criterion_main, measurement::WallTime, profiler::Profiler, BatchSize,
    BenchmarkGroup, BenchmarkId, Criterion,
};
use pprof::{flamegraph::Options, ProfilerGuard};

use opentelemetry::{
    trace::{SpanContext, SpanId, SpanKind, Status, TraceFlags, TraceId, TraceState},
    InstrumentationScope, KeyValue, Value,
};
use opentelemetry_aws::xray_exporter::{
    daemon_client::XrayDaemonClient, SegmentDocumentExporter, SegmentTranslator, XrayExporter,
};
use opentelemetry_sdk::{
    trace::{SpanData, SpanEvents, SpanExporter, SpanLinks},
    Resource,
};
use rand::{rngs::StdRng, seq::IndexedRandom, Rng, SeedableRng};

// OpenTelemetry Batch Processor default MAX_BATCH_SIZE is 512
// So most of the time we will never have bigger batch than that to process
const BATCH_SIZES: [usize; 3] = [10, 100, 512];

/// UDP receiver that counts received segments and discards payloads
struct UdpReceiverControler {
    command_sender: SyncSender<UdpReceiverThreadControlCommand>,
    response_receiver: Receiver<UdpReceiverThreadControlResponses>,
}

#[derive(Debug)]
enum UdpReceiverThreadControlCommand {
    Report,
    Reset,
    Stop,
    Start,
}

#[derive(Debug)]
enum UdpReceiverThreadControlResponses {
    Report {
        packets_received: u64,
        bytes_received: usize,
    },
    ResetDone,
    StartDone,
    StopDone,
}

impl UdpReceiverControler {
    fn new(addr: SocketAddr) -> std::io::Result<Self> {
        let (command_sender, command_receiver) = sync_channel(1);
        let (response_sender, response_receiver) = sync_channel(1);

        let socket = UdpSocket::bind(addr)?;
        socket.set_nonblocking(false)?;
        socket.set_read_timeout(Some(Duration::from_millis(10)))?;

        thread::spawn(move || {
            let mut buffer = vec![0u8; 65536]; // Max UDP packet size
            let mut packets_received_counter = 0;
            let mut bytes_received_counter = 0;

            // While not started, nothing other to do than wait for that command
            let started = loop {
                match command_receiver.recv() {
                    Ok(UdpReceiverThreadControlCommand::Start) => break true,
                    Ok(UdpReceiverThreadControlCommand::Stop) => break false,
                    Ok(cmd) => println!("Command {cmd:?} received but we have not started"),
                    Err(RecvError) => break false, // Channel closed
                }
            };

            // If started, listen the socket (else just end the thread)
            if started {
                response_sender
                    .send(UdpReceiverThreadControlResponses::StartDone)
                    .unwrap();
                let mut notify_stop = false;
                loop {
                    // Start the outer loop by verifying if we need to process a command.
                    match command_receiver.recv_timeout(Duration::from_nanos(0)) {
                        Ok(UdpReceiverThreadControlCommand::Report) => {
                            response_sender
                                .send(UdpReceiverThreadControlResponses::Report {
                                    packets_received: packets_received_counter,
                                    bytes_received: bytes_received_counter,
                                })
                                .unwrap();
                        }
                        Ok(UdpReceiverThreadControlCommand::Reset) => {
                            packets_received_counter = 0;
                            bytes_received_counter = 0;
                            response_sender
                                .send(UdpReceiverThreadControlResponses::ResetDone)
                                .unwrap();
                        }
                        Ok(UdpReceiverThreadControlCommand::Stop) => {
                            notify_stop = true;
                            break;
                        } // Exit the loop
                        Ok(UdpReceiverThreadControlCommand::Start) => println!(
                            "Command {:?} received but we have already started",
                            UdpReceiverThreadControlCommand::Start
                        ),
                        Err(RecvTimeoutError::Timeout) => (), // No command
                        Err(RecvTimeoutError::Disconnected) => break, // Channel closed, need to exit the loop
                    };

                    // We stay in this inner loop as long as the socket is continuously receiving data
                    // without falling in timeout (10ms).
                    // When any error (including timeout) arise, we try to featch a command before
                    // continuing to listen the socket.
                    while let Ok(size) = socket.recv(&mut buffer) {
                        packets_received_counter += 1;
                        bytes_received_counter += size;
                    }
                }
                if notify_stop {
                    response_sender
                        .send(UdpReceiverThreadControlResponses::StopDone)
                        .unwrap();
                }
            }
        });

        Ok(Self {
            command_sender,
            response_receiver,
        })
    }

    fn start(&self) {
        self.command_sender
            .send(UdpReceiverThreadControlCommand::Start)
            .unwrap();
        match self.response_receiver.recv().unwrap() {
            UdpReceiverThreadControlResponses::StartDone => (),
            _ => unreachable!(),
        }
    }

    fn stop(&self) {
        self.command_sender
            .send(UdpReceiverThreadControlCommand::Stop)
            .unwrap();
        match self.response_receiver.recv().unwrap() {
            UdpReceiverThreadControlResponses::StopDone => (),
            _ => unreachable!(),
        }
    }

    fn reset_counters(&self) {
        self.command_sender
            .send(UdpReceiverThreadControlCommand::Reset)
            .unwrap();
        match self.response_receiver.recv().unwrap() {
            UdpReceiverThreadControlResponses::ResetDone => (),
            _ => unreachable!(),
        }
    }

    fn report(&self) -> (u64, usize) {
        self.command_sender
            .send(UdpReceiverThreadControlCommand::Report)
            .unwrap();
        match self.response_receiver.recv().unwrap() {
            UdpReceiverThreadControlResponses::Report {
                packets_received,
                bytes_received,
            } => (packets_received, bytes_received),
            _ => unreachable!(),
        }
    }
}

/// Enum representing different parent span patterns (mapping to X-Ray Segments)
#[derive(Debug, Clone, Copy)]
enum ParentSpanType {
    Lambda,
    Ec2,
    EcsFargate,
    EcsEc2,
    Eks,
    Beanstalk,
}

impl ParentSpanType {
    fn random(rng: &mut StdRng) -> Self {
        let all = [
            Self::Lambda,
            Self::Ec2,
            Self::EcsFargate,
            Self::EcsEc2,
            Self::Eks,
            Self::Beanstalk,
        ];
        *all.choose(rng).unwrap()
    }
}

/// Enum representing different child span patterns (mapping to X-Ray Subsegments)
#[derive(Debug, Clone, Copy)]
enum ChildSpanType {
    AwsDynamoDb,
    AwsS3,
    AwsSqs,
    HttpExternal,
    SqlPostgres,
    SqlMysql,
    HttpWithError,
    GenericRemote,
}

impl ChildSpanType {
    fn random(rng: &mut StdRng) -> Self {
        let all = [
            Self::AwsDynamoDb,
            Self::AwsS3,
            Self::AwsSqs,
            Self::HttpExternal,
            Self::SqlPostgres,
            Self::SqlMysql,
            Self::HttpWithError,
            Self::GenericRemote,
        ];
        *all.choose(rng).unwrap()
    }
}

/// Timing information for nested spans
#[derive(Debug, Clone, Copy)]
struct SpanTiming {
    start_time: SystemTime,
    end_time: SystemTime,
}
impl SpanTiming {
    fn random(rng: &mut StdRng) -> Self {
        let base_timestamp = rng.random_range(1640995200000..1640995300000);

        let duration_ms = rng.random_range(500..5000);
        let start_time = UNIX_EPOCH + Duration::from_millis(base_timestamp);
        let end_time = start_time + Duration::from_millis(duration_ms);
        Self {
            start_time,
            end_time,
        }
    }
    fn random_sub(rng: &mut StdRng, parent: SpanTiming, count: usize) -> Vec<Self> {
        let mut timings = Vec::with_capacity(count);
        let Self {
            start_time,
            end_time,
        } = parent;

        let total_duration_ms = end_time.duration_since(start_time).unwrap().as_millis() as u64;
        let avg_child_duration_ms = total_duration_ms / count as u64;
        let dispertion_ms = avg_child_duration_ms / 8;

        let mut last_end_time = start_time;
        for _ in 0..(count - 1) {
            let start_time =
                last_end_time + Duration::from_millis(rng.random_range(0..dispertion_ms));
            let end_time = start_time
                + Duration::from_millis(rng.random_range(
                    (avg_child_duration_ms - dispertion_ms)
                        ..(avg_child_duration_ms + dispertion_ms),
                ));
            timings.push(SpanTiming {
                start_time,
                end_time,
            });
            last_end_time = end_time;
        }
        let start_time = last_end_time + Duration::from_millis(rng.random_range(0..dispertion_ms));
        let end_time = end_time - Duration::from_millis(rng.random_range(0..dispertion_ms));
        timings.push(SpanTiming {
            start_time,
            end_time,
        });
        timings
    }
}

/// Generates base attributes common to stress test spans
fn generate_random_attributes(rng: &mut StdRng) -> Vec<KeyValue> {
    let mut attributes = Vec::with_capacity(50);

    // Add many different types of attributes for density
    for i in 0..50 {
        match i % 6 {
            0 => attributes.push(KeyValue::new(
                format!("string_attr_{i}"),
                format!("value_{}", rng.random::<u32>()),
            )),
            1 => attributes.push(KeyValue::new(
                format!("int_attr_{i}"),
                Value::I64(rng.random::<i64>()),
            )),
            2 => attributes.push(KeyValue::new(
                format!("float_attr_{i}"),
                Value::F64(rng.random::<f64>()),
            )),
            3 => attributes.push(KeyValue::new(
                format!("bool_attr_{i}"),
                Value::Bool(rng.random::<bool>()),
            )),
            4 => {
                let arr: Vec<String> = (0..5)
                    .map(|_| format!("item_{}", rng.random::<u32>()))
                    .collect();
                let string_values: Vec<opentelemetry::StringValue> = arr
                    .into_iter()
                    .map(opentelemetry::StringValue::from)
                    .collect();
                attributes.push(KeyValue::new(
                    format!("str_array_attr_{i}"),
                    Value::Array(opentelemetry::Array::String(string_values)),
                ));
            }
            5 => {
                let arr: Vec<i64> = (0..5).map(|_| rng.random::<i64>()).collect();
                attributes.push(KeyValue::new(
                    format!("int_array_attr_{i}"),
                    Value::Array(opentelemetry::Array::I64(arr)),
                ));
            }
            _ => unreachable!(),
        }
    }

    attributes
}

fn add_common_attributes(
    rng: &mut StdRng,
    name: &str,
    timing: SpanTiming,
    attributes: &mut Vec<KeyValue>,
) {
    let duration_ns = timing
        .end_time
        .duration_since(timing.start_time)
        .unwrap()
        .as_micros() as i64;
    let busy = rng.random_range(0..duration_ns);
    let idle = duration_ns - busy;

    // Common attributes
    attributes.extend([
        KeyValue::new("code.file.path", "lambdas/dummy/src/main.rs"),
        KeyValue::new("code.module.name", format!("dummy::{name}")),
        KeyValue::new("code.line.number", Value::I64(rng.random_range(0..1000))),
        KeyValue::new("target", format!("dummy::{name}")),
        KeyValue::new("busy_ns", Value::I64(busy)),
        KeyValue::new("idle_ns", Value::I64(idle)),
    ]);
}

/// Returns a random internal operation name
fn get_internal_operation_name(rng: &mut StdRng) -> &'static str {
    let operations = [
        "serialize_request",
        "deserialize_response",
        "compute_hash",
        "validate_input",
        "format_output",
        "parse_json",
        "compress_data",
        "encrypt_payload",
    ];
    operations.choose(rng).unwrap()
}

/// Creates a parent (Server) span based on the parent span type
fn create_parent_span(
    trace_id: TraceId,
    span_id: SpanId,
    parent_type: ParentSpanType,
    timing: SpanTiming,
    rng: &mut StdRng,
) -> SpanData {
    let span_context = SpanContext::new(
        trace_id,
        span_id,
        TraceFlags::SAMPLED,
        false,
        TraceState::default(),
    );

    let mut attributes = generate_random_attributes(rng);

    // Common attributes
    add_common_attributes(rng, "server_handler", timing, &mut attributes);

    // Type-specific attributes
    match parent_type {
        ParentSpanType::Lambda => {
            attributes.extend([
                KeyValue::new("cloud.provider", "aws"),
                KeyValue::new("cloud.platform", "aws_lambda"),
                KeyValue::new("faas.trigger", "http"),
                KeyValue::new(
                    "cloud.resource_id",
                    format!(
                        "arn:aws:lambda:us-east-1:123456789012:function/handler-{}",
                        rng.random::<u16>()
                    ),
                ),
                KeyValue::new("faas.invocation_id", format!("{:x}", rng.random::<u128>())),
                KeyValue::new("cloud.account.id", "123456789012"),
                KeyValue::new("faas.coldstart", Value::Bool(rng.random())),
            ]);
        }
        ParentSpanType::Ec2 => {
            attributes.extend([
                KeyValue::new("cloud.provider", "aws"),
                KeyValue::new("cloud.platform", "aws_ec2"),
                KeyValue::new("host.id", format!("i-{:x}", rng.random::<u64>())),
                KeyValue::new("cloud.availability_zone", "us-east-1a"),
                KeyValue::new("host.type", "t3.medium"),
                KeyValue::new("host.image.id", format!("ami-{:x}", rng.random::<u64>())),
            ]);
        }
        ParentSpanType::EcsFargate | ParentSpanType::EcsEc2 => {
            attributes.extend([
                KeyValue::new("cloud.provider", "aws"),
                KeyValue::new("cloud.platform", "aws_ecs"),
                KeyValue::new(
                    "aws.ecs.launchtype",
                    match parent_type {
                        ParentSpanType::EcsFargate => "fargate",
                        ParentSpanType::EcsEc2 => "ec2",
                        _ => unreachable!(),
                    },
                ),
                KeyValue::new(
                    "container.name",
                    format!("app-container-{}", rng.random::<u16>()),
                ),
                KeyValue::new("container.id", format!("{:x}", rng.random::<u128>())),
                KeyValue::new(
                    "aws.ecs.container.arn",
                    format!(
                        "arn:aws:ecs:us-east-1:123456789012:container/{:x}",
                        rng.random::<u64>()
                    ),
                ),
                KeyValue::new(
                    "aws.ecs.cluster.arn",
                    "arn:aws:ecs:us-east-1:123456789012:cluster/my-cluster",
                ),
                KeyValue::new(
                    "aws.ecs.task.arn",
                    format!(
                        "arn:aws:ecs:us-east-1:123456789012:task/my-cluster/{:x}",
                        rng.random::<u64>()
                    ),
                ),
                KeyValue::new("aws.ecs.task.family", "my-task-family"),
            ]);
        }
        ParentSpanType::Eks => {
            attributes.extend([
                KeyValue::new("cloud.provider", "aws"),
                KeyValue::new("cloud.platform", "aws_eks"),
                KeyValue::new("k8s.cluster.name", "my-eks-cluster"),
                KeyValue::new("k8s.pod.name", format!("app-pod-{}", rng.random::<u16>())),
                KeyValue::new("k8s.pod.uid", format!("{:x}", rng.random::<u128>())),
            ]);
        }
        ParentSpanType::Beanstalk => {
            attributes.extend([
                KeyValue::new("cloud.provider", "aws"),
                KeyValue::new("cloud.platform", "aws_elastic_beanstalk"),
                KeyValue::new("service.namespace", "production"),
                KeyValue::new("service.version", format!("v{}", rng.random_range(1..100))),
                KeyValue::new("service.instance.id", format!("{}", rng.random::<u32>())),
            ]);
        }
    }

    let name = match parent_type {
        ParentSpanType::Lambda => "lambda_handler",
        ParentSpanType::Ec2 => "http_server",
        ParentSpanType::EcsFargate | ParentSpanType::EcsEc2 => "ecs_handler",
        ParentSpanType::Eks => "k8s_handler",
        ParentSpanType::Beanstalk => "beanstalk_app",
    }
    .into();

    SpanData {
        span_context,
        parent_span_id: SpanId::INVALID,
        parent_span_is_remote: false,
        span_kind: SpanKind::Server,
        name,
        start_time: timing.start_time,
        end_time: timing.end_time,
        attributes,
        dropped_attributes_count: 0,
        events: SpanEvents::default(),
        links: SpanLinks::default(),
        status: Status::Ok,
        instrumentation_scope: InstrumentationScope::builder("bench").build(),
    }
}

/// Creates a child (Client) span based on the child span type
fn create_child_span(
    trace_id: TraceId,
    span_id: SpanId,
    parent_span_id: SpanId,
    child_type: ChildSpanType,
    timing: SpanTiming,
    rng: &mut StdRng,
) -> SpanData {
    let span_context = SpanContext::new(
        trace_id,
        span_id,
        TraceFlags::SAMPLED,
        false,
        TraceState::default(),
    );

    let mut attributes = generate_random_attributes(rng);
    // Common attributes
    add_common_attributes(rng, "client", timing, &mut attributes);

    let mut events = SpanEvents::default();
    let status;
    let name;

    // Add type-specific attributes
    match child_type {
        ChildSpanType::AwsDynamoDb => {
            let operation = *["PutItem", "GetItem", "Query", "Scan"].choose(rng).unwrap();
            attributes.extend([
                KeyValue::new("rpc.system", "aws-api"),
                KeyValue::new("rpc.service", "DynamoDB"),
                KeyValue::new("rpc.method", operation),
                KeyValue::new("aws.service", "dynamodb"),
                KeyValue::new("aws.operation", operation),
                KeyValue::new("aws.region", "us-east-1"),
                KeyValue::new("aws.request_id", format!("{:X}", rng.random::<u128>())),
                KeyValue::new(
                    "aws.dynamodb.table_names",
                    Value::Array(opentelemetry::Array::String(vec![
                        opentelemetry::StringValue::from(format!("Table-{}", rng.random::<u16>())),
                    ])),
                ),
                KeyValue::new("http.response.status_code", Value::I64(200)),
                KeyValue::new("peer.service", "DynamoDB"),
            ]);
            name = format!("DynamoDB.{operation}").into();
            status = Status::Ok;
        }
        ChildSpanType::AwsS3 => {
            let operation = *["GetObject", "PutObject", "DeleteObject"]
                .choose(rng)
                .unwrap();
            attributes.extend([
                KeyValue::new("rpc.system", "aws-api"),
                KeyValue::new("rpc.service", "S3"),
                KeyValue::new("rpc.method", operation),
                KeyValue::new("aws.service", "s3"),
                KeyValue::new("aws.operation", operation),
                KeyValue::new("aws.s3.bucket", format!("bucket-{}", rng.random::<u16>())),
                KeyValue::new("aws.s3.key", format!("data/{}.json", rng.random::<u16>())),
                KeyValue::new("aws.region", "us-west-2"),
                KeyValue::new("aws.request_id", format!("{:X}", rng.random::<u128>())),
                KeyValue::new("http.response.status_code", Value::I64(200)),
                KeyValue::new(
                    "http.response.body.size",
                    Value::I64(rng.random_range(1024..8192)),
                ),
                KeyValue::new("rpc.message.type", "RECEIVED"),
                KeyValue::new("peer.service", "S3"),
            ]);
            name = format!("S3.{operation}").into();
            status = Status::Ok;
        }
        ChildSpanType::AwsSqs => {
            let operation = *["SendMessage", "ReceiveMessage", "DeleteMessage"]
                .choose(rng)
                .unwrap();
            attributes.extend([
                KeyValue::new("rpc.system", "aws-api"),
                KeyValue::new("rpc.service", "SQS"),
                KeyValue::new("rpc.method", operation),
                KeyValue::new("aws.service", "sqs"),
                KeyValue::new("aws.operation", operation),
                KeyValue::new(
                    "aws.sqs.queue_url",
                    format!(
                        "https://sqs.us-east-1.amazonaws.com/123456789012/Queue-{}",
                        rng.random::<u16>()
                    ),
                ),
                KeyValue::new("aws.region", "us-east-1"),
                KeyValue::new("aws.request_id", format!("{:X}", rng.random::<u128>())),
                KeyValue::new("http.response.status_code", Value::I64(200)),
                KeyValue::new("peer.service", "SQS"),
            ]);
            name = format!("SQS.{operation}").into();
            status = Status::Ok;
        }
        ChildSpanType::HttpExternal => {
            let method = *["GET", "POST", "PUT", "DELETE"].choose(rng).unwrap();
            attributes.extend([
                KeyValue::new("peer.service", "external-api"),
                KeyValue::new("http.request.method", method),
                KeyValue::new("url.scheme", "https"),
                KeyValue::new("url.domain", "external-api.com"),
                KeyValue::new("url.port", Value::I64(443)),
                KeyValue::new("url.path", format!("/api/resource/{}", rng.random::<u16>())),
                KeyValue::new("url.query", "param=value"),
                KeyValue::new(
                    "client.address",
                    format!(
                        "10.0.{}.{}",
                        rng.random_range(1..255),
                        rng.random_range(1..255)
                    ),
                ),
                KeyValue::new("http.response.status_code", Value::I64(200)),
                KeyValue::new(
                    "http.response.body.size",
                    Value::I64(rng.random_range(512..4096)),
                ),
                KeyValue::new("rpc.message.type", "RECEIVED"),
            ]);
            name = format!("{method} /api/resource").into();
            status = Status::Ok;
        }
        ChildSpanType::SqlPostgres => {
            let operation = *["SELECT", "INSERT", "UPDATE", "DELETE"]
                .choose(rng)
                .unwrap();
            attributes.extend([
                KeyValue::new("db.system.name", "postgresql"),
                KeyValue::new("db.namespace", format!("db_{}", rng.random::<u16>())),
                KeyValue::new("db.operation.name", operation),
                KeyValue::new("db.collection.name", "table_name"),
                KeyValue::new("server.address", "postgres.example.com"),
                KeyValue::new("server.port", Value::I64(5432)),
                KeyValue::new(
                    "db.query.text",
                    format!("{operation} FROM table_name WHERE id = ?"),
                ),
                KeyValue::new("peer.service", "postgresql"),
            ]);
            name = format!("{operation} table_name").into();
            status = Status::Ok;
        }
        ChildSpanType::SqlMysql => {
            let operation = *["SELECT", "INSERT", "UPDATE", "DELETE"]
                .choose(rng)
                .unwrap();
            attributes.extend([
                KeyValue::new("db.system.name", "mysql"),
                KeyValue::new("db.namespace", format!("db_{}", rng.random::<u16>())),
                KeyValue::new("db.operation.name", operation),
                KeyValue::new("db.collection.name", "orders"),
                KeyValue::new("server.address", "mysql.example.com"),
                KeyValue::new("server.port", Value::I64(3306)),
                KeyValue::new(
                    "db.query.text",
                    format!("{operation} FROM orders WHERE user_id = ?"),
                ),
                KeyValue::new("peer.service", "mysql"),
            ]);
            name = format!("{operation} orders").into();
            status = Status::Ok;
        }
        ChildSpanType::HttpWithError => {
            let (status_code, exception_type, exception_message) = *[
                (429, "RateLimitExceeded", "Rate limit exceeded"),
                (500, "ConnectionTimeout", "Connection timeout"),
                (404, "NotFound", "Resource not found"),
                (503, "ServiceUnavailable", "Service unavailable"),
            ]
            .choose(rng)
            .unwrap();

            attributes.extend([
                KeyValue::new("http.request.method", "POST"),
                KeyValue::new("url.full", "https://api.failing.com/endpoint"),
                KeyValue::new(
                    "client.address",
                    format!(
                        "10.0.{}.{}",
                        rng.random_range(1..255),
                        rng.random_range(1..255)
                    ),
                ),
                KeyValue::new("http.response.status_code", Value::I64(status_code)),
                KeyValue::new("peer.service", "failing-service"),
            ]);

            events.events.push(opentelemetry::trace::Event::new(
                "exception",
                timing.start_time + Duration::from_millis(20),
                vec![
                    KeyValue::new("exception.type", exception_type),
                    KeyValue::new("exception.message", exception_message),
                    KeyValue::new("exception.stacktrace", r#"
                        0: __rustc::rust_begin_unwind
                            at /rustc/254b59607d4417e9dffbc307138ae5c86280fe4c/library/std/src/panicking.rs:689:5
                        1: core::panicking::panic_fmt
                            at /rustc/254b59607d4417e9dffbc307138ae5c86280fe4c/library/core/src/panicking.rs:80:14
                        2: playground::module::some_function
                            at ./src/main.rs:3:9
                        3: playground::main
                            at ./src/main.rs:10:5
                    "#),
                ],
                0,
            ));

            name = "POST /endpoint".into();
            status = if status_code >= 500 {
                Status::error("Server error")
            } else {
                Status::error("Client error")
            };
        }
        ChildSpanType::GenericRemote => {
            let method = *["GET", "POST"].choose(rng).unwrap();
            attributes.extend([
                KeyValue::new("peer.service", "generic-service"),
                KeyValue::new("http.request.method", method),
                KeyValue::new(
                    "url.full",
                    format!("https://service{}.com/api", rng.random::<u16>()),
                ),
                KeyValue::new("http.response.status_code", Value::I64(200)),
            ]);
            name = format!("{method} /api").into();
            status = Status::Ok;
        }
    }

    SpanData {
        span_context,
        parent_span_id,
        parent_span_is_remote: false,
        span_kind: SpanKind::Client,
        name,
        start_time: timing.start_time,
        end_time: timing.end_time,
        attributes,
        dropped_attributes_count: 0,
        events,
        links: SpanLinks::default(),
        status,
        instrumentation_scope: InstrumentationScope::builder("bench").build(),
    }
}

/// Creates a grandchild (Internal) span
fn create_grandchild_span(
    trace_id: TraceId,
    parent_span_id: SpanId,
    timing: SpanTiming,
    rng: &mut StdRng,
) -> SpanData {
    let span_id = SpanId::from_bytes(rng.random());
    let span_context = SpanContext::new(
        trace_id,
        span_id,
        TraceFlags::SAMPLED,
        false,
        TraceState::default(),
    );

    let mut attributes = generate_random_attributes(rng);
    // Common attributes
    let op = get_internal_operation_name(rng);
    add_common_attributes(rng, op, timing, &mut attributes);

    SpanData {
        span_context,
        parent_span_id,
        parent_span_is_remote: false,
        span_kind: SpanKind::Internal,
        name: get_internal_operation_name(rng).into(),
        start_time: timing.start_time,
        end_time: timing.end_time,
        attributes,
        dropped_attributes_count: 0,
        events: SpanEvents::default(),
        links: SpanLinks::default(),
        status: Status::Ok,
        instrumentation_scope: InstrumentationScope::builder("stress-test").build(),
    }
}

fn get_rng() -> StdRng {
    StdRng::seed_from_u64(42) // Deterministic for benchmarking
}

/// Generate resource with telemetry SDK attributes (generic over all spans)
fn generate_resource() -> Resource {
    Resource::builder()
        .with_attributes(vec![
            KeyValue::new("telemetry.sdk.name", "opentelemetry"),
            KeyValue::new("telemetry.sdk.version", "0.31.0"),
            KeyValue::new("telemetry.sdk.language", "rust"),
            KeyValue::new("telemetry.auto.version", "0.0.1-bench"),
            KeyValue::new("service.name", "bench-service"),
        ])
        .build()
}

fn generate_valid_trace_id(rng: &mut StdRng) -> TraceId {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as u128;

    let random_part = rng.random::<u128>();
    let trace_id = (ts << 96) | (random_part >> 32);
    TraceId::from_bytes(trace_id.to_be_bytes())
}

/// Generate a batch of benchmark spans with 3-level nesting
/// Each sub-level will have 1-3 siblings
fn generate_spans(count: usize) -> Vec<SpanData> {
    let mut rng = get_rng();
    let mut spans = Vec::with_capacity(count);

    loop {
        let trace_id = generate_valid_trace_id(&mut rng);
        let span_timings = SpanTiming::random(&mut rng);

        let max_children = 3.min(count - spans.len() - 1); // Acknowledge for the parent spot
        let children_count = rng.random_range(1..=max_children);
        let children_timings = SpanTiming::random_sub(&mut rng, span_timings, children_count);
        let mut max_grandchildren =
            (3 * children_timings.len()).min(count - spans.len() - 1 - children_timings.len()); // Acknowledge for the parent and children spots
        let grand_children_timings = children_timings
            .iter()
            .map(|t| {
                if max_grandchildren > 0 {
                    let max = 3.min(max_grandchildren);
                    let grandchildren_count = rng.random_range(1..=max);
                    let timings = SpanTiming::random_sub(&mut rng, *t, grandchildren_count);
                    max_grandchildren -= timings.len();
                    timings
                } else {
                    vec![]
                }
            })
            .collect::<Vec<_>>();

        // Push the parent
        let parent_span_id = SpanId::from_bytes(rng.random());
        let parent_type = ParentSpanType::random(&mut rng);
        spans.push(create_parent_span(
            trace_id,
            parent_span_id,
            parent_type,
            span_timings,
            &mut rng,
        ));
        for (child_timing, grand_children_timings) in children_timings
            .into_iter()
            .zip(grand_children_timings.into_iter())
        {
            let span_id = SpanId::from_bytes(rng.random());
            let child_type = ChildSpanType::random(&mut rng);
            spans.push(create_child_span(
                trace_id,
                span_id,
                parent_span_id,
                child_type,
                child_timing,
                &mut rng,
            ));
            for grandchild_timing in grand_children_timings {
                spans.push(create_grandchild_span(
                    trace_id,
                    span_id,
                    grandchild_timing,
                    &mut rng,
                ));
            }
        }

        // Quit when filled
        if spans.len() == count {
            break;
        }
    }

    spans
}

fn _benchmark_translation_and_export(
    group: &mut BenchmarkGroup<'_, WallTime>,
    benchmark_id_prefix: &'static str,
    with_subsegment_nesting: bool,
) {
    let runtime = tokio::runtime::Runtime::new().unwrap();

    // Set up UDP receiver
    let receiver_addr: SocketAddr = "127.0.0.1:12000".parse().unwrap();
    let receiver = UdpReceiverControler::new(receiver_addr).expect("Failed to create UDP receiver");
    receiver.start();

    // Give the receiver thread a moment to start
    thread::sleep(Duration::from_millis(10));

    // Create XRay exporter that connects to our test receiver
    let daemon_client =
        XrayDaemonClient::new(receiver_addr).expect("Failed to create daemon client");

    let mut exporter = if with_subsegment_nesting {
        XrayExporter::new(daemon_client)
            .with_translator(SegmentTranslator::new().always_nest_subsegments())
    } else {
        XrayExporter::new(daemon_client)
    };

    // Set up resource with telemetry attributes
    let resource = generate_resource();
    exporter.set_resource(&resource);

    // Test different batch sizes
    for batch_size in BATCH_SIZES {
        let spans = generate_spans(batch_size);

        receiver.reset_counters();

        group.bench_with_input(
            BenchmarkId::new(benchmark_id_prefix, batch_size),
            &spans,
            |b, spans| {
                b.to_async(&runtime).iter_batched(
                    || spans.clone(),
                    |spans| async { black_box(exporter.export(spans).await) },
                    BatchSize::SmallInput,
                );
            },
        );

        let (packets_received, bytes_received) = receiver.report();
        // Print some statistics
        println!(
            "Final stats - Packets received: {packets_received}, Bytes received: {bytes_received}"
        );
    }

    receiver.stop();
}
/// Benchmark the full XRay export pipeline without nesting subsegments
fn benchmark_translation_and_export(c: &mut Criterion) {
    let mut group = c.benchmark_group("translation_and_export");
    _benchmark_translation_and_export(&mut group, "with_nesting", true);
    _benchmark_translation_and_export(&mut group, "without_nesting", false);
    group.finish();
}

/// Benchmark just the translation phase (SpanData -> SegmentDocument)
fn _benchmark_translation_only(
    group: &mut BenchmarkGroup<'_, WallTime>,
    benchmark_id_prefix: &'static str,
    with_subsegment_nesting: bool,
) {
    let translator = if with_subsegment_nesting {
        SegmentTranslator::new().always_nest_subsegments()
    } else {
        SegmentTranslator::new()
    };

    // Test spans translation
    for batch_size in BATCH_SIZES {
        let spans = generate_spans(batch_size);

        group.bench_with_input(
            BenchmarkId::new(benchmark_id_prefix, batch_size),
            &spans,
            |b, spans| {
                b.iter(|| black_box(translator.translate_spans(spans)));
            },
        );
    }
}
/// Benchmark just the translation phase (SpanData -> SegmentDocument) without nesting subsegments
fn benchmark_translation_only(c: &mut Criterion) {
    let mut group = c.benchmark_group("translation_only");
    _benchmark_translation_only(&mut group, "with_nesting", true);
    _benchmark_translation_only(&mut group, "without_nesting", false);
    group.finish();
}

/// Benchmark just the serialization phase (SegmentDocument -> JSON bytes)
fn benchmark_export_only(c: &mut Criterion) {
    let runtime = tokio::runtime::Runtime::new().unwrap();

    // Set up UDP receiver
    let receiver_addr: SocketAddr = "127.0.0.1:12000".parse().unwrap();
    let receiver = UdpReceiverControler::new(receiver_addr).expect("Failed to create UDP receiver");
    receiver.start();

    // Give the receiver thread a moment to start
    thread::sleep(Duration::from_millis(10));

    // Create XRay exporter that connects to our test receiver
    let daemon_client =
        XrayDaemonClient::new(receiver_addr).expect("Failed to create daemon client");

    let mut translator = SegmentTranslator::default();
    // Set up resource with telemetry attributes
    let resource = generate_resource();
    translator.set_resource(&resource);

    let mut group = c.benchmark_group("export_only");

    // Test different batch sizes
    for batch_size in BATCH_SIZES {
        let spans = generate_spans(batch_size);

        receiver.reset_counters();

        group.bench_with_input(BenchmarkId::new("spans", batch_size), &spans, |b, spans| {
            b.to_async(&runtime).iter_batched(
                || {
                    translator
                        .translate_spans(spans)
                        .expect("Failed to translate spans")
                },
                |docs| async { black_box(daemon_client.export_segment_documents(docs).await) },
                BatchSize::SmallInput,
            );
        });

        let (packets_received, bytes_received) = receiver.report();
        // Print some statistics
        println!(
            "Final stats - Packets received: {packets_received}, Bytes received: {bytes_received}"
        );
    }

    group.finish();
    receiver.stop();
}

criterion_group!(
    name = benches;
    config = Criterion::default().sample_size(500).measurement_time(Duration::from_secs(10)).with_profiler(PProfProfiler::default());
    targets =
        benchmark_translation_and_export,
        benchmark_translation_only,
        benchmark_export_only,
);

criterion_main!(benches);

use std::fs::OpenOptions;
struct PProfProfiler<'a> {
    frequency: i32,
    active_profiler: Option<ProfilerGuard<'a>>,
}
impl Default for PProfProfiler<'_> {
    fn default() -> Self {
        Self {
            frequency: 10000,
            active_profiler: None,
        }
    }
}
impl Profiler for PProfProfiler<'_> {
    fn start_profiling(&mut self, _benchmark_id: &str, _benchmark_dir: &Path) {
        println!("Profiling started");
        self.active_profiler = Some(ProfilerGuard::new(self.frequency).unwrap());
    }

    fn stop_profiling(&mut self, _benchmark_id: &str, benchmark_dir: &Path) {
        std::fs::create_dir_all(benchmark_dir).unwrap();

        let output_path = benchmark_dir.join("flamegraph.svg");
        let output_file = match OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&output_path)
        {
            Ok(f) => f,
            Err(_) => {
                let mv_output_path = benchmark_dir.join("flamegraph.old.svg");
                std::fs::rename(&output_path, mv_output_path).unwrap_or_else(|_| {
                    panic!("File system error while creating {}", output_path.display())
                });
                File::create(&output_path).unwrap_or_else(|_| {
                    panic!("File system error while creating {}", output_path.display())
                })
            }
        };

        if let Some(profiler) = self.active_profiler.take() {
            let default_options = &mut Options::default();

            profiler
                .report()
                .build()
                .unwrap()
                .flamegraph_with_options(output_file, default_options)
                .expect("Error while writing flamegraph");
        }
    }
}
