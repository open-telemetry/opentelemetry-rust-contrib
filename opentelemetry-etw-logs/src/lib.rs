//! The ETW exporter will enable applications to use OpenTelemetry API
//! to capture the telemetry events, and write them to the ETW subsystem.
//!
//! ## Resource Attribute Handling
//!
//! **Important**: By default, resource attributes are NOT exported with log records.
//! The ETW exporter only automatically exports these specific resource attributes:
//!
//! - **`service.name`** → Exported as `cloud.roleName` in PartA of Common Schema
//! - **`service.instance.id`** → Exported as `cloud.roleInstance` in PartA of Common Schema
//!
//! All other resource attributes are ignored unless explicitly specified.
//!
//! ### Opting in to Additional Resource Attributes
//!
//! To export additional resource attributes, use the `with_resource_attributes()` method:
//!
//! ```rust
//! use opentelemetry_sdk::logs::SdkLoggerProvider;
//! use opentelemetry_sdk::Resource;
//! use opentelemetry_etw_logs::Processor;
//! use opentelemetry::KeyValue;
//!
//! let etw_processor = Processor::builder("myprovider")
//!     // Only export specific resource attributes
//!     .with_resource_attributes(["custom_attribute1", "custom_attribute2"])
//!     .build()
//!     .unwrap();
//!
//! let provider = SdkLoggerProvider::builder()
//!     .with_resource(
//!         Resource::builder_empty()
//!             .with_service_name("example")
//!             .with_attribute(KeyValue::new("custom_attribute1", "value1"))
//!             .with_attribute(KeyValue::new("custom_attribute2", "value2"))
//!             .with_attribute(KeyValue::new("custom_attribute3", "value3"))  // This won't be exported
//!             .build(),
//!     )
//!     .with_log_processor(etw_processor)
//!     .build();
//! ```
//!
//! ### Performance Considerations for ETW
//!
//! **Warning**: Each specified resource attribute will be serialized and sent
//! with EVERY log record. This is different from OTLP exporters where resource
//! attributes are serialized once per batch. Consider the performance impact
//! when selecting which attributes to export.
//!
//! **Recommendation**: Be selective about which resource attributes to export.
//! Since ETW writes to a local kernel buffer and requires a local
//! listener/agent, the agent can often deduce many resource attributes without
//! requiring them to be sent with each log:
//!
//! - **Infrastructure attributes** (datacenter, region, availability zone) can
//!   be determined by the local agent.
//! - **Host attributes** (hostname, IP address, OS version) are available locally.
//! - **Deployment attributes** (environment, cluster) may be known to the agent.
//!
//! Focus on attributes that are truly specific to your application instance
//! and cannot be easily determined by the local agent.

#![warn(missing_debug_implementations, missing_docs)]

#[cfg(feature = "serde_json")]
mod converters;
mod exporter;
mod processor;

pub use processor::Processor;
pub use processor::ProcessorBuilder;

#[cfg(all(test, target_os = "windows"))]
mod integration_tests {
    use std::collections::HashMap;
    use std::sync::mpsc;
    use std::time::Duration;

    use ferrisetw::schema_locator::SchemaLocator;
    use ferrisetw::trace::UserTrace;
    use ferrisetw::EventRecord;

    use tracelogging_dynamic as tld;

    use opentelemetry::logs::{AnyValue, LogRecord, Logger, LoggerProvider};
    use opentelemetry::KeyValue;
    use opentelemetry_sdk::logs::SdkLoggerProvider;
    use opentelemetry_sdk::Resource;

    // -----------------------------------------------------------------------
    // TDH-based ETW event property parser
    //
    // ferrisetw's Parser cannot handle TraceLogging struct properties (PartA,
    // PartB, ext_dt, etc.) — it returns UnimplementedType("structure").
    // We use TDH APIs directly to enumerate all event properties from the
    // schema (TRACE_EVENT_INFO) and read their values from the raw data
    // buffer, producing a flat HashMap keyed by dotted paths like
    // "PartA.ext_cloud.role" or "PartB.body".
    // -----------------------------------------------------------------------

    /// Captured event with all properties parsed into named fields.
    struct CapturedEvent {
        event_name: String,
        level: u8,
        keyword: u64,
        /// Properties keyed by dotted path: "PartA.ext_cloud.role", "PartB.body", etc.
        /// Values stored as raw bytes — use typed accessors to decode.
        properties: HashMap<String, Vec<u8>>,
    }

    impl CapturedEvent {
        fn get_str(&self, key: &str) -> Option<&str> {
            self.properties
                .get(key)
                .and_then(|v| std::str::from_utf8(v).ok())
        }

        fn get_u16(&self, key: &str) -> Option<u16> {
            self.properties
                .get(key)
                .filter(|v| v.len() >= 2)
                .map(|v| u16::from_le_bytes([v[0], v[1]]))
        }

        fn get_i16(&self, key: &str) -> Option<i16> {
            self.properties
                .get(key)
                .filter(|v| v.len() >= 2)
                .map(|v| i16::from_le_bytes([v[0], v[1]]))
        }

        fn get_i64(&self, key: &str) -> Option<i64> {
            self.properties
                .get(key)
                .filter(|v| v.len() >= 8)
                .map(|v| i64::from_le_bytes(v[..8].try_into().unwrap()))
        }

        fn get_f64(&self, key: &str) -> Option<f64> {
            self.properties
                .get(key)
                .filter(|v| v.len() >= 8)
                .map(|v| f64::from_le_bytes(v[..8].try_into().unwrap()))
        }

        fn get_bool(&self, key: &str) -> Option<bool> {
            self.properties
                .get(key)
                .filter(|v| v.len() >= 4)
                .map(|v| i32::from_le_bytes(v[..4].try_into().unwrap()) != 0)
        }

        fn has(&self, key: &str) -> bool {
            self.properties.contains_key(key)
        }
    }

    /// Parse all event properties using Windows TDH APIs.
    ///
    /// Walks the TRACE_EVENT_INFO property array (which mirrors the
    /// TraceLogging metadata) and reads leaf values from the user data
    /// buffer. Struct entries (PartA, PartB, ext_dt, etc.) occupy 0 bytes
    /// in the data and are used only to build dotted path names.
    ///
    /// # Safety
    /// `er` must point to a valid EVENT_RECORD for the duration of the call.
    #[allow(unsafe_op_in_unsafe_fn)]
    unsafe fn parse_event_properties(
        er: *const windows::Win32::System::Diagnostics::Etw::EVENT_RECORD,
    ) -> HashMap<String, Vec<u8>> {
        use windows::Win32::System::Diagnostics::Etw::{
            TdhGetEventInformation, EVENT_PROPERTY_INFO, TRACE_EVENT_INFO,
        };

        let mut result = HashMap::new();

        // Query required buffer size
        let mut buf_size = 0u32;
        let _ = TdhGetEventInformation(er, None, None, &mut buf_size);
        if buf_size == 0 {
            return result;
        }

        // Allocate and retrieve TRACE_EVENT_INFO
        let mut buf = vec![0u8; buf_size as usize];
        let tei_ptr = buf.as_mut_ptr() as *mut TRACE_EVENT_INFO;
        let status = TdhGetEventInformation(er, None, Some(tei_ptr), &mut buf_size);
        if status != 0 {
            return result;
        }

        let tei = &*tei_ptr;
        let top_level_count = tei.TopLevelPropertyCount as usize;
        let prop_count = tei.PropertyCount as usize;

        let props = std::slice::from_raw_parts(tei.EventPropertyInfoArray.as_ptr(), prop_count);

        // User data buffer (leaf values only; struct headers are 0 bytes)
        let data: &[u8] = if (*er).UserData.is_null() || (*er).UserDataLength == 0 {
            &[]
        } else {
            std::slice::from_raw_parts((*er).UserData as *const u8, (*er).UserDataLength as usize)
        };

        let base = buf.as_ptr();
        let mut data_pos = 0usize;

        // Recursive DFS: TDH lists top-level properties at [0..TopLevelPropertyCount).
        // Struct entries use StructStartIndex/NumOfStructMembers to reference their
        // children elsewhere in the array. The data buffer follows DFS order.
        fn visit(
            props: &[EVENT_PROPERTY_INFO],
            indices: &[usize],
            prefix: &str,
            base: *const u8,
            data: &[u8],
            data_pos: &mut usize,
            result: &mut HashMap<String, Vec<u8>>,
        ) {
            for &idx in indices {
                let prop = &props[idx];
                let name = unsafe { read_wide_string_at(base, prop.NameOffset as usize) };
                let is_struct = (prop.Flags.0 & 0x1) != 0;
                let full_path = if prefix.is_empty() {
                    name.clone()
                } else {
                    format!("{prefix}.{name}")
                };

                if is_struct {
                    let start = unsafe { prop.Anonymous1.structType.StructStartIndex } as usize;
                    let count = unsafe { prop.Anonymous1.structType.NumOfStructMembers } as usize;
                    let child_indices: Vec<usize> = (start..start + count).collect();
                    visit(
                        props,
                        &child_indices,
                        &full_path,
                        base,
                        data,
                        data_pos,
                        result,
                    );
                } else {
                    let in_type = unsafe { prop.Anonymous1.nonStructType.InType };
                    let (value, size) = read_leaf_value(data, *data_pos, in_type);
                    result.insert(full_path, value);
                    *data_pos += size;
                }
            }
        }

        let top_level_indices: Vec<usize> = (0..top_level_count).collect();
        visit(
            props,
            &top_level_indices,
            "",
            base,
            data,
            &mut data_pos,
            &mut result,
        );

        result
    }

    #[allow(unsafe_op_in_unsafe_fn)]
    unsafe fn read_wide_string_at(base: *const u8, offset: usize) -> String {
        let ptr = base.add(offset) as *const u16;
        let mut len = 0;
        while *ptr.add(len) != 0 {
            len += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len))
    }

    /// Read a leaf value from the data buffer based on its TDH InType.
    ///
    /// Returns (raw_bytes, total_bytes_consumed_from_buffer).
    fn read_leaf_value(data: &[u8], pos: usize, in_type: u16) -> (Vec<u8>, usize) {
        match in_type {
            // Counted types: u16 byte-length prefix + data bytes
            // 1=UnicodeString, 2=AnsiString, 14=Binary,
            // 300=CountedString(UTF-16), 301=CountedAnsiString(UTF-8)
            1 | 2 | 14 | 300 | 301 => {
                let byte_len = u16::from_le_bytes([data[pos], data[pos + 1]]) as usize;
                (data[pos + 2..pos + 2 + byte_len].to_vec(), 2 + byte_len)
            }
            // Fixed-size types
            3 | 4 => (data[pos..pos + 1].to_vec(), 1), // Int8 / UInt8
            5 | 6 => (data[pos..pos + 2].to_vec(), 2), // Int16 / UInt16
            7 | 8 | 13 => (data[pos..pos + 4].to_vec(), 4), // Int32 / UInt32 / Bool32
            9 | 10 => (data[pos..pos + 8].to_vec(), 8), // Int64 / UInt64
            11 => (data[pos..pos + 4].to_vec(), 4),    // Float32
            12 => (data[pos..pos + 8].to_vec(), 8),    // Float64
            _ => panic!("Unsupported TDH InType: {in_type}"),
        }
    }

    /// Start a ferrisetw UserTrace session for the given provider name.
    fn start_etw_trace(
        provider_name: &str,
    ) -> (ferrisetw::trace::UserTrace, mpsc::Receiver<CapturedEvent>) {
        use windows::Win32::System::Diagnostics::Etw::EVENT_RECORD;

        let guid_bytes = tld::Guid::from_name(provider_name).to_utf8_bytes();
        let guid_str = std::str::from_utf8(&guid_bytes).unwrap();

        let (tx, rx) = mpsc::sync_channel::<CapturedEvent>(16);

        let etw_provider = ferrisetw::provider::Provider::by_guid(guid_str)
            .add_callback(
                move |record: &EventRecord, _schema_locator: &SchemaLocator| {
                    let er_ptr = record as *const EventRecord as *const EVENT_RECORD;
                    let properties = unsafe { parse_event_properties(er_ptr) };

                    let captured = CapturedEvent {
                        event_name: record.event_name(),
                        level: record.level(),
                        keyword: record.keyword(),
                        properties,
                    };
                    let _ = tx.try_send(captured);
                },
            )
            .build();

        let trace = UserTrace::new()
            .enable(etw_provider)
            .start_and_process()
            .unwrap();

        (trace, rx)
    }

    /// Receive a captured event with the given event name, within a timeout.
    fn recv_event(rx: &mpsc::Receiver<CapturedEvent>, expected_name: &str) -> CapturedEvent {
        let deadline = Duration::from_secs(10);
        let start = std::time::Instant::now();
        loop {
            match rx.recv_timeout(deadline.saturating_sub(start.elapsed())) {
                Ok(evt) if evt.event_name == expected_name => return evt,
                Ok(_) => continue,
                Err(_) => panic!("Timed out waiting for ETW event with name '{expected_name}'"),
            }
        }
    }

    // -----------------------------------------------------------------------
    // Integration tests — require admin privileges to start an ETW session.
    //
    // Run with:
    //   cargo test -p opentelemetry-etw-logs --all-features -- --ignored --test-threads=1
    //
    // These tests emit logs via the public OTel Logger API
    // (SdkLoggerProvider → Logger → Processor → ETW) and capture the
    // resulting ETW events with ferrisetw for payload validation.
    // -----------------------------------------------------------------------

    /// Full test: validates PartA, PartB, and PartC with multiple attribute types.
    #[ignore = "Requires admin privileges to start ETW trace session"]
    #[test]
    fn integration_test_full() {
        let provider_name = "OTelETWLogsIntTest_Full";

        let (trace, rx) = start_etw_trace(provider_name);

        let etw_processor = crate::Processor::builder(provider_name)
            .with_resource_attributes(["resource_attribute1", "resource_attribute2"])
            .build()
            .unwrap();

        let logger_provider = SdkLoggerProvider::builder()
            .with_resource(
                Resource::builder()
                    .with_service_name("my_test_service")
                    .with_attribute(KeyValue::new("service.instance.id", "test_instance_1"))
                    .with_attribute(KeyValue::new("resource_attribute1", "v1"))
                    .with_attribute(KeyValue::new("resource_attribute2", "v2"))
                    .with_attribute(KeyValue::new("resource_attribute3", "v3"))
                    .build(),
            )
            .with_log_processor(etw_processor)
            .build();

        std::thread::sleep(Duration::from_millis(100));

        let logger = logger_provider.logger("test");
        let mut record = logger.create_log_record();
        record.set_severity_number(opentelemetry::logs::Severity::Error);
        record.set_severity_text("ERROR");
        record.set_body("This is a test message".into());
        record.add_attribute("event_id", AnyValue::Int(20));
        record.add_attribute("user_name", AnyValue::String("otel user".into()));
        record.add_attribute("bool_field", AnyValue::Boolean(true));
        record.add_attribute("int_field", AnyValue::Int(42));
        record.add_attribute("double_field", AnyValue::Double(1.5));
        logger.emit(record);

        let evt = recv_event(&rx, "Log");

        // Event metadata
        assert_eq!(evt.level, tld::Level::Error.as_int());
        assert_eq!(evt.keyword, 1);

        // __csver__
        assert_eq!(evt.get_u16("__csver__"), Some(1024));

        // PartA
        assert_eq!(evt.get_str("PartA.ext_cloud.role"), Some("my_test_service"));
        assert_eq!(
            evt.get_str("PartA.ext_cloud.roleInstance"),
            Some("test_instance_1")
        );
        assert!(evt.has("PartA.time"));

        // PartC — resource attributes (opted-in only)
        assert_eq!(evt.get_str("PartC.resource_attribute1"), Some("v1"));
        assert_eq!(evt.get_str("PartC.resource_attribute2"), Some("v2"));
        assert!(!evt.has("PartC.resource_attribute3")); // not opted in

        // PartC — log record attributes (order-independent)
        assert_eq!(evt.get_str("PartC.user_name"), Some("otel user"));
        assert_eq!(evt.get_bool("PartC.bool_field"), Some(true));
        assert_eq!(evt.get_i64("PartC.int_field"), Some(42));
        assert_eq!(evt.get_f64("PartC.double_field"), Some(1.5));

        // PartB
        assert_eq!(evt.get_str("PartB._typeName"), Some("Log"));
        assert_eq!(evt.get_str("PartB.body"), Some("This is a test message"));
        assert_eq!(
            evt.get_i16("PartB.severityNumber"),
            Some(opentelemetry::logs::Severity::Error as i16)
        );
        assert_eq!(evt.get_str("PartB.severityText"), Some("ERROR"));
        assert_eq!(evt.get_i64("PartB.eventId"), Some(20));

        trace.stop().unwrap();
        let _ = logger_provider.shutdown();
    }

    /// Minimal test: only service.name, body, and severity — no attributes.
    #[ignore = "Requires admin privileges to start ETW trace session"]
    #[test]
    fn integration_test_minimal() {
        let provider_name = "OTelETWLogsIntTest_Minimal";

        let (trace, rx) = start_etw_trace(provider_name);

        let etw_processor = crate::Processor::builder(provider_name).build().unwrap();

        let logger_provider = SdkLoggerProvider::builder()
            .with_resource(
                Resource::builder()
                    .with_service_name("minimal_service")
                    .build(),
            )
            .with_log_processor(etw_processor)
            .build();

        std::thread::sleep(Duration::from_millis(100));

        let logger = logger_provider.logger("test");
        let mut record = logger.create_log_record();
        record.set_severity_number(opentelemetry::logs::Severity::Warn);
        record.set_severity_text("WARN");
        record.set_body("warn message".into());
        logger.emit(record);

        let evt = recv_event(&rx, "Log");
        assert_eq!(evt.level, tld::Level::Warning.as_int());

        // __csver__
        assert_eq!(evt.get_u16("__csver__"), Some(1024));

        // PartA: only role (no roleInstance)
        assert_eq!(evt.get_str("PartA.ext_cloud.role"), Some("minimal_service"));
        assert!(!evt.has("PartA.ext_cloud.roleInstance"));
        assert!(evt.has("PartA.time"));

        // No PartC

        // PartB
        assert_eq!(evt.get_str("PartB._typeName"), Some("Log"));
        assert_eq!(evt.get_str("PartB.body"), Some("warn message"));
        assert_eq!(
            evt.get_i16("PartB.severityNumber"),
            Some(opentelemetry::logs::Severity::Warn as i16)
        );
        assert_eq!(evt.get_str("PartB.severityText"), Some("WARN"));

        trace.stop().unwrap();
        let _ = logger_provider.shutdown();
    }

    /// No-resource test: empty resource — validates no role/roleInstance in PartA.
    #[ignore = "Requires admin privileges to start ETW trace session"]
    #[test]
    fn integration_test_no_resource() {
        let provider_name = "OTelETWLogsIntTest_NoResource";

        let (trace, rx) = start_etw_trace(provider_name);

        let etw_processor = crate::Processor::builder(provider_name).build().unwrap();

        let logger_provider = SdkLoggerProvider::builder()
            .with_resource(Resource::builder_empty().build())
            .with_log_processor(etw_processor)
            .build();

        std::thread::sleep(Duration::from_millis(100));

        let logger = logger_provider.logger("test");
        let mut record = logger.create_log_record();
        record.set_severity_number(opentelemetry::logs::Severity::Error);
        record.set_severity_text("ERROR");
        record.set_body("error without resource".into());
        logger.emit(record);

        let evt = recv_event(&rx, "Log");
        assert_eq!(evt.level, tld::Level::Error.as_int());

        // __csver__
        assert_eq!(evt.get_u16("__csver__"), Some(1024));

        // PartA: no role or roleInstance
        assert!(!evt.has("PartA.ext_cloud.role"));
        assert!(!evt.has("PartA.ext_cloud.roleInstance"));
        assert!(evt.has("PartA.time"));

        // No PartC

        // PartB
        assert_eq!(evt.get_str("PartB._typeName"), Some("Log"));
        assert_eq!(evt.get_str("PartB.body"), Some("error without resource"));
        assert_eq!(
            evt.get_i16("PartB.severityNumber"),
            Some(opentelemetry::logs::Severity::Error as i16)
        );
        assert_eq!(evt.get_str("PartB.severityText"), Some("ERROR"));

        trace.stop().unwrap();
        let _ = logger_provider.shutdown();
    }

    /// Test with trace context — validates ext_dt.traceId and ext_dt.spanId in PartA.
    #[ignore = "Requires admin privileges to start ETW trace session"]
    #[test]
    fn integration_test_with_trace_context() {
        use opentelemetry::trace::{TraceContextExt, Tracer, TracerProvider};
        use opentelemetry_sdk::trace::{Sampler, SdkTracerProvider};

        let provider_name = "OTelETWLogsIntTest_TraceCtx";

        let (trace, rx) = start_etw_trace(provider_name);

        // Set up both trace and log providers
        let tracer_provider = SdkTracerProvider::builder()
            .with_sampler(Sampler::AlwaysOn)
            .build();
        let tracer = tracer_provider.tracer("test-tracer");

        let etw_processor = crate::Processor::builder(provider_name).build().unwrap();

        let logger_provider = SdkLoggerProvider::builder()
            .with_resource(
                Resource::builder()
                    .with_service_name("trace_ctx_service")
                    .build(),
            )
            .with_log_processor(etw_processor)
            .build();

        std::thread::sleep(Duration::from_millis(100));

        // Emit a log inside a span context so trace_context is populated
        let logger = logger_provider.logger("test");
        let (trace_id_expected, span_id_expected) = tracer.in_span("test-span", |cx| {
            let trace_id = cx.span().span_context().trace_id();
            let span_id = cx.span().span_context().span_id();

            let mut record = logger.create_log_record();
            record.set_severity_number(opentelemetry::logs::Severity::Info);
            record.set_severity_text("INFO");
            record.set_body("log inside span".into());
            logger.emit(record);

            (trace_id, span_id)
        });

        let evt = recv_event(&rx, "Log");
        assert_eq!(evt.level, tld::Level::Informational.as_int());

        // __csver__
        assert_eq!(evt.get_u16("__csver__"), Some(1024));

        // PartA: ext_dt should contain traceId and spanId
        assert_eq!(
            evt.get_str("PartA.ext_dt.traceId"),
            Some(trace_id_expected.to_string().as_str())
        );
        assert_eq!(
            evt.get_str("PartA.ext_dt.spanId"),
            Some(span_id_expected.to_string().as_str())
        );

        // PartA: ext_cloud.role should be present
        assert_eq!(
            evt.get_str("PartA.ext_cloud.role"),
            Some("trace_ctx_service")
        );
        assert!(evt.has("PartA.time"));

        // PartB
        assert_eq!(evt.get_str("PartB._typeName"), Some("Log"));
        assert_eq!(evt.get_str("PartB.body"), Some("log inside span"));
        assert_eq!(
            evt.get_i16("PartB.severityNumber"),
            Some(opentelemetry::logs::Severity::Info as i16)
        );
        assert_eq!(evt.get_str("PartB.severityText"), Some("INFO"));

        trace.stop().unwrap();
        let _ = logger_provider.shutdown();
    }

    /// Test with event_name — validates the name field in PartB.
    #[ignore = "Requires admin privileges to start ETW trace session"]
    #[test]
    fn integration_test_with_event_name() {
        let provider_name = "OTelETWLogsIntTest_EvtName";

        let (trace, rx) = start_etw_trace(provider_name);

        let etw_processor = crate::Processor::builder(provider_name).build().unwrap();

        let logger_provider = SdkLoggerProvider::builder()
            .with_resource(Resource::builder_empty().build())
            .with_log_processor(etw_processor)
            .build();

        std::thread::sleep(Duration::from_millis(100));

        let logger = logger_provider.logger("test");
        let mut record = logger.create_log_record();
        record.set_severity_number(opentelemetry::logs::Severity::Warn);
        record.set_severity_text("WARN");
        record.set_body("event with name".into());
        record.set_event_name("my_custom_event");
        logger.emit(record);

        let evt = recv_event(&rx, "Log");
        assert_eq!(evt.level, tld::Level::Warning.as_int());

        // PartB should include name field
        assert_eq!(evt.get_str("PartB._typeName"), Some("Log"));
        assert_eq!(evt.get_str("PartB.body"), Some("event with name"));
        assert_eq!(evt.get_str("PartB.name"), Some("my_custom_event"));
        assert_eq!(
            evt.get_i16("PartB.severityNumber"),
            Some(opentelemetry::logs::Severity::Warn as i16)
        );
        assert_eq!(evt.get_str("PartB.severityText"), Some("WARN"));

        trace.stop().unwrap();
        let _ = logger_provider.shutdown();
    }
}
