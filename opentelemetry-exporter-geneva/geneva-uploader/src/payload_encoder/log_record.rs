// Copyright The OpenTelemetry Authors
// SPDX-License-Identifier: Apache-2.0

//! Abstraction layer between log record representations and Geneva Bond encoding.
//!
//! [`GenevaLogRecord`] is the single trait a log record type must implement to
//! flow through the shared Bond/LZ4/CentralBlob pipeline.  Two implementations
//! ship out of the box:
//!
//! - [`&opentelemetry_proto::tonic::logs::v1::LogRecord`] — OTLP proto records.
//! - [`LogRecordViewAdapter`](super::view_log_record::LogRecordViewAdapter) —
//!   any type implementing `otap_df_pdata_views::views::logs::LogRecordView`.
//!
//! Adding support for a new ingestion format means only implementing this trait;
//! no encoder internals need to change.

use opentelemetry_proto::tonic::{common::v1::any_value::Value, logs::v1::LogRecord};

/// Typed attribute value yielded by [`GenevaLogRecord::visit_attributes`].
#[derive(Clone, Copy)]
pub(crate) enum AttrValue<'a> {
    /// UTF-8 string value.
    String(&'a str),
    /// 64-bit signed integer.
    Int64(i64),
    /// 64-bit floating-point.
    Double(f64),
    /// Boolean.
    Bool(bool),
}

/// Adapter trait for Geneva Bond log encoding.
///
/// Abstracts field access from different log record representations so that the
/// Bond/LZ4/CentralBlob encoding pipeline is independent of the concrete record
/// type.  Adding a new source format only requires implementing this trait.
pub(crate) trait GenevaLogRecord {
    /// Name used to route this record into its per-event batch.
    ///
    /// Never empty; returns `"Log"` when the record carries no explicit name.
    fn routing_event_name(&self) -> &str;

    /// Returns the explicit event name to be stored as the `name` Bond field,
    /// or `None` if the record has no event name (field is omitted from schema).
    fn explicit_event_name(&self) -> Option<&str>;

    /// Primary timestamp in nanoseconds since the Unix epoch.
    fn timestamp_nanos(&self) -> u64;

    /// Severity byte written into `CentralEventEntry::level` (0 = unknown).
    fn severity_level(&self) -> u8;

    /// Trace ID as a fixed-size 16-byte array, or `None` if absent.
    fn trace_id_bytes(&self) -> Option<[u8; 16]>;

    /// Span ID as a fixed-size 8-byte array, or `None` if absent.
    fn span_id_bytes(&self) -> Option<[u8; 8]>;

    /// Trace flags, or `None` if absent / zero.
    fn flags(&self) -> Option<u32>;

    /// Numeric severity (OTLP SeverityNumber).
    fn severity_number(&self) -> i32;

    /// Severity text (log level string), or `None` if absent.
    fn severity_text(&self) -> Option<&str>;

    /// Returns `true` if the body is a UTF-8 string.
    ///
    /// Used during schema determination to decide whether to include the `body`
    /// field, avoiding the lifetime complications of returning a reference to a
    /// GAT-backed temporary.
    fn has_body_string(&self) -> bool;

    /// Calls `f` with the body string when the body is a UTF-8 string, and
    /// returns `true` iff `f` was called.
    ///
    /// A callback is used instead of returning `&str` because GAT-backed view
    /// types produce a body temporary that cannot outlive the call.
    fn with_body_string<F: FnMut(&str)>(&self, f: &mut F) -> bool;

    /// Calls `visitor` once for each attribute with a scalar value type
    /// encodable in Geneva Bond (string, i64, f64, bool).  Other value types
    /// are silently skipped.
    ///
    /// # Ordering contract
    ///
    /// Implementations **must** yield attributes in the same order on every
    /// call for a given record.  [`LogBatchAccumulator`] calls this method
    /// twice per record — once during schema discovery
    /// ([`OtlpEncoder::determine_fields_for`]) and once during row encoding
    /// ([`OtlpEncoder::write_row_data_for`]).  If the order differs between
    /// the two calls the Bond schema field list and the row data will be
    /// misaligned, producing corrupt output.
    fn visit_attributes<F: FnMut(&str, AttrValue<'_>)>(&self, visitor: &mut F);
}

// ---------------------------------------------------------------------------
// OTLP proto implementation
// ---------------------------------------------------------------------------

impl GenevaLogRecord for &LogRecord {
    fn routing_event_name(&self) -> &str {
        self.explicit_event_name().unwrap_or("Log")
    }

    fn explicit_event_name(&self) -> Option<&str> {
        if self.event_name.is_empty() {
            None
        } else {
            Some(&self.event_name)
        }
    }

    fn timestamp_nanos(&self) -> u64 {
        if self.time_unix_nano != 0 {
            self.time_unix_nano
        } else {
            self.observed_time_unix_nano
        }
    }

    fn severity_level(&self) -> u8 {
        self.severity_number as u8
    }

    fn trace_id_bytes(&self) -> Option<[u8; 16]> {
        if self.trace_id.is_empty() {
            None
        } else {
            self.trace_id.as_slice().try_into().ok()
        }
    }

    fn span_id_bytes(&self) -> Option<[u8; 8]> {
        if self.span_id.is_empty() {
            None
        } else {
            self.span_id.as_slice().try_into().ok()
        }
    }

    fn flags(&self) -> Option<u32> {
        if self.flags != 0 {
            Some(self.flags)
        } else {
            None
        }
    }

    fn severity_number(&self) -> i32 {
        self.severity_number
    }

    fn severity_text(&self) -> Option<&str> {
        if self.severity_text.is_empty() {
            None
        } else {
            Some(&self.severity_text)
        }
    }

    fn has_body_string(&self) -> bool {
        matches!(
            self.body.as_ref().and_then(|b| b.value.as_ref()),
            Some(Value::StringValue(_))
        )
    }

    fn with_body_string<F: FnMut(&str)>(&self, f: &mut F) -> bool {
        if let Some(body) = &self.body {
            if let Some(Value::StringValue(s)) = &body.value {
                f(s.as_str());
                return true;
            }
        }
        false
    }

    fn visit_attributes<F: FnMut(&str, AttrValue<'_>)>(&self, visitor: &mut F) {
        for kv in &self.attributes {
            if let Some(val) = kv.value.as_ref().and_then(|v| v.value.as_ref()) {
                let attr_val = match val {
                    Value::StringValue(s) => AttrValue::String(s.as_str()),
                    Value::IntValue(i) => AttrValue::Int64(*i),
                    Value::DoubleValue(d) => AttrValue::Double(*d),
                    Value::BoolValue(b) => AttrValue::Bool(*b),
                    _ => continue,
                };
                visitor(&kv.key, attr_val);
            }
        }
    }
}
