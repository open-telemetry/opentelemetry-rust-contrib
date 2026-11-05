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
//! - **Part A** (envelope): `time`, `ext_dt { traceId, spanId }`, `ext_cloud { role, roleInstance }`
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

#[cfg(feature = "bench-internals")]
#[doc(hidden)]
pub use exporter::benchmark;

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

    use opentelemetry::trace::{TraceContextExt, Tracer, TracerProvider};
    use opentelemetry::KeyValue;
    use opentelemetry_sdk::trace::SdkTracerProvider;
    use opentelemetry_sdk::Resource;

    // -----------------------------------------------------------------------
    // ETW event property capture via one_collect
    //
    // one_collect's TdhDecoder resolves TraceLogging struct properties (PartA,
    // PartB, ext_dt, etc.) into a flat EventFormat whose nested fields are
    // flattened with dotted names ("PartA.ext_cloud.role", "PartB.name").
    // We walk that schema and read each leaf value from the event payload,
    // producing a flat HashMap keyed by those dotted paths.
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
    fn start_etw_trace(provider_name: &str) -> (EtwTrace, mpsc::Receiver<CapturedEvent>) {
        // TraceLogging derives the provider GUID from the provider name; mirror
        // that here so we subscribe to the same GUID the exporter registers.
        let guid = Guid::from_u128(tld::Guid::from_name(provider_name).to_u128());

        let (tx, rx) = mpsc::sync_channel::<CapturedEvent>(16);
        let stop = Arc::new(AtomicBool::new(false));
        let stop_worker = stop.clone();
        let session_name = format!("{provider_name}_session");

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
                        keyword: descriptor.Keyword,
                        properties: build_properties(&decoded),
                    };
                    let _ = tx.try_send(captured);
                }

                Ok(())
            });

            session.add_event(event, None);

            let _ = session.parse_until(&session_name, move || stop_worker.load(Ordering::Relaxed));
        });

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
    //   cargo test -p opentelemetry-etw-traces --all-features -- --ignored --test-threads=1
    //
    // These tests emit spans via the public OTel Tracer API
    // (SdkTracerProvider → Tracer → Processor → ETW) and capture the
    // resulting ETW events with one_collect for payload validation.
    // -----------------------------------------------------------------------

    #[cfg(feature = "bench-internals")]
    #[ignore = "Requires admin privileges to start ETW trace session"]
    #[test]
    fn integration_test_serialization_matches_export() {
        assert_serialization_matches_export(false);
    }

    #[cfg(all(feature = "bench-internals", feature = "experimental-geneva-mapping"))]
    #[ignore = "Requires admin privileges to start ETW trace session"]
    #[test]
    fn integration_test_geneva_serialization_matches_export() {
        assert_serialization_matches_export(true);
    }

    #[cfg(feature = "bench-internals")]
    fn assert_serialization_matches_export(geneva: bool) {
        let provider_name = if geneva {
            "OTelETWTracesIntTest_GenevaSerialization"
        } else {
            "OTelETWTracesIntTest_Serialization"
        };
        let part_b = if geneva { "" } else { "PartB." };
        let part_c = if geneva { "" } else { "PartC." };
        let (trace, rx) = start_etw_trace(provider_name);
        let resource = Resource::builder_empty()
            .with_service_name("serialization-test")
            .build();
        let exporter = crate::benchmark::ExportBenchmark::new(provider_name, &resource);
        #[cfg(feature = "experimental-geneva-mapping")]
        let exporter = exporter.with_geneva_mapping(geneva);
        let provider = Box::pin(tld::Provider::new(provider_name, &tld::Provider::options()));
        // SAFETY: the provider remains pinned until drop unregisters the callback.
        assert_eq!(unsafe { provider.as_ref().register() }, 0);
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !exporter.enabled() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(exporter.enabled());

        let mut event = tld::EventBuilder::new();
        // Alternate large and small payloads to exercise reset of reused buffers.
        for count in [6, 0, 3] {
            let attributes = (0..count)
                .map(|i| KeyValue::new(format!("key{i}"), format!("value{i}")))
                .collect();
            let mut span =
                crate::exporter::common::test_utils::create_test_span_data(Some(attributes));
            span.start_time = std::time::UNIX_EPOCH + Duration::new(10, 123_456_000);
            span.end_time = std::time::UNIX_EPOCH + Duration::new(20, 987_654_300);
            exporter.serialize(&mut event, &span);
            assert_eq!(event.write(&provider, None, None), 0);
            let serialized = recv_event(&rx, "Span");
            exporter.export(&span);
            let exported = recv_event(&rx, "Span");

            assert_eq!(serialized.keyword, exported.keyword);
            assert_eq!(serialized.properties, exported.properties);
            assert_eq!(
                exported.get_str(&format!("{part_b}name")),
                Some("test-span")
            );
            assert_eq!(
                exported.get_str(if geneva { "env_time" } else { "PartA.time" }),
                Some("1970-01-01T00:00:20.987654300Z")
            );
            assert_eq!(
                exported.get_str(&format!("{part_b}startTime")),
                Some("1970-01-01T00:00:10.123456Z")
            );
            assert_eq!(
                exported.get_str(if geneva {
                    "env_dt_traceId"
                } else {
                    "PartA.ext_dt.traceId"
                }),
                Some(span.span_context.trace_id().to_string().as_str())
            );
            assert_eq!(
                exported.get_str(if geneva {
                    "env_dt_spanId"
                } else {
                    "PartA.ext_dt.spanId"
                }),
                Some(span.span_context.span_id().to_string().as_str())
            );
            assert_eq!(
                exported.get_str(if geneva {
                    "env_cloud_role"
                } else {
                    "PartA.ext_cloud.role"
                }),
                Some("serialization-test")
            );
            for i in 0..6 {
                let key = format!("{part_c}key{i}");
                let value = format!("value{i}");
                assert_eq!(
                    exported.get_str(&key),
                    (i < count).then_some(value.as_str())
                );
            }
        }

        let mut span = crate::exporter::common::test_utils::create_test_span_data(Some(vec![
            KeyValue::new("http.request.method", "GET"),
            KeyValue::new("custom", "custom-value"),
            KeyValue::new("db.system.name", "postgresql"),
            KeyValue::new("http.request.method", "POST"),
        ]));
        span.parent_span_id = opentelemetry::trace::SpanId::from_bytes([0x0a; 8]);
        for description in ["failure \u{2603}", ""] {
            for description in [
                std::borrow::Cow::Borrowed(description),
                std::borrow::Cow::Owned(description.to_string()),
            ] {
                span.status = opentelemetry::trace::Status::Error {
                    description: description.clone(),
                };
                exporter.export(&span);
                let exported = recv_event(&rx, "Span");
                assert_eq!(
                    exported.get_str(&format!("{part_b}parentId")),
                    Some("0a0a0a0a0a0a0a0a")
                );
                assert_eq!(
                    exported.get_str(&format!("{part_b}httpMethod")),
                    Some("GET")
                );
                assert_eq!(
                    exported.get_str(&format!("{part_b}dbSystem")),
                    Some("postgresql")
                );
                assert_eq!(
                    exported.get_str(&format!("{part_c}custom")),
                    Some("custom-value")
                );
                assert!(!exported.has(&format!("{part_c}http.request.method")));
                assert_eq!(exported.get_bool(&format!("{part_b}success")), Some(false));
                assert_eq!(
                    exported.get_str(&format!("{part_b}statusMessage")),
                    (!description.is_empty()).then_some(description.as_ref())
                );
            }
        }
        trace.stop().unwrap();
    }

    /// Full test: validates PartA, PartB (incl. well-known attributes), and PartC.
    #[ignore = "Requires admin privileges to start ETW trace session"]
    #[test]
    fn integration_test_full() {
        assert_full_event(false);
    }

    #[cfg(feature = "experimental-geneva-mapping")]
    #[ignore = "Requires admin privileges to start ETW trace session"]
    #[test]
    fn integration_test_geneva_mapping() {
        assert_full_event(true);
    }

    fn assert_full_event(geneva: bool) {
        let provider_name = if geneva {
            "OTelETWTracesIntTest_Geneva"
        } else {
            "OTelETWTracesIntTest_Full"
        };
        let part_b = if geneva { "" } else { "PartB." };
        let part_c = if geneva { "" } else { "PartC." };

        let (trace, rx) = start_etw_trace(provider_name);

        let builder = crate::Processor::builder(provider_name)
            .with_resource_attributes(["resource_attribute1", "resource_attribute2"]);
        #[cfg(feature = "experimental-geneva-mapping")]
        let builder = builder.with_geneva_mapping(geneva);
        let processor = builder.build().unwrap();

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

        let probe = Box::pin(tld::Provider::new(provider_name, &tld::Provider::options()));
        // SAFETY: the probe remains pinned until drop unregisters the callback.
        assert_eq!(unsafe { probe.as_ref().register() }, 0);
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !probe.enabled(tld::Level::Verbose, 1) && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(probe.enabled(tld::Level::Verbose, 1));

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
        if geneva {
            assert_eq!(evt.get_str("env_ver"), Some("4.0"));
            assert_eq!(evt.get_str("env_name"), Some("Span"));
            assert!(!evt.has("__csver__"));
            assert!(!evt.has("_typeName"));
            assert!(!evt.properties.keys().any(|key| key.starts_with("Part")));
        } else {
            assert_eq!(evt.get_u16("__csver__"), Some(1024));
            assert_eq!(evt.get_str("PartB._typeName"), Some("Span"));
            assert!(!evt.properties.keys().any(|key| key.starts_with("env_")));
        }

        // PartA
        assert_eq!(
            evt.get_str(if geneva {
                "env_cloud_role"
            } else {
                "PartA.ext_cloud.role"
            }),
            Some("my_test_service")
        );
        assert_eq!(
            evt.get_str(if geneva {
                "env_cloud_roleInstance"
            } else {
                "PartA.ext_cloud.roleInstance"
            }),
            Some("test_instance_1")
        );
        assert!(evt.has(if geneva { "env_time" } else { "PartA.time" }));
        assert!(evt.has(if geneva {
            "env_dt_traceId"
        } else {
            "PartA.ext_dt.traceId"
        }));
        assert!(evt.has(if geneva {
            "env_dt_spanId"
        } else {
            "PartA.ext_dt.spanId"
        }));

        // PartB — base fields
        assert_eq!(evt.get_str(&format!("{part_b}name")), Some("my-span"));
        assert_eq!(evt.get_u8(&format!("{part_b}kind")), Some(0)); // Internal
        assert!(evt.has(&format!("{part_b}startTime")));
        // Root span — no parentId.
        assert!(!evt.has(&format!("{part_b}parentId")));
        // No links, no statusMessage in this test.
        assert!(!evt.has(&format!("{part_b}links")));
        assert!(!evt.has(&format!("{part_b}statusMessage")));
        // Well-known PartB attribute (promoted from span attributes).
        assert_eq!(evt.get_str(&format!("{part_b}httpMethod")), Some("GET"));
        // success = true (status is Unset).
        assert_eq!(evt.get_bool(&format!("{part_b}success")), Some(true));

        // PartC — opted-in resource attributes.
        assert_eq!(
            evt.get_str(&format!("{part_c}resource_attribute1")),
            Some("v1")
        );
        assert_eq!(
            evt.get_str(&format!("{part_c}resource_attribute2")),
            Some("v2")
        );
        assert!(!evt.has(&format!("{part_c}resource_attribute3"))); // not opted in

        // PartC — non-well-known span attributes.
        assert_eq!(
            evt.get_str(&format!("{part_c}custom_attr")),
            Some("custom_value")
        );
        assert_eq!(evt.get_i64(&format!("{part_c}int_attr")), Some(42));
        assert_eq!(evt.get_bool(&format!("{part_c}bool_attr")), Some(true));
        // Well-known attribute must NOT be duplicated in PartC.
        assert!(!evt.has(&format!("{part_c}http.request.method")));

        trace.stop().unwrap();
        let _ = tracer_provider.shutdown();
    }
}
