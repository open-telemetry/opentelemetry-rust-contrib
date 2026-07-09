use geneva_test_server::testing::TestServer;
use geneva_uploader::{AuthMethod, GenevaClient, GenevaClientConfig, LogMethod, SpanMethod};
use opentelemetry_proto::tonic::collector::logs::v1::ExportLogsServiceRequest;
use opentelemetry_proto::tonic::common::v1::{any_value::Value, AnyValue, KeyValue};
use opentelemetry_proto::tonic::logs::v1::{LogRecord, ResourceLogs, ScopeLogs};
use otap_df_pdata::views::otlp::bytes::logs::RawLogsData;
use prost::Message as _;

#[tokio::test]
#[ignore = "run by the dedicated geneva-uploader test-server CI job"]
async fn uploader_batch_is_accepted_and_decoded_by_test_server() {
    let server = TestServer::start().await;
    let client = GenevaClient::new(GenevaClientConfig {
        endpoint: server.base_url().to_string(),
        environment: "testenv".to_string(),
        account: "testaccount".to_string(),
        namespace: "TestNamespace".to_string(),
        region: "testregion".to_string(),
        config_major_version: 1,
        auth_method: AuthMethod::MockAuth,
        tenant: "tenant-a".to_string(),
        role_name: "checkout".to_string(),
        role_instance: "instance-1".to_string(),
        msi_resource: None,
        logs: LogMethod {
            default_event_name: None,
        },
        spans: SpanMethod {
            default_event_name: None,
        },
        obo_event_map: None,
    })
    .expect("client should initialize");

    let request = ExportLogsServiceRequest {
        resource_logs: vec![ResourceLogs {
            scope_logs: vec![ScopeLogs {
                log_records: vec![LogRecord {
                    time_unix_nano: 1_718_432_000_000_000_000,
                    event_name: "CheckoutEvent".to_string(),
                    severity_number: 17,
                    severity_text: "ERROR".to_string(),
                    attributes: vec![
                        string_attr("operation", "checkout"),
                        int_attr("result", 127),
                    ],
                    body: Some(AnyValue {
                        value: Some(Value::StringValue("checkout failed".to_string())),
                    }),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        }],
    };
    let request_bytes = request.encode_to_vec();
    let request_view = RawLogsData::new(&request_bytes);

    let detail = upload_single_batch_and_wait(&client, &server, &request_view).await;
    assert_eq!(detail["decode_status"], "decoded");
    assert_eq!(detail["event_name"], "Log");
    assert_eq!(detail["row_count"], 1);

    let records = detail["records"].as_array().expect("records array");
    assert_eq!(records.len(), 1);
    let payload = &records[0]["payload"];
    assert_eq!(payload["Role"], "checkout");
    assert_eq!(payload["RoleInstance"], "instance-1");
    assert_eq!(payload["body"], "checkout failed");
    assert_eq!(payload["operation"], "checkout");
    assert_eq!(payload["result"], 127);

    let common_schema_request = ExportLogsServiceRequest {
        resource_logs: vec![ResourceLogs {
            scope_logs: vec![ScopeLogs {
                log_records: vec![LogRecord {
                    time_unix_nano: 1_718_432_000_000_000_000,
                    attributes: vec![
                        int_attr("__csver__", 0x400),
                        string_attr("PartA.time", "2024-06-15T06:00:00Z"),
                        string_attr("PartA.ext_cloud_role", "checkout"),
                        string_attr("PartA.ext_cloud_roleInstance", "instance-1"),
                        string_attr("PartC.operation", "checkout"),
                        int_attr("PartC.result", 127),
                        string_attr("PartB._typeName", "Log"),
                        string_attr("PartB.name", "CommonSchemaCheckoutEvent"),
                        string_attr("PartB.body", "common schema checkout failed"),
                        int_attr("PartB.severityNumber", 17),
                        string_attr("PartB.severityText", "ERROR"),
                    ],
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        }],
    };
    let common_schema_request_bytes = common_schema_request.encode_to_vec();
    let common_schema_request_view = RawLogsData::new(&common_schema_request_bytes);

    let common_schema_detail =
        upload_single_batch_and_wait(&client, &server, &common_schema_request_view).await;
    assert_eq!(common_schema_detail["decode_status"], "decoded");
    assert_eq!(common_schema_detail["event_name"], "Log");
    assert_eq!(common_schema_detail["row_count"], 1);

    let records = common_schema_detail["records"]
        .as_array()
        .expect("common schema records array");
    assert_eq!(records.len(), 1);
    let payload = &records[0]["payload"];
    assert_eq!(payload["Role"], "checkout");
    assert_eq!(payload["RoleInstance"], "instance-1");
    assert_eq!(payload["body"], "common schema checkout failed");
    assert_eq!(payload["SeverityNumber"], 17);
    assert_eq!(payload["SeverityText"], "ERROR");
    assert_eq!(payload["operation"], "checkout");
    assert_eq!(payload["result"], 127);
}

async fn upload_single_batch_and_wait(
    client: &GenevaClient,
    server: &TestServer,
    request_view: &RawLogsData<'_>,
) -> serde_json::Value {
    let batches = client
        .encode_and_compress_logs(request_view)
        .expect("batch should encode");
    assert_eq!(batches.len(), 1);
    client
        .upload_batch(&batches[0])
        .await
        .expect("batch should upload");

    server.wait_for_request(&batches[0].event_name).await
}

fn string_attr(key: &str, value: &str) -> KeyValue {
    KeyValue {
        key: key.to_string(),
        key_strindex: 0,
        value: Some(AnyValue {
            value: Some(Value::StringValue(value.to_string())),
        }),
    }
}

fn int_attr(key: &str, value: i64) -> KeyValue {
    KeyValue {
        key: key.to_string(),
        key_strindex: 0,
        value: Some(AnyValue {
            value: Some(Value::IntValue(value)),
        }),
    }
}
