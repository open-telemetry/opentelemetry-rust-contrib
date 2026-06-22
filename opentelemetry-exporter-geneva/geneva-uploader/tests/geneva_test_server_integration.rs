use geneva_test_server::testing::TestServer;
use geneva_uploader::{AuthMethod, GenevaClient, GenevaClientConfig};
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

    let batches = client
        .encode_and_compress_logs(&request_view)
        .expect("batch should encode");
    assert_eq!(batches.len(), 1);
    client
        .upload_batch(&batches[0])
        .await
        .expect("batch should upload");

    let detail = server.wait_for_request(&batches[0].event_name).await;
    assert_eq!(detail["decode_status"], "decoded");
    assert_eq!(detail["event_name"], "CheckoutEvent");
    assert_eq!(detail["row_count"], 1);

    let records = detail["records"].as_array().expect("records array");
    assert_eq!(records.len(), 1);
    let payload = &records[0]["payload"];
    assert_eq!(payload["Role"], "checkout");
    assert_eq!(payload["RoleInstance"], "instance-1");
    assert_eq!(payload["body"], "checkout failed");
    assert_eq!(payload["operation"], "checkout");
    assert_eq!(payload["result"], 127);
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
