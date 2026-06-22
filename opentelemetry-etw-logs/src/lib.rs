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
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc;
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::Duration;

    use one_collect::etw::tdh::TdhDecoder;
    use one_collect::etw::{EtwSession, LEVEL_VERBOSE};
    use one_collect::event::os::windows::WindowsEventExtension;
    use one_collect::event::Event;
    use one_collect::Guid;

    use tracelogging_dynamic as tld;

    use opentelemetry::logs::{AnyValue, LogRecord, Logger, LoggerProvider};
    use opentelemetry::KeyValue;
    use opentelemetry_sdk::logs::SdkLoggerProvider;
    use opentelemetry_sdk::Resource;

    // -----------------------------------------------------------------------
    // ETW event property capture via one_collect
    //
    // one_collect's TdhDecoder resolves TraceLogging struct properties (PartA,
    // PartB, ext_dt, etc.) into a flat EventFormat whose nested fields are
    // flattened with dotted names ("PartA.ext_cloud.role", "PartB.body").
    // We walk that schema and read each leaf value from the event payload,
    // producing a flat HashMap keyed by those dotted paths.
    // -----------------------------------------------------------------------

    /// Delay after the worker signals readiness to give `parse_until` time to call
    /// `StartTrace` + `EnableTraceEx2` in the kernel.  The channel signal only means
    /// the worker is *about to* call `parse_until` — the actual kernel setup still
    /// takes an indeterminate amount of time, so we sleep here to cover cold-start
    /// overhead on CI runners where that can be several hundred milliseconds.
    const ETW_SESSION_START_DELAY: Duration = Duration::from_millis(1000);

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

    /// Build a flat property map from a decoded TraceLogging event.
    ///
    /// one_collect's `TdhDecoder` produces an `EventFormat` whose nested
    /// struct fields are already flattened with dotted names. We resolve
    /// each leaf field's bytes (handling variable-length skip chains via
    /// `try_get_field_data_closure`) into a `HashMap` keyed by dotted path.
    fn build_properties(
        decoded: &one_collect::etw::tdh::TdhDecodedEvent<'_>,
    ) -> HashMap<String, Vec<u8>> {
        let format = decoded.event_data.format();
        let payload = decoded.event_data.event_data();

        let mut result = HashMap::new();
        for field in format.fields() {
            if let Some(mut get_data) = format.try_get_field_data_closure(&field.name) {
                result.insert(field.name.clone(), get_data(payload).to_vec());
            }
        }
        result
    }

    /// Handle to a running ETW capture session backed by one_collect.
    ///
    /// `one_collect::etw::EtwSession::parse_until` blocks the calling thread
    /// and runs event callbacks on it, so the session is driven on a
    /// dedicated worker thread. Dropping (or explicitly `stop`-ping) the
    /// handle signals that thread to stop and joins it.
    struct EtwTrace {
        stop: Arc<AtomicBool>,
        worker: Mutex<Option<thread::JoinHandle<()>>>,
    }

    impl EtwTrace {
        fn stop(&self) -> thread::Result<()> {
            self.stop.store(true, Ordering::Relaxed);
            match self.worker.lock().unwrap().take() {
                Some(worker) => worker.join(),
                None => Ok(()),
            }
        }
    }

    impl Drop for EtwTrace {
        fn drop(&mut self) {
            let _ = self.stop();
        }
    }

    /// Start a one_collect ETW capture session for the given provider name.
    ///
    /// This function blocks until the worker thread has signalled that it is
    /// **about to** call `parse_until`, then sleeps `ETW_SESSION_START_DELAY`
    /// (1 s) to give `parse_until` time to call `StartTrace` / `EnableTraceEx2`
    /// in the kernel.  The channel signal alone does *not* mean the ETW session
    /// is ready — events emitted before `parse_until` finishes kernel setup are
    /// silently dropped.  Without this synchronisation the first integration
    /// test (cold-start) races with the ETW session setup and times out.
    fn start_etw_trace(provider_name: &str) -> (EtwTrace, mpsc::Receiver<CapturedEvent>) {
        // TraceLogging derives the provider GUID from the provider name; mirror
        // that here so we subscribe to the same GUID the exporter registers.
        let guid = Guid::from_u128(tld::Guid::from_name(provider_name).to_u128());

        let (tx, rx) = mpsc::sync_channel::<CapturedEvent>(16);
        let stop = Arc::new(AtomicBool::new(false));
        let stop_worker = stop.clone();
        let session_name = format!("{provider_name}_session");

        // One-shot channel: the worker sends () just before calling
        // `parse_until` so the main thread can wait for it.
        let (session_ready_tx, session_ready_rx) = mpsc::sync_channel::<()>(1);

        let worker = thread::spawn(move || {
            let mut session = EtwSession::new();
            let ancillary = session.ancillary_data();
            let decoder = Rc::new(RefCell::new(TdhDecoder::new()));

            // Wide event: capture every event from the provider (any ID), at
            // verbose level with all keywords enabled.
            let mut event = Event::for_etw(0, "Event".to_string(), guid, LEVEL_VERBOSE, u64::MAX);
            event.set_id_wild_card_flag();

            event.add_callback(move |_data| {
                let ancillary = ancillary.borrow();
                let record = match ancillary.record() {
                    Some(record) => record,
                    None => return Ok(()),
                };

                let mut decoder = decoder.borrow_mut();
                if let Ok(decoded) = decoder.decode(record) {
                    let descriptor = record.EventHeader.EventDescriptor;
                    let captured = CapturedEvent {
                        event_name: decoded.event_name.unwrap_or_default().to_string(),
                        level: descriptor.Level,
                        keyword: descriptor.Keyword,
                        properties: build_properties(&decoded),
                    };
                    let _ = tx.try_send(captured);
                }

                Ok(())
            });

            session.add_event(event, None);

            // Signal the main thread that one_collect event setup is complete
            // and we are *about to* call parse_until (which will call
            // StartTrace / EnableTraceEx2 in the kernel).  The main thread
            // will sleep after receiving this signal to give the kernel time
            // to complete session setup.
            let _ = session_ready_tx.send(());

            let _ = session.parse_until(&session_name, move || stop_worker.load(Ordering::Relaxed));
        });

        // The channel signal means the worker is about to call parse_until —
        // not that the ETW session is fully active.  Sleep ETW_SESSION_START_DELAY
        // to give parse_until time to call StartTrace + EnableTraceEx2 in the
        // kernel before we return and the caller starts emitting events.
        session_ready_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("ETW worker thread failed to signal readiness within 5 seconds");
        std::thread::sleep(ETW_SESSION_START_DELAY);

        (
            EtwTrace {
                stop,
                worker: Mutex::new(Some(worker)),
            },
            rx,
        )
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
    // resulting ETW events with one_collect for payload validation.
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
