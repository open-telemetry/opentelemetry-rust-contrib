//! # OpenTelemetry ETW Traces Exporter
//!
//! Exports OpenTelemetry span data to [Event Tracing for Windows (ETW)][etw]
//! using the [TraceLogging Dynamic][tld] format with
//! [Microsoft Common Schema v4.0][cs] encoding.
//!
//! This crate provides a [`SpanProcessor`] implementation that writes
//! completed spans directly to ETW, suitable for integration with the
//! OpenTelemetry SDK's [`SdkTracerProvider`].
//!
//! ## Usage
//!
//! ```no_run
//! use opentelemetry_etw_traces::Processor;
//! use opentelemetry_sdk::trace::SdkTracerProvider;
//!
//! let processor = Processor::builder("MyAppTracing")
//!     .with_event_name("MyAppEventName") // If not provided, defaults to "Span"
//!     .with_resource_attributes(vec!["custom_attribute1", "custom_attribute2"]) // Only specified resource attributes will be promoted as Part C fields, others will be ignored.
//!     .build()
//!     .expect("Failed to create ETW processor");
//!
//! let provider = SdkTracerProvider::builder()
//!     .with_span_processor(processor)
//!     .build();
//! ```
//!
//! ## Common Schema Mapping
//!
//! Each span is written as an ETW event following
//! the TraceLoggingDynamic format with the following structure:
//!
//! - **`__csver__`**: Common Schema version (`u16`, value `1024` / `0x0400`)
//! - **Part A** (envelope): `time`, `ext_dt { traceId, spanId }`, `ext_cloud { cloud.role, cloud.roleInstance }`
//! - **Part B** (payload): `_typeName="Span"`, `name`, `kind`, `startTime`,
//!   `parentId`, `links`, `statusMessage`, `success`
//! - **Part C** (extensions): promoted resource attributes, span's attributes
//!
//! ## Platform Support
//!
//! This crate is Windows-only, as ETW is a Windows-specific tracing facility.
//!
//! [etw]: https://learn.microsoft.com/en-us/windows/win32/etw/about-event-tracing
//! [tld]: https://crates.io/crates/tracelogging_dynamic
//! [cs]: https://learn.microsoft.com/en-us/opentelemetry/common-schema
//! [`SpanProcessor`]: opentelemetry_sdk::trace::SpanProcessor
//! [`SdkTracerProvider`]: opentelemetry_sdk::trace::SdkTracerProvider

#![warn(missing_debug_implementations, missing_docs)]

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

    use opentelemetry::trace::{TraceContextExt, Tracer, TracerProvider};
    use opentelemetry::KeyValue;
    use opentelemetry_sdk::trace::SdkTracerProvider;
    use opentelemetry_sdk::Resource;

    // -----------------------------------------------------------------------
    // TDH-based ETW event property parser
    //
    // ferrisetw's Parser cannot handle TraceLogging struct properties (PartA,
    // PartB, ext_dt, etc.) — it returns UnimplementedType("structure").
    // We use TDH APIs directly to enumerate all event properties from the
    // schema (TRACE_EVENT_INFO) and read their values from the raw data
    // buffer, producing a flat HashMap keyed by dotted paths like
    // "PartA.ext_cloud.role" or "PartB.name".
    // -----------------------------------------------------------------------

    /// Captured event with all properties parsed into named fields.
    struct CapturedEvent {
        event_name: String,
        keyword: u64,
        /// Properties keyed by dotted path: "PartA.ext_cloud.role", "PartB.name", etc.
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

        fn get_u8(&self, key: &str) -> Option<u8> {
            self.properties.get(key).and_then(|v| v.first().copied())
        }

        fn get_i64(&self, key: &str) -> Option<i64> {
            self.properties
                .get(key)
                .filter(|v| v.len() >= 8)
                .map(|v| i64::from_le_bytes(v[..8].try_into().unwrap()))
        }

        fn get_bool(&self, key: &str) -> Option<bool> {
            // Accepts either 1-byte (u8 with OutType::Boolean) or 4-byte
            // (Bool32) representations — TLD allows both.
            self.properties.get(key).map(|v| match v.len() {
                1 => v[0] != 0,
                4 => i32::from_le_bytes(v[..4].try_into().unwrap()) != 0,
                _ => panic!("Unexpected boolean width: {} bytes for key {key}", v.len()),
            })
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
            17 => (data[pos..pos + 8].to_vec(), 8),    // FILETIME
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
    //   cargo test -p opentelemetry-etw-traces --all-features -- --ignored --test-threads=1
    //
    // These tests emit spans via the public OTel Tracer API
    // (SdkTracerProvider → Tracer → Processor → ETW) and capture the
    // resulting ETW events with ferrisetw for payload validation.
    // -----------------------------------------------------------------------

    /// Full test: validates PartA, PartB (incl. well-known attributes), and PartC.
    #[ignore = "Requires admin privileges to start ETW trace session"]
    #[test]
    fn integration_test_full() {
        let provider_name = "OTelETWTracesIntTest_Full";

        let (trace, rx) = start_etw_trace(provider_name);

        let processor = crate::Processor::builder(provider_name)
            .with_resource_attributes(["resource_attribute1", "resource_attribute2"])
            .build()
            .unwrap();

        let tracer_provider = SdkTracerProvider::builder()
            .with_resource(
                Resource::builder()
                    .with_service_name("my_test_service")
                    .with_attribute(KeyValue::new("service.instance.id", "test_instance_1"))
                    .with_attribute(KeyValue::new("resource_attribute1", "v1"))
                    .with_attribute(KeyValue::new("resource_attribute2", "v2"))
                    .with_attribute(KeyValue::new("resource_attribute3", "v3"))
                    .build(),
            )
            .with_span_processor(processor)
            .build();

        std::thread::sleep(Duration::from_millis(100));

        let tracer = tracer_provider.tracer("test");
        tracer.in_span("my-span", |cx| {
            let span = cx.span();
            span.set_attribute(KeyValue::new("custom_attr", "custom_value"));
            span.set_attribute(KeyValue::new("int_attr", 42_i64));
            span.set_attribute(KeyValue::new("bool_attr", true));
            // Well-known attribute — should land in PartB, not PartC.
            span.set_attribute(KeyValue::new("http.request.method", "GET"));
        });

        let evt = recv_event(&rx, "Span");

        // Event metadata
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
        assert!(evt.has("PartA.ext_dt.traceId"));
        assert!(evt.has("PartA.ext_dt.spanId"));

        // PartB — base fields
        assert_eq!(evt.get_str("PartB._typeName"), Some("Span"));
        assert_eq!(evt.get_str("PartB.name"), Some("my-span"));
        assert_eq!(evt.get_u8("PartB.kind"), Some(0)); // Internal
        assert!(evt.has("PartB.startTime"));
        // Root span — no parentId.
        assert!(!evt.has("PartB.parentId"));
        // No links, no statusMessage in this test.
        assert!(!evt.has("PartB.links"));
        assert!(!evt.has("PartB.statusMessage"));
        // Well-known PartB attribute (promoted from span attributes).
        assert_eq!(evt.get_str("PartB.httpMethod"), Some("GET"));
        // success = true (status is Unset).
        assert_eq!(evt.get_bool("PartB.success"), Some(true));

        // PartC — opted-in resource attributes.
        assert_eq!(evt.get_str("PartC.resource_attribute1"), Some("v1"));
        assert_eq!(evt.get_str("PartC.resource_attribute2"), Some("v2"));
        assert!(!evt.has("PartC.resource_attribute3")); // not opted in

        // PartC — non-well-known span attributes.
        assert_eq!(evt.get_str("PartC.custom_attr"), Some("custom_value"));
        assert_eq!(evt.get_i64("PartC.int_attr"), Some(42));
        assert_eq!(evt.get_bool("PartC.bool_attr"), Some(true));
        // Well-known attribute must NOT be duplicated in PartC.
        assert!(!evt.has("PartC.http.request.method"));

        trace.stop().unwrap();
        let _ = tracer_provider.shutdown();
    }
}
