// Copyright The OpenTelemetry Authors
// SPDX-License-Identifier: Apache-2.0

//! [`GenevaLogRecord`] adapter for any type implementing
//! `otap_df_pdata_views::views::logs::LogRecordView`.
//!
//! Wrap a `&T` in [`LogRecordViewAdapter`] to feed view-backed telemetry into
//! the Geneva Bond encoding pipeline:
//!
//! ```ignore
//! let adapter = LogRecordViewAdapter(&my_log_record_view);
//! accumulator.push(&adapter, &metadata_fields);
//! ```

use otap_df_pdata_views::views::{
    common::{AnyValueView, AttributeView, ValueType},
    logs::LogRecordView,
};

use super::log_record::{AttrValue, GenevaLogRecord};

/// Bridges any [`LogRecordView`] implementation to [`GenevaLogRecord`].
///
/// The adapter holds a shared reference to the view item, performing zero-copy
/// access for all scalar fields.  String-typed body and attribute values are
/// converted from `&[u8]` to `&str` via [`std::str::from_utf8`]; invalid UTF-8
/// sequences cause the value to be treated as absent.
pub(crate) struct LogRecordViewAdapter<'a, T>(pub &'a T);

impl<'a, T: LogRecordView> GenevaLogRecord for LogRecordViewAdapter<'a, T> {
    fn routing_event_name(&self) -> &str {
        self.explicit_event_name().unwrap_or("Log")
    }

    fn explicit_event_name(&self) -> Option<&str> {
        self.0
            .event_name()
            .and_then(|b| std::str::from_utf8(b).ok())
            .filter(|s| !s.is_empty())
    }

    fn timestamp_nanos(&self) -> u64 {
        self.0
            .time_unix_nano()
            .filter(|&t| t != 0)
            .or_else(|| self.0.observed_time_unix_nano())
            .unwrap_or(0)
    }

    fn severity_level(&self) -> u8 {
        self.0.severity_number().unwrap_or(0) as u8
    }

    fn trace_id_bytes(&self) -> Option<[u8; 16]> {
        self.0.trace_id().copied()
    }

    fn span_id_bytes(&self) -> Option<[u8; 8]> {
        self.0.span_id().copied()
    }

    fn flags(&self) -> Option<u32> {
        self.0.flags().filter(|&f| f != 0)
    }

    fn severity_number(&self) -> i32 {
        self.0.severity_number().unwrap_or(0)
    }

    fn severity_text(&self) -> Option<&str> {
        self.0
            .severity_text()
            .and_then(|b| std::str::from_utf8(b).ok())
            .filter(|s| !s.is_empty())
    }

    fn has_body_string(&self) -> bool {
        let body = match self.0.body() {
            Some(b) => b,
            None => return false,
        };
        body.value_type() == ValueType::String
            && body
                .as_string()
                .is_some_and(|bytes| std::str::from_utf8(bytes).is_ok())
    }

    fn with_body_string<F: FnMut(&str)>(&self, f: &mut F) -> bool {
        let body = match self.0.body() {
            Some(b) => b,
            None => return false,
        };
        if body.value_type() == ValueType::String {
            if let Some(bytes) = body.as_string() {
                if let Ok(s) = std::str::from_utf8(bytes) {
                    f(s);
                    return true;
                }
            }
        }
        false
    }

    fn visit_attributes<F: FnMut(&str, AttrValue<'_>)>(&self, visitor: &mut F) {
        for attr in self.0.attributes() {
            let key = match std::str::from_utf8(attr.key()) {
                Ok(k) => k,
                Err(_) => continue,
            };
            let val = match attr.value() {
                Some(v) => v,
                None => continue,
            };
            let attr_val = match val.value_type() {
                ValueType::String => {
                    match val.as_string().and_then(|b| std::str::from_utf8(b).ok()) {
                        Some(s) => AttrValue::String(s),
                        None => continue,
                    }
                }
                ValueType::Int64 => match val.as_int64() {
                    Some(i) => AttrValue::Int64(i),
                    None => continue,
                },
                ValueType::Double => match val.as_double() {
                    Some(d) => AttrValue::Double(d),
                    None => continue,
                },
                ValueType::Bool => match val.as_bool() {
                    Some(b) => AttrValue::Bool(b),
                    None => continue,
                },
                _ => continue,
            };
            visitor(key, attr_val);
        }
    }
}
