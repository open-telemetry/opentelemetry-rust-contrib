// Copyright The OpenTelemetry Authors
// SPDX-License-Identifier: Apache-2.0

use crate::client::EncodedBatch;
use crate::payload_encoder::bond_encoder::{BondDataType, BondEncodedSchema, BondWriter, FieldDef};
use crate::payload_encoder::central_blob::{
    BatchMetadata, CentralBlob, CentralEventEntry, CentralSchemaEntry,
};
use crate::payload_encoder::lz4_chunked_compression::lz4_chunked_compression;
use chrono::{TimeZone, Utc};
use md5::{Digest as _, Md5};
use opentelemetry_proto::tonic::common::v1::any_value::Value;
use opentelemetry_proto::tonic::trace::v1::Span;
use otap_df_pdata_views::views::common::{AnyValueView, AttributeView, ValueType};
use otap_df_pdata_views::views::logs::{
    LogRecordView, LogsDataView, ResourceLogsView, ScopeLogsView,
};
use otap_df_pdata_views::views::resource::ResourceView;
use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, error};

const CS_VERSION_4: i64 = 0x400;
const KEY_CSVER: &str = "__csver__";
const KEY_PARTB_TYPENAME: &str = "PartB._typeName";
const CS_LOG_TYPENAME: &str = "Log";

const FIELD_ENV_NAME: &str = "env_name";
const FIELD_ENV_VER: &str = "env_ver";
const FIELD_TIMESTAMP: &str = "timestamp";
const FIELD_ENV_TIME: &str = "env_time";
const FIELD_TRACE_ID: &str = "env_dt_traceId";
const FIELD_SPAN_ID: &str = "env_dt_spanId";
const FIELD_TRACE_FLAGS: &str = "env_dt_traceFlags";
const FIELD_NAME: &str = "name";
const FIELD_SEVERITY_NUMBER: &str = "SeverityNumber";
const FIELD_SEVERITY_TEXT: &str = "SeverityText";
const FIELD_BODY: &str = "body";

// Tenant/Role/RoleInstance fields
const FIELD_TENANT: &str = "Tenant";
const FIELD_ROLE: &str = "Role";
const FIELD_ROLE_INSTANCE: &str = "RoleInstance";

// Span-specific field constants
const FIELD_KIND: &str = "kind";
const FIELD_START_TIME: &str = "startTime";
const FIELD_SUCCESS: &str = "success";
const FIELD_TRACE_STATE: &str = "traceState";
const FIELD_PARENT_ID: &str = "parentId";
const FIELD_LINKS: &str = "links";
const FIELD_STATUS_MESSAGE: &str = "statusMessage";

// OBO (On Behalf Of) field constants
const FIELD_OBO_SERVICE_ID: &str = "onbehalfServiceId";
const FIELD_OBO_ANNOTATIONS: &str = "onbehalfAnnotations";

/// Per-event OBO configuration, matching AMACA's EventStreamingAnnotation model.
/// The identity field should already contain the resolved identity
/// (altOnbehalfIdentity > onBehalfIdentity > serviceIdentity).
#[derive(Clone, Debug)]
pub struct OboEventConfig {
    pub identity: String,
    pub annotations: Option<String>,
}

impl OboEventConfig {
    /// Returns true if OBO is meaningfully configured (identity is non-empty)
    pub fn is_active(&self) -> bool {
        !self.identity.trim().is_empty()
    }

    /// Returns annotations only if non-empty
    pub fn active_annotations(&self) -> Option<&str> {
        self.annotations.as_deref().filter(|s| !s.trim().is_empty())
    }
}

/// Map of event_name -> OBO config. Events not in the map don't get OBO.
pub type OboEventMap = HashMap<String, OboEventConfig>;

fn non_blank_utf8(bytes: &[u8]) -> Option<&str> {
    let s = std::str::from_utf8(bytes).ok()?;
    (!s.trim().is_empty()).then_some(s)
}

fn normalized_event_name(record: &impl LogRecordView) -> Option<&str> {
    record.event_name().and_then(non_blank_utf8)
}

/// Look up an event in the OBO map, handling .NET-style anchored regex format.
/// Tries the literal event name first, then checks the simple anchored regex form.
pub(crate) fn lookup_obo_config<'a>(
    obo_event_map: Option<&'a OboEventMap>,
    event_name: &str,
) -> Option<&'a OboEventConfig> {
    let map = obo_event_map?;
    if let Some(config) = map.get(event_name) {
        return Some(config);
    }
    if let Some(stripped) = event_name
        .strip_prefix('^')
        .and_then(|name| name.strip_suffix('$'))
    {
        if let Some(config) = map.get(stripped) {
            return Some(config);
        }
    }
    let anchored = format!("^{event_name}$");
    map.get(&anchored)
}

#[derive(Default)]
struct RoleOverrides {
    role: Option<String>,
    role_instance: Option<String>,
}

impl RoleOverrides {
    fn from_resource<R: ResourceView>(resource: &R) -> Self {
        let mut overrides = Self::default();
        for attr in resource.attributes() {
            let Ok(key) = std::str::from_utf8(attr.key()) else {
                continue;
            };
            match key {
                "service.name" if overrides.role.is_none() => {
                    overrides.role = attr.value().and_then(|value| {
                        value_as_utf8(&value)
                            .filter(|value| !value.trim().is_empty())
                            .map(str::to_owned)
                    });
                }
                "service.instance.id" if overrides.role_instance.is_none() => {
                    overrides.role_instance = attr.value().and_then(|value| {
                        value_as_utf8(&value)
                            .filter(|value| !value.trim().is_empty())
                            .map(str::to_owned)
                    });
                }
                _ => {}
            }
        }
        overrides
    }
}

struct DynamicField {
    name: Cow<'static, str>,
    type_id: BondDataType,
    value_start: usize,
    value_len: usize,
}

impl DynamicField {
    fn field_def(&self, field_id: u16) -> FieldDef {
        FieldDef {
            name: self.name.clone(),
            type_id: self.type_id,
            field_id,
        }
    }
}

struct LogRecordParts<'a> {
    timestamp: u64,
    routing_event_name: Cow<'a, str>,
    name: Option<Cow<'a, str>>,
    severity_number: i32,
    severity_text: Option<Cow<'a, str>>,
    body: Option<Cow<'a, str>>,
    trace_id: Option<[u8; 16]>,
    span_id: Option<[u8; 8]>,
    trace_flags: Option<u32>,
    role: Cow<'a, str>,
    role_instance: Cow<'a, str>,
    obo_config: Option<&'a OboEventConfig>,
    fields: Vec<FieldDef>,
    dynamic_fields_start: usize,
    dynamic_fields: Vec<DynamicField>,
    dynamic_values: Vec<u8>,
}

impl<'a> LogRecordParts<'a> {
    fn new<R: LogRecordView>(
        record: &'a R,
        metadata_fields: &'a MetadataFields,
        resource_role: &'a RoleOverrides,
        obo_event_map: Option<&'a OboEventMap>,
    ) -> Self {
        let cs = CommonSchemaRecord::detect(record);
        let role = resource_role
            .role
            .as_deref()
            .map(Cow::Borrowed)
            .unwrap_or_else(|| Cow::Borrowed(&metadata_fields.role));
        let role_instance = resource_role
            .role_instance
            .as_deref()
            .map(Cow::Borrowed)
            .unwrap_or_else(|| Cow::Borrowed(&metadata_fields.role_instance));

        let mut parts = if cs.is_common_schema {
            Self::from_common_schema(record, role, role_instance)
        } else {
            Self::from_canonical(record, role, role_instance)
        };
        parts.obo_config = lookup_obo_config(obo_event_map, parts.routing_event_name.as_ref());
        parts.finish_fields();
        parts
    }

    fn from_canonical<R: LogRecordView>(
        record: &'a R,
        role: Cow<'a, str>,
        role_instance: Cow<'a, str>,
    ) -> Self {
        let event_name = normalized_event_name(record);
        let name = event_name.map(Cow::Borrowed);
        let routing_event_name = event_name
            .map(Cow::Borrowed)
            .unwrap_or_else(|| Cow::Borrowed(CS_LOG_TYPENAME));
        let severity_text = record
            .severity_text()
            .and_then(|b| std::str::from_utf8(b).ok())
            .filter(|s| !s.is_empty())
            .map(Cow::Borrowed);
        let body_value = record.body();
        let body = body_value.as_ref().and_then(|body| {
            (body.value_type() == ValueType::String)
                .then(|| body.as_string())
                .flatten()
                .and_then(|bytes| std::str::from_utf8(bytes).ok())
                .map(|value| Cow::Owned(value.to_owned()))
        });
        let mut parts = Self {
            timestamp: record_timestamp(record),
            routing_event_name,
            name,
            severity_number: record.severity_number().unwrap_or(0),
            severity_text,
            body,
            trace_id: record.trace_id().copied(),
            span_id: record.span_id().copied(),
            trace_flags: record.flags(),
            role,
            role_instance,
            obo_config: None,
            fields: Vec::new(),
            dynamic_fields_start: 0,
            dynamic_fields: Vec::new(),
            dynamic_values: Vec::new(),
        };

        if let Some(body) = body_value.as_ref() {
            if parts.body.is_none() {
                parts.push_dynamic_value(Cow::Borrowed(FIELD_BODY), body);
            }
        }

        for attr in record.attributes() {
            let Ok(key) = std::str::from_utf8(attr.key()) else {
                continue;
            };
            let Some(value) = attr.value() else {
                continue;
            };
            parts.push_dynamic_value(Cow::Owned(key.to_owned()), &value);
        }

        parts
    }

    fn from_common_schema<R: LogRecordView>(
        record: &'a R,
        role: Cow<'a, str>,
        role_instance: Cow<'a, str>,
    ) -> Self {
        let mut parts = Self {
            timestamp: record_timestamp(record),
            routing_event_name: Cow::Borrowed(CS_LOG_TYPENAME),
            name: None,
            severity_number: record.severity_number().unwrap_or(0),
            severity_text: record
                .severity_text()
                .and_then(|b| std::str::from_utf8(b).ok())
                .filter(|s| !s.is_empty())
                .map(Cow::Borrowed),
            body: record.body().and_then(|body| {
                (body.value_type() == ValueType::String)
                    .then(|| body.as_string())
                    .flatten()
                    .and_then(|bytes| std::str::from_utf8(bytes).ok())
                    .map(|value| Cow::Owned(value.to_owned()))
            }),
            trace_id: record.trace_id().copied(),
            span_id: record.span_id().copied(),
            trace_flags: record.flags(),
            role,
            role_instance,
            obo_config: None,
            fields: Vec::new(),
            dynamic_fields_start: 0,
            dynamic_fields: Vec::new(),
            dynamic_values: Vec::new(),
        };

        let mut part_a_name = None;
        let mut part_b_name = None;
        for attr in record.attributes() {
            let Ok(key) = std::str::from_utf8(attr.key()) else {
                continue;
            };
            let value = attr.value();
            match key {
                KEY_CSVER | KEY_PARTB_TYPENAME => {}
                "PartA.time" => {
                    if let Some(time) = value.as_ref().and_then(value_as_utf8) {
                        if let Some(nanos) = parse_rfc3339_nanos(time) {
                            parts.timestamp = nanos;
                        }
                    }
                }
                "PartA.name" => {
                    if let Some(name) = value
                        .as_ref()
                        .and_then(value_as_utf8)
                        .filter(|s| !s.is_empty())
                    {
                        part_a_name = Some(Cow::Owned(name.to_owned()));
                    }
                }
                "PartB.name" => {
                    if let Some(name) = value
                        .as_ref()
                        .and_then(value_as_utf8)
                        .filter(|s| !s.is_empty())
                    {
                        part_b_name = Some(Cow::Owned(name.to_owned()));
                    }
                }
                "PartA.ext_dt_traceId" => {
                    if let Some(trace_id) = value.as_ref().and_then(value_as_utf8) {
                        if let Some(bytes) = parse_hex_bytes::<16>(trace_id.trim()) {
                            parts.trace_id = Some(bytes);
                        }
                    }
                }
                "PartA.ext_dt_spanId" => {
                    if let Some(span_id) = value.as_ref().and_then(value_as_utf8) {
                        if let Some(bytes) = parse_hex_bytes::<8>(span_id.trim()) {
                            parts.span_id = Some(bytes);
                        }
                    }
                }
                "PartA.ext_dt_traceFlags" => {
                    if let Some(flags) = value.as_ref().and_then(value_as_i64) {
                        if let Ok(flags) = u32::try_from(flags) {
                            parts.trace_flags = Some(flags);
                        }
                    }
                }
                "PartA.ext_cloud_role" => {
                    if let Some(value) = value
                        .as_ref()
                        .and_then(value_as_utf8)
                        .filter(|s| !s.trim().is_empty())
                    {
                        parts.role = Cow::Owned(value.to_owned());
                    }
                }
                "PartA.ext_cloud_roleInstance" => {
                    if let Some(value) = value
                        .as_ref()
                        .and_then(value_as_utf8)
                        .filter(|s| !s.trim().is_empty())
                    {
                        parts.role_instance = Cow::Owned(value.to_owned());
                    }
                }
                "PartB.body" => {
                    if let Some(value) = value.as_ref() {
                        if let Some(body) = value_as_utf8(value) {
                            parts.body = Some(Cow::Owned(body.to_owned()));
                        } else {
                            parts.push_dynamic_value(Cow::Borrowed(FIELD_BODY), value);
                        }
                    }
                }
                "PartB.severityNumber" => {
                    if let Some(number) = value.as_ref().and_then(value_as_i64) {
                        parts.severity_number = number as i32;
                    }
                }
                "PartB.severityText" => {
                    if let Some(text) = value
                        .as_ref()
                        .and_then(value_as_utf8)
                        .filter(|s| !s.is_empty())
                    {
                        parts.severity_text = Some(Cow::Owned(text.to_owned()));
                    }
                }
                "PartB.eventId" => {
                    if let Some(value) = value.as_ref() {
                        parts.push_dynamic_value(Cow::Borrowed("eventId"), value);
                    }
                }
                key if key.starts_with("PartC.") => {
                    if let Some(value) = value.as_ref() {
                        parts.push_dynamic_value(
                            Cow::Owned(key["PartC.".len()..].to_owned()),
                            value,
                        );
                    }
                }
                key if key.starts_with("PartA.") => {
                    if let Some(value) = value.as_ref() {
                        parts.push_dynamic_value(
                            Cow::Owned(key["PartA.".len()..].to_owned()),
                            value,
                        );
                    }
                }
                key if key.starts_with("PartB.") => {
                    if let Some(value) = value.as_ref() {
                        parts.push_dynamic_value(
                            Cow::Owned(key["PartB.".len()..].to_owned()),
                            value,
                        );
                    }
                }
                _ => {
                    if let Some(value) = value.as_ref() {
                        parts.push_dynamic_value(Cow::Owned(key.to_owned()), value);
                    }
                }
            }
        }

        parts.name = part_b_name.or(part_a_name);
        if let Some(name) = &parts.name {
            parts.routing_event_name = Cow::Owned(name.to_string());
        }
        parts
    }

    fn push_dynamic_value<'value, V>(&mut self, name: Cow<'static, str>, value: &V) -> bool
    where
        V: AnyValueView<'value>,
    {
        let Some(type_id) = bond_type_for_value(value) else {
            return false;
        };
        let value_start = self.dynamic_values.len();
        write_view_value(&mut self.dynamic_values, value, type_id);
        let value_len = self.dynamic_values.len() - value_start;
        self.dynamic_fields.push(DynamicField {
            name,
            type_id,
            value_start,
            value_len,
        });
        true
    }

    fn finish_fields(&mut self) {
        let estimated_capacity = 14 + self.dynamic_fields.len();
        self.fields = Vec::with_capacity(estimated_capacity);
        self.fields.push(FieldDef {
            name: Cow::Borrowed(FIELD_ENV_NAME),
            type_id: BondDataType::BT_STRING,
            field_id: 1,
        });
        self.fields.push(FieldDef {
            name: Cow::Borrowed(FIELD_ENV_VER),
            type_id: BondDataType::BT_STRING,
            field_id: 2,
        });
        self.fields.push(FieldDef {
            name: Cow::Borrowed(FIELD_TIMESTAMP),
            type_id: BondDataType::BT_STRING,
            field_id: 3,
        });
        self.fields.push(FieldDef {
            name: Cow::Borrowed(FIELD_ENV_TIME),
            type_id: BondDataType::BT_STRING,
            field_id: 4,
        });
        self.fields.push(FieldDef {
            name: Cow::Borrowed(FIELD_TENANT),
            type_id: BondDataType::BT_STRING,
            field_id: 5,
        });
        self.fields.push(FieldDef {
            name: Cow::Borrowed(FIELD_ROLE),
            type_id: BondDataType::BT_STRING,
            field_id: 6,
        });
        self.fields.push(FieldDef {
            name: Cow::Borrowed(FIELD_ROLE_INSTANCE),
            type_id: BondDataType::BT_STRING,
            field_id: 7,
        });

        if self.trace_id.is_some() {
            self.push_field(FIELD_TRACE_ID, BondDataType::BT_STRING);
        }
        if self.span_id.is_some() {
            self.push_field(FIELD_SPAN_ID, BondDataType::BT_STRING);
        }
        if self.trace_flags.is_some() {
            self.push_field(FIELD_TRACE_FLAGS, BondDataType::BT_UINT32);
        }
        if self.name.is_some() {
            self.push_field(FIELD_NAME, BondDataType::BT_STRING);
        }
        self.push_field(FIELD_SEVERITY_NUMBER, BondDataType::BT_INT32);
        if self.severity_text.is_some() {
            self.push_field(FIELD_SEVERITY_TEXT, BondDataType::BT_STRING);
        }
        if self.body.is_some() {
            self.push_field(FIELD_BODY, BondDataType::BT_STRING);
        }
        if self.obo_config.is_some_and(|c| c.is_active()) {
            self.push_field(FIELD_OBO_SERVICE_ID, BondDataType::BT_STRING);
            if self
                .obo_config
                .and_then(OboEventConfig::active_annotations)
                .is_some()
            {
                self.push_field(FIELD_OBO_ANNOTATIONS, BondDataType::BT_STRING);
            }
        }

        self.dynamic_fields_start = self.fields.len();
        for dynamic in &self.dynamic_fields {
            let field_id = (self.fields.len() + 1) as u16;
            self.fields.push(dynamic.field_def(field_id));
        }
    }

    fn push_field(&mut self, name: &'static str, type_id: BondDataType) {
        self.fields.push(FieldDef {
            name: Cow::Borrowed(name),
            type_id,
            field_id: (self.fields.len() + 1) as u16,
        });
    }
}

#[derive(Default)]
struct CommonSchemaRecord {
    is_common_schema: bool,
}

impl CommonSchemaRecord {
    fn detect(record: &impl LogRecordView) -> Self {
        let mut has_version = false;
        let mut has_log_type = false;
        for attr in record.attributes() {
            let Ok(key) = std::str::from_utf8(attr.key()) else {
                continue;
            };
            match key {
                KEY_CSVER => {
                    has_version = attr
                        .value()
                        .and_then(|value| value_as_i64(&value))
                        .is_some_and(|value| value == CS_VERSION_4);
                }
                KEY_PARTB_TYPENAME => {
                    has_log_type = attr.value().is_some_and(|value| {
                        value_as_utf8(&value).is_some_and(|value| value == CS_LOG_TYPENAME)
                    });
                }
                _ => {}
            }
            if has_version && has_log_type {
                return Self {
                    is_common_schema: true,
                };
            }
        }
        Self::default()
    }
}

fn record_timestamp(record: &impl LogRecordView) -> u64 {
    record
        .time_unix_nano()
        .filter(|&t| t != 0)
        .or_else(|| record.observed_time_unix_nano())
        .unwrap_or(0)
}

fn value_as_utf8<'value, V>(value: &V) -> Option<&str>
where
    V: AnyValueView<'value>,
{
    if value.value_type() != ValueType::String {
        return None;
    }
    value
        .as_string()
        .and_then(|bytes| std::str::from_utf8(bytes).ok())
}

fn value_as_i64<'value, V>(value: &V) -> Option<i64>
where
    V: AnyValueView<'value>,
{
    // Numeric-only on purpose: string values such as "1024" or "0x400" must not
    // trigger Common Schema auto-detection by accident.
    value.as_int64()
}

fn bond_type_for_value<'value, V>(value: &V) -> Option<BondDataType>
where
    V: AnyValueView<'value>,
{
    match value.value_type() {
        ValueType::String if value_as_utf8(value).is_some() => Some(BondDataType::BT_STRING),
        ValueType::Int64 if value.as_int64().is_some() => Some(BondDataType::BT_INT64),
        ValueType::Double if value.as_double().is_some() => Some(BondDataType::BT_DOUBLE),
        ValueType::Bool if value.as_bool().is_some() => Some(BondDataType::BT_BOOL),
        _ => None,
    }
}

fn write_view_value<'value, V>(buffer: &mut Vec<u8>, value: &V, type_id: BondDataType)
where
    V: AnyValueView<'value>,
{
    match type_id {
        BondDataType::BT_STRING => {
            if let Some(value) = value_as_utf8(value) {
                BondWriter::write_string(buffer, value);
            }
        }
        BondDataType::BT_INT64 => {
            if let Some(value) = value.as_int64() {
                BondWriter::write_numeric(buffer, value);
            }
        }
        BondDataType::BT_DOUBLE => {
            if let Some(value) = value.as_double() {
                BondWriter::write_numeric(buffer, value);
            }
        }
        BondDataType::BT_BOOL => {
            if let Some(value) = value.as_bool() {
                BondWriter::write_bool(buffer, value);
            }
        }
        _ => {}
    }
}

fn parse_rfc3339_nanos(value: &str) -> Option<u64> {
    let nanos = chrono::DateTime::parse_from_rfc3339(value)
        .ok()?
        .timestamp_nanos_opt()?;
    u64::try_from(nanos).ok()
}

fn parse_hex_bytes<const N: usize>(value: &str) -> Option<[u8; N]> {
    if value.len() != N * 2 {
        return None;
    }
    let mut bytes = [0u8; N];
    hex::decode_to_slice(value, &mut bytes).ok()?;
    Some(bytes)
}

/// Metadata fields that should appear as Bond schema fields (queryable in Geneva)
#[derive(Clone, Debug)]
pub(crate) struct MetadataFields {
    pub env_name: String,
    pub env_ver: String,
    pub tenant: String,
    pub role: String,
    pub role_instance: String,
    pub namespace: String,
    pub event_version: String, // TODO - do we need both env_ver and event_version?
    metadata_string: String,   // preformatted metadata string for central blob
}

impl MetadataFields {
    pub fn new(
        env_name: String,
        env_ver: String,
        tenant: String,
        role: String,
        role_instance: String,
        namespace: String,
        event_version: String,
    ) -> Self {
        let metadata_string = format!(
            "namespace={}/eventVersion={}/tenant={}/role={}/roleinstance={}",
            namespace, event_version, tenant, role, role_instance
        );

        Self {
            env_name,
            env_ver,
            tenant,
            role,
            role_instance,
            namespace,
            event_version,
            metadata_string,
        }
    }

    /// Get pre-formatted metadata string (zero allocation in hot path)
    #[inline]
    pub(crate) fn metadata_string(&self) -> &str {
        &self.metadata_string
    }

    fn metadata_string_for(&self, role: &str, role_instance: &str) -> String {
        format!(
            "namespace={}/eventVersion={}/tenant={}/role={}/roleinstance={}",
            self.namespace, self.event_version, self.tenant, role, role_instance
        )
    }
}

// ---------------------------------------------------------------------------
// Log batch accumulator
// ---------------------------------------------------------------------------

/// Accumulates log records (from any source) into Bond batches keyed by routing
/// event name and effective role identity.
///
/// Drive it with a `for` loop calling [`push`](LogBatchAccumulator::push) for
/// each record, then call [`finalize`](LogBatchAccumulator::finalize) to
/// compress and produce the [`EncodedBatch`] list.  This design sidesteps the
/// "lending iterator" problem that arises when flattening GAT-backed view
/// iterators with `flat_map`.
struct LogBatchAccumulator {
    batches: HashMap<String, BatchData>,
}

struct BatchData {
    // Cached Arc clone of the HashMap key for cheap reuse in CentralEventEntry.
    routing_name: Arc<str>,
    blob_metadata: String,
    schemas: Vec<CentralSchemaEntry>,
    events: Vec<CentralEventEntry>,
    metadata: BatchMetadata,
}

impl BatchData {
    fn format_schema_ids(&self) -> String {
        use std::fmt::Write;

        if self.schemas.is_empty() {
            return String::new();
        }

        let estimated_capacity = self.schemas.len() * 32 + self.schemas.len().saturating_sub(1);
        self.schemas.iter().enumerate().fold(
            String::with_capacity(estimated_capacity),
            |mut acc, (i, s)| {
                if i > 0 {
                    acc.push(';');
                }
                for byte in s.md5 {
                    let _ = write!(acc, "{byte:02x}");
                }
                acc
            },
        )
    }
}

impl LogBatchAccumulator {
    fn new() -> Self {
        Self {
            batches: HashMap::new(),
        }
    }

    /// Encode a single log record and append it to the appropriate batch.
    fn push<R: LogRecordView>(
        &mut self,
        record: &R,
        metadata_fields: &MetadataFields,
        resource_role: &RoleOverrides,
        obo_event_map: Option<&OboEventMap>,
    ) {
        let parts = LogRecordParts::new(record, metadata_fields, resource_role, obo_event_map);
        let timestamp = parts.timestamp;
        let routing_event_name = parts.routing_event_name.as_ref();
        // Role identity is included because the central blob metadata is batch-level.
        // Mixing roles would make the per-row Role columns disagree with upload metadata.
        let batch_key = format!(
            "{}\0{}\0{}",
            routing_event_name, parts.role, parts.role_instance
        );

        if !self.batches.contains_key(&batch_key) {
            let key: Arc<str> = Arc::from(routing_event_name);
            self.batches.insert(
                batch_key.clone(),
                BatchData {
                    routing_name: key,
                    blob_metadata: metadata_fields
                        .metadata_string_for(parts.role.as_ref(), parts.role_instance.as_ref()),
                    schemas: Vec::new(),
                    events: Vec::new(),
                    metadata: BatchMetadata {
                        start_time: if timestamp == 0 { u64::MAX } else { timestamp },
                        end_time: timestamp,
                        schema_ids: String::new(),
                    },
                },
            );
        }

        let entry = self.batches.get_mut(&batch_key).unwrap();

        if timestamp != 0 {
            entry.metadata.start_time = entry.metadata.start_time.min(timestamp);
            entry.metadata.end_time = entry.metadata.end_time.max(timestamp);
        }

        // Find or create schema (reverse comparison: dynamic attrs vary more)
        let schema_id = match entry.schemas.iter().position(|s| {
            s.fields.len() == parts.fields.len()
                && s.fields
                    .iter()
                    .zip(&parts.fields)
                    .rev()
                    .all(|(a, b)| a.type_id == b.type_id && a.name == b.name)
        }) {
            Some(idx) => (idx + 1) as u64,
            None => {
                let new_id = (entry.schemas.len() + 1) as u64;
                let schema_entry = OtlpEncoder::create_schema(new_id, &parts.fields);
                entry.schemas.push(schema_entry);
                new_id
            }
        };

        let row_buffer = OtlpEncoder::write_row_parts(&parts, metadata_fields);
        let level = parts.severity_number as u8;
        // Reuse the Arc already stored in BatchData — just a refcount increment.
        let event_name = Arc::clone(&entry.routing_name);

        entry.events.push(CentralEventEntry {
            schema_id,
            level,
            event_name,
            row: row_buffer,
        });
    }

    /// Compress all accumulated batches and return the encoded results.
    fn finalize(self) -> Result<Vec<EncodedBatch>, String> {
        let mut blobs = Vec::with_capacity(self.batches.len());

        for (_, mut batch_data) in self.batches {
            let batch_event_name = Arc::clone(&batch_data.routing_name);
            let schema_ids_string = batch_data.format_schema_ids();
            batch_data.metadata.schema_ids = schema_ids_string;

            let schemas_count = batch_data.schemas.len();
            let events_count = batch_data.events.len();

            let blob = CentralBlob {
                version: 1,
                format: 2,
                metadata: batch_data.blob_metadata,
                schemas: batch_data.schemas,
                events: batch_data.events,
            };
            let uncompressed = blob.to_bytes();
            let compressed = lz4_chunked_compression(&uncompressed).map_err(|e| {
                debug!(
                    name: "encoder.encode_log_batch.compress_error",
                    target: "geneva-uploader",
                    event_name = %batch_event_name,
                    error = %e,
                    "LZ4 compression failed"
                );
                format!("compression failed: {e}")
            })?;

            debug!(
                name: "encoder.encode_log_batch",
                target: "geneva-uploader",
                event_name = %batch_event_name,
                schemas = schemas_count,
                events = events_count,
                uncompressed_size = uncompressed.len(),
                compressed_size = compressed.len(),
                "Encoded log batch"
            );

            blobs.push(EncodedBatch {
                event_name: batch_event_name.to_string(),
                data: compressed,
                metadata: BatchMetadata {
                    start_time: if batch_data.metadata.start_time == u64::MAX {
                        0
                    } else {
                        batch_data.metadata.start_time
                    },
                    end_time: batch_data.metadata.end_time,
                    schema_ids: batch_data.metadata.schema_ids,
                },
                row_count: events_count,
            });
        }
        Ok(blobs)
    }
}

// ---------------------------------------------------------------------------
// Encoder
// ---------------------------------------------------------------------------

/// Encoder to write OTLP/view payload in Bond form.
///
/// TODO: `OtlpEncoder` and `otlp_encoder.rs` are misnomers now that this
/// encoder handles both OTLP proto and `LogsDataView`-backed records.
/// Rename to `GenevaLogEncoder` / `log_encoder.rs` in a follow-up PR.
#[derive(Clone)]
pub(crate) struct OtlpEncoder;

impl OtlpEncoder {
    pub(crate) fn new() -> Self {
        OtlpEncoder {}
    }

    /// Encode logs from any [`LogsDataView`] implementation into LZ4-chunked
    /// compressed batches.
    ///
    /// Uses explicit nested loops rather than `flat_map` to avoid the "lending
    /// iterator" limitation that arises with GAT-backed view iterators.
    pub(crate) fn encode_logs_from_view<T: LogsDataView>(
        &self,
        view: &T,
        metadata_fields: &MetadataFields,
        obo_event_map: Option<&OboEventMap>,
    ) -> Result<Vec<EncodedBatch>, String> {
        let mut acc = LogBatchAccumulator::new();
        for resource_logs in view.resources() {
            let resource_role = resource_logs
                .resource()
                .as_ref()
                .map(RoleOverrides::from_resource)
                .unwrap_or_default();
            for scope_logs in resource_logs.scopes() {
                for log_record in scope_logs.log_records() {
                    acc.push(&log_record, metadata_fields, &resource_role, obo_event_map);
                }
            }
        }
        acc.finalize()
    }

    /// Encode a batch of spans into a single payload
    /// All spans are grouped into a single batch with event_name "Span" for routing
    /// The returned `data` field contains LZ4 chunked compressed bytes.
    /// On compression failure, the error is returned (no logging, no fallback).
    pub(crate) fn encode_span_batch<'a>(
        &self,
        spans: impl IntoIterator<Item = &'a Span>,
        metadata_fields: &MetadataFields,
        obo_event_map: Option<&OboEventMap>,
    ) -> Result<Vec<EncodedBatch>, String> {
        // All spans use "Span" as event name for routing - no grouping by span name
        const EVENT_NAME: &str = "Span";

        let obo_config = lookup_obo_config(obo_event_map, EVENT_NAME);
        let mut schemas = Vec::new();
        let mut events = Vec::new();
        let mut start_time = u64::MAX;
        let mut end_time = 0u64;

        for span in spans {
            // 1. Get schema fields
            let field_info = Self::determine_span_fields(span, EVENT_NAME, obo_config);

            // 2. Update timestamp range
            if span.start_time_unix_nano != 0 {
                start_time = start_time.min(span.start_time_unix_nano);
            }
            if span.end_time_unix_nano != 0 {
                end_time = end_time.max(span.end_time_unix_nano);
            }

            // 3. Find or create schema with exact equality check
            // Compare stored fields to avoid encoding schema per event
            // Check in reverse order: Part C (dynamic attributes) vary more than Part A/B (standard fields)
            // Check type_id first (u8 comparison) before name (&str comparison) for faster short-circuit
            let schema_id = match schemas.iter().position(|s: &CentralSchemaEntry| {
                s.fields.len() == field_info.len()
                    && s.fields
                        .iter()
                        .zip(&field_info)
                        .rev()
                        .all(|(a, b)| a.type_id == b.type_id && a.name == b.name)
            }) {
                Some(idx) => (idx + 1) as u64,
                None => {
                    // New schema - assign next auto-incrementing ID (starting from 1)
                    let new_id = (schemas.len() + 1) as u64;
                    let schema_entry = Self::create_span_schema(new_id, &field_info);
                    schemas.push(schema_entry);
                    new_id
                }
            };

            // 4. Encode row
            let row_buffer =
                self.write_span_row_data(span, &field_info, metadata_fields, obo_config);
            let level = 5; // Default level for spans (INFO equivalent)

            // 5. Create CentralEventEntry
            let central_event = CentralEventEntry {
                schema_id,
                level,
                event_name: Arc::from(EVENT_NAME),
                row: row_buffer,
            };
            events.push(central_event);
        }

        // Handle case with no spans
        if events.is_empty() {
            return Ok(Vec::new());
        }

        // Format schema IDs
        // TODO: This can be shared code with log batch
        let schema_ids_string = {
            use std::fmt::Write;
            if schemas.is_empty() {
                String::new()
            } else {
                // Pre-allocate capacity: Each MD5 hash is 32 hex chars + 1 semicolon (except last)
                // Total: (32 chars per hash * num_schemas) + (semicolons = num_schemas - 1)
                let estimated_capacity = schemas.len() * 32 + schemas.len().saturating_sub(1);
                schemas.iter().enumerate().fold(
                    String::with_capacity(estimated_capacity),
                    |mut acc, (i, s)| {
                        if i > 0 {
                            acc.push(';');
                        }
                        // Use stored MD5 hash (already computed when schema was created)
                        for byte in s.md5 {
                            let _ = write!(acc, "{byte:02x}");
                        }
                        acc
                    },
                )
            }
        };

        // Create single batch with all spans
        let batch_metadata = BatchMetadata {
            start_time: if start_time == u64::MAX {
                0
            } else {
                start_time
            },
            end_time,
            schema_ids: schema_ids_string,
        };

        let schemas_count = schemas.len();
        let events_count = events.len();
        let blob = CentralBlob {
            version: 1,
            format: 2,
            metadata: metadata_fields.metadata_string().to_owned(),
            schemas,
            events,
        };

        let uncompressed = blob.to_bytes();
        let compressed = lz4_chunked_compression(&uncompressed).map_err(|e| {
            debug!(
                name: "encoder.encode_span_batch.compress_error",
                target: "geneva-uploader",
                error = %e,
                "LZ4 compression failed for spans"
            );
            format!("compression failed: {e}")
        })?;

        debug!(
            name: "encoder.encode_span_batch",
            target: "geneva-uploader",
            event_name = EVENT_NAME,
            schemas = schemas_count,
            spans = events_count,
            uncompressed_size = uncompressed.len(),
            compressed_size = compressed.len(),
            "Encoded span batch"
        );

        Ok(vec![EncodedBatch {
            event_name: EVENT_NAME.to_string(),
            data: compressed,
            metadata: batch_metadata,
            row_count: events_count,
        }])
    }

    // ---------------------------------------------------------------------------
    // Generic log field helpers (used by LogBatchAccumulator)
    // ---------------------------------------------------------------------------

    /// Determine Bond schema fields for any [`LogRecordView`].
    #[cfg(test)]
    fn determine_fields(record: &impl LogRecordView) -> (Vec<FieldDef>, usize) {
        let metadata_fields = MetadataFields::new(
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
        );
        let role_overrides = RoleOverrides::default();
        let parts = LogRecordParts::new(record, &metadata_fields, &role_overrides, None);
        (parts.fields, parts.dynamic_fields_start)
    }

    /// Write Bond row data for any [`LogRecordView`].
    #[cfg(test)]
    fn write_row_data(
        record: &impl LogRecordView,
        fields: &[FieldDef],
        dynamic_fields_start: usize,
        metadata_fields: &MetadataFields,
    ) -> Vec<u8> {
        let role_overrides = RoleOverrides::default();
        let parts = LogRecordParts::new(record, metadata_fields, &role_overrides, None);
        debug_assert_eq!(fields.len(), parts.fields.len());
        debug_assert_eq!(dynamic_fields_start, parts.dynamic_fields_start);
        Self::write_row_parts(&parts, metadata_fields)
    }

    fn write_row_parts(parts: &LogRecordParts<'_>, metadata_fields: &MetadataFields) -> Vec<u8> {
        let mut buffer = Vec::with_capacity(parts.fields.len() * 50);
        let formatted_timestamp = Self::format_timestamp(parts.timestamp);
        for field in &parts.fields[..parts.dynamic_fields_start] {
            match field.name.as_ref() {
                FIELD_ENV_NAME => BondWriter::write_string(&mut buffer, &metadata_fields.env_name),
                FIELD_ENV_VER => BondWriter::write_string(&mut buffer, &metadata_fields.env_ver),
                FIELD_TENANT => BondWriter::write_string(&mut buffer, &metadata_fields.tenant),
                FIELD_ROLE => BondWriter::write_string(&mut buffer, &parts.role),
                FIELD_ROLE_INSTANCE => BondWriter::write_string(&mut buffer, &parts.role_instance),
                FIELD_TIMESTAMP | FIELD_ENV_TIME => {
                    BondWriter::write_string(&mut buffer, &formatted_timestamp);
                }
                FIELD_TRACE_ID => {
                    if let Some(id) = parts.trace_id {
                        let hex = Self::encode_id_to_hex::<32>(&id);
                        let s = std::str::from_utf8(&hex)
                            .expect("hex encoding always produces valid UTF-8");
                        BondWriter::write_string(&mut buffer, s);
                    }
                }
                FIELD_SPAN_ID => {
                    if let Some(id) = parts.span_id {
                        let hex = Self::encode_id_to_hex::<16>(&id);
                        let s = std::str::from_utf8(&hex)
                            .expect("hex encoding always produces valid UTF-8");
                        BondWriter::write_string(&mut buffer, s);
                    }
                }
                FIELD_TRACE_FLAGS => {
                    BondWriter::write_numeric(&mut buffer, parts.trace_flags.unwrap_or(0));
                }
                FIELD_NAME => {
                    BondWriter::write_string(&mut buffer, parts.name.as_deref().unwrap_or(""));
                }
                FIELD_SEVERITY_NUMBER => {
                    BondWriter::write_numeric(&mut buffer, parts.severity_number);
                }
                FIELD_SEVERITY_TEXT => {
                    if let Some(text) = &parts.severity_text {
                        BondWriter::write_string(&mut buffer, text);
                    }
                }
                FIELD_BODY => {
                    if let Some(body) = &parts.body {
                        BondWriter::write_string(&mut buffer, body);
                    }
                }
                FIELD_OBO_SERVICE_ID => {
                    if let Some(config) = parts.obo_config.filter(|c| c.is_active()) {
                        BondWriter::write_string(&mut buffer, config.identity.trim());
                    }
                }
                FIELD_OBO_ANNOTATIONS => {
                    if let Some(ann) = parts
                        .obo_config
                        .and_then(OboEventConfig::active_annotations)
                    {
                        BondWriter::write_string(&mut buffer, ann);
                    }
                }
                _ => {}
            }
        }

        let expected_dynamic_fields = parts
            .fields
            .len()
            .saturating_sub(parts.dynamic_fields_start);
        if expected_dynamic_fields > 0 {
            for dynamic in &parts.dynamic_fields {
                buffer.extend_from_slice(
                    &parts.dynamic_values
                        [dynamic.value_start..dynamic.value_start + dynamic.value_len],
                );
            }
        }
        buffer
    }

    // ---------------------------------------------------------------------------
    // Span field helpers (unchanged)
    // ---------------------------------------------------------------------------

    /// Determine span fields
    fn determine_span_fields(
        span: &Span,
        _event_name: &str,
        obo_config: Option<&OboEventConfig>,
    ) -> Vec<FieldDef> {
        // Pre-allocate with estimated capacity to avoid reallocations
        let estimated_capacity = 18 + span.attributes.len(); // 7 base + 3 tenant/role + 3 span-specific + 5 max conditional + attributes
        let mut fields = Vec::with_capacity(estimated_capacity);

        // Part A - Always present fields for spans
        fields.push((Cow::Borrowed(FIELD_ENV_NAME), BondDataType::BT_STRING));
        fields.push((FIELD_ENV_VER.into(), BondDataType::BT_STRING));
        fields.push((FIELD_TIMESTAMP.into(), BondDataType::BT_STRING));
        fields.push((FIELD_ENV_TIME.into(), BondDataType::BT_STRING));
        fields.push((FIELD_TENANT.into(), BondDataType::BT_STRING));
        fields.push((FIELD_ROLE.into(), BondDataType::BT_STRING));
        fields.push((FIELD_ROLE_INSTANCE.into(), BondDataType::BT_STRING));

        // Span-specific required fields
        fields.push((FIELD_KIND.into(), BondDataType::BT_INT32));
        fields.push((FIELD_START_TIME.into(), BondDataType::BT_STRING));
        fields.push((FIELD_SUCCESS.into(), BondDataType::BT_BOOL));

        // Part A extension - Conditional fields
        if !span.trace_id.is_empty() {
            fields.push((FIELD_TRACE_ID.into(), BondDataType::BT_STRING));
        }
        if !span.span_id.is_empty() {
            fields.push((FIELD_SPAN_ID.into(), BondDataType::BT_STRING));
        }
        if span.flags != 0 {
            fields.push((FIELD_TRACE_FLAGS.into(), BondDataType::BT_UINT32));
        }

        // Part B - Span-specific optional fields
        if !span.name.is_empty() {
            fields.push((FIELD_NAME.into(), BondDataType::BT_STRING));
        }
        if !span.trace_state.is_empty() {
            fields.push((FIELD_TRACE_STATE.into(), BondDataType::BT_STRING));
        }
        if !span.parent_span_id.is_empty() {
            fields.push((FIELD_PARENT_ID.into(), BondDataType::BT_STRING));
        }
        if !span.links.is_empty() {
            fields.push((FIELD_LINKS.into(), BondDataType::BT_STRING));
        }
        if let Some(status) = &span.status {
            if !status.message.is_empty() {
                fields.push((FIELD_STATUS_MESSAGE.into(), BondDataType::BT_STRING));
            }
        }
        if obo_config.is_some_and(|c| c.is_active()) {
            fields.push((FIELD_OBO_SERVICE_ID.into(), BondDataType::BT_STRING));
            if obo_config
                .and_then(OboEventConfig::active_annotations)
                .is_some()
            {
                fields.push((FIELD_OBO_ANNOTATIONS.into(), BondDataType::BT_STRING));
            }
        }

        // Part C - Dynamic attributes
        for attr in &span.attributes {
            if let Some(val) = attr.value.as_ref().and_then(|v| v.value.as_ref()) {
                let type_id = match val {
                    Value::StringValue(_) => BondDataType::BT_STRING,
                    Value::IntValue(_) => BondDataType::BT_INT64,
                    Value::DoubleValue(_) => BondDataType::BT_DOUBLE,
                    Value::BoolValue(_) => BondDataType::BT_BOOL,
                    _ => continue,
                };
                fields.push((attr.key.clone().into(), type_id));
            }
        }

        // Convert to FieldDef with field IDs
        fields
            .into_iter()
            .enumerate()
            .map(|(i, (name, type_id))| FieldDef {
                name,
                type_id,
                field_id: (i + 1) as u16,
            })
            .collect()
    }

    /// Create log schema entry.
    fn create_schema(schema_id: u64, field_info: &[FieldDef]) -> CentralSchemaEntry {
        let schema = BondEncodedSchema::from_fields("OtlpLogRecord", "telemetry", field_info); //TODO - use actual struct name and namespace

        let schema_bytes = schema.as_bytes();
        let schema_md5: [u8; 16] = Md5::digest(schema_bytes).into();

        CentralSchemaEntry {
            id: schema_id,
            md5: schema_md5,
            schema,
            fields: field_info.to_vec(),
        }
    }

    /// Create span schema entry.
    fn create_span_schema(schema_id: u64, field_info: &[FieldDef]) -> CentralSchemaEntry {
        let schema = BondEncodedSchema::from_fields("OtlpSpanRecord", "telemetry", field_info);

        let schema_bytes = schema.as_bytes();
        let schema_md5: [u8; 16] = Md5::digest(schema_bytes).into();

        CentralSchemaEntry {
            id: schema_id,
            md5: schema_md5,
            schema,
            fields: field_info.to_vec(),
        }
    }

    /// Write span row data directly from Span
    // TODO - code duplication between write_span_row_data() and write_row_data() - consider extracting common field handling
    fn write_span_row_data(
        &self,
        span: &Span,
        fields: &[FieldDef],
        metadata_fields: &MetadataFields,
        obo_config: Option<&OboEventConfig>,
    ) -> Vec<u8> {
        let mut buffer = Vec::with_capacity(fields.len() * 50);

        // Pre-calculate timestamp (use start time as primary timestamp for both fields)
        let formatted_timestamp = Self::format_timestamp(span.start_time_unix_nano);

        for field in fields {
            match field.name.as_ref() {
                FIELD_ENV_NAME => BondWriter::write_string(&mut buffer, &metadata_fields.env_name),
                FIELD_ENV_VER => BondWriter::write_string(&mut buffer, &metadata_fields.env_ver),
                FIELD_TENANT => BondWriter::write_string(&mut buffer, &metadata_fields.tenant),
                FIELD_ROLE => BondWriter::write_string(&mut buffer, &metadata_fields.role),
                FIELD_ROLE_INSTANCE => {
                    BondWriter::write_string(&mut buffer, &metadata_fields.role_instance)
                }
                FIELD_TIMESTAMP | FIELD_ENV_TIME => {
                    BondWriter::write_string(&mut buffer, &formatted_timestamp);
                }
                FIELD_KIND => {
                    BondWriter::write_numeric(&mut buffer, span.kind);
                }
                FIELD_START_TIME => {
                    BondWriter::write_string(&mut buffer, &formatted_timestamp);
                }
                FIELD_SUCCESS => {
                    // Determine success based on status
                    let success = match &span.status {
                        Some(status) => {
                            use opentelemetry_proto::tonic::trace::v1::status::StatusCode;
                            match StatusCode::try_from(status.code) {
                                Ok(StatusCode::Ok) => true,
                                Ok(StatusCode::Error) => false,
                                _ => true, // Unset or unknown defaults to true
                            }
                        }
                        None => true, // No status defaults to true
                    };
                    BondWriter::write_bool(&mut buffer, success);
                }
                FIELD_TRACE_ID => {
                    let hex_bytes = Self::encode_id_to_hex::<32>(&span.trace_id);
                    let hex_str = std::str::from_utf8(&hex_bytes)
                        .expect("hex encoding always produces valid UTF-8");
                    BondWriter::write_string(&mut buffer, hex_str);
                }
                FIELD_SPAN_ID => {
                    let hex_bytes = Self::encode_id_to_hex::<16>(&span.span_id);
                    let hex_str = std::str::from_utf8(&hex_bytes)
                        .expect("hex encoding always produces valid UTF-8");
                    BondWriter::write_string(&mut buffer, hex_str);
                }
                FIELD_TRACE_FLAGS => {
                    BondWriter::write_numeric(&mut buffer, span.flags);
                }
                FIELD_NAME => {
                    BondWriter::write_string(&mut buffer, &span.name);
                }
                FIELD_TRACE_STATE => {
                    BondWriter::write_string(&mut buffer, &span.trace_state);
                }
                FIELD_PARENT_ID => {
                    let hex_bytes = Self::encode_id_to_hex::<16>(&span.parent_span_id);
                    let hex_str = std::str::from_utf8(&hex_bytes)
                        .expect("hex encoding always produces valid UTF-8");
                    BondWriter::write_string(&mut buffer, hex_str);
                }
                FIELD_LINKS => {
                    // Manual JSON building to avoid intermediate allocations
                    let links_json = Self::serialize_links(&span.links);
                    BondWriter::write_string(&mut buffer, &links_json);
                }
                FIELD_STATUS_MESSAGE => {
                    if let Some(status) = &span.status {
                        BondWriter::write_string(&mut buffer, &status.message);
                    }
                }
                FIELD_OBO_SERVICE_ID => {
                    if let Some(config) = obo_config.filter(|c| c.is_active()) {
                        BondWriter::write_string(&mut buffer, config.identity.trim());
                    }
                }
                FIELD_OBO_ANNOTATIONS => {
                    if let Some(ann) = obo_config.and_then(OboEventConfig::active_annotations) {
                        BondWriter::write_string(&mut buffer, ann);
                    }
                }
                _ => {
                    // Handle dynamic attributes
                    // TODO - optimize better - we could update determine_fields to also return a vec of bytes which has bond serialized attributes
                    if let Some(attr) = span.attributes.iter().find(|a| a.key == field.name) {
                        self.write_attribute_value(&mut buffer, attr, field.type_id);
                    }
                }
            }
        }

        buffer
    }

    fn encode_id_to_hex<const N: usize>(id: &[u8]) -> [u8; N] {
        let mut hex_bytes = [0u8; N];
        // If encoding fails (buffer size mismatch), log error and return zeros
        if let Err(e) = hex::encode_to_slice(id, &mut hex_bytes) {
            let id_type = match N {
                32 => "trace ID",
                16 => "span ID",
                _ => "input",
            };
            error!(
                name: "encoder.encode_id_to_hex.error",
                target: "geneva-uploader",
                error = %e,
                id_len = id.len(),
                buffer_size = N,
                "Hex encoding failed, using zeros - indicates an invalid {}",
                id_type
            );
        }
        hex_bytes
    }

    /// Links serialization
    fn serialize_links(links: &[opentelemetry_proto::tonic::trace::v1::span::Link]) -> String {
        use std::fmt::Write;

        if links.is_empty() {
            return "[]".to_string();
        }

        // Estimate capacity: Each link needs ~80 chars for JSON structure + 32 chars for trace_id + 16 chars for span_id
        // JSON overhead: {"toSpanId":"","toTraceId":""} = ~30 chars + commas/brackets
        let estimated_capacity = links.len() * 128 + 2; // Extra buffer for safety
        let mut json = String::with_capacity(estimated_capacity);

        json.push('[');

        for (i, link) in links.iter().enumerate() {
            if i > 0 {
                json.push(',');
            }

            json.push_str(r#"{"toSpanId":""#);

            // Write hex directly to avoid temporary string allocation
            for &byte in &link.span_id {
                let _ = write!(&mut json, "{byte:02x}");
            }

            json.push_str(r#"","toTraceId":""#);

            // Write hex directly to avoid temporary string allocation
            for &byte in &link.trace_id {
                let _ = write!(&mut json, "{byte:02x}");
            }

            json.push_str(r#""}"#);
        }

        json.push(']');
        json
    }

    /// Format timestamp from nanoseconds to RFC3339 string
    fn format_timestamp(nanos: u64) -> String {
        let secs = (nanos / 1_000_000_000) as i64;
        let nsec = (nanos % 1_000_000_000) as u32;

        match Utc.timestamp_opt(secs, nsec).single() {
            Some(dt) => dt.to_rfc3339(),
            None => {
                error!(
                    name: "encoder.format_timestamp.invalid",
                    target: "geneva-uploader",
                    nanos = nanos,
                    secs = secs,
                    nsec = nsec,
                    "Timestamp out of range, using epoch"
                );
                "1970-01-01T00:00:00+00:00".to_string()
            }
        }
    }

    /// Write attribute value based on its type
    fn write_attribute_value(
        &self,
        buffer: &mut Vec<u8>,
        attr: &opentelemetry_proto::tonic::common::v1::KeyValue,
        expected_type: BondDataType,
    ) {
        if let Some(val) = &attr.value {
            match (&val.value, expected_type) {
                (Some(Value::StringValue(s)), BondDataType::BT_STRING) => {
                    BondWriter::write_string(buffer, s)
                }
                (Some(Value::IntValue(i)), BondDataType::BT_INT64) => {
                    BondWriter::write_numeric(buffer, *i)
                }
                (Some(Value::DoubleValue(d)), BondDataType::BT_DOUBLE) => {
                    BondWriter::write_numeric(buffer, *d)
                }
                (Some(Value::BoolValue(b)), BondDataType::BT_BOOL) => {
                    // TODO - represent bool as BT_BOOL
                    BondWriter::write_bool(buffer, *b)
                }
                _ => {} // TODO - handle more types
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry_proto::tonic::common::v1::{AnyValue, KeyValue};
    use opentelemetry_proto::tonic::logs::v1::{LogRecord, ResourceLogs, ScopeLogs};
    use opentelemetry_proto::tonic::resource::v1::Resource;

    fn make_metadata(namespace: &str) -> MetadataFields {
        MetadataFields::new(
            "TestEnv".to_string(),
            "Ver1v0".to_string(),
            "TestTenant".to_string(),
            "TestRole".to_string(),
            "TestRoleInstance".to_string(),
            namespace.to_string(),
            "Ver1v0".to_string(),
        )
    }

    fn encode_log_batch_via_proto<'a>(
        encoder: &OtlpEncoder,
        logs: impl IntoIterator<Item = &'a LogRecord>,
        metadata: &MetadataFields,
    ) -> Result<Vec<EncodedBatch>, String> {
        encode_log_batch_with_resource_attrs(encoder, logs, Vec::new(), metadata)
    }

    fn encode_log_batch_with_resource_attrs<'a>(
        encoder: &OtlpEncoder,
        logs: impl IntoIterator<Item = &'a LogRecord>,
        resource_attrs: Vec<KeyValue>,
        metadata: &MetadataFields,
    ) -> Result<Vec<EncodedBatch>, String> {
        use otap_df_pdata::views::otlp::bytes::logs::RawLogsData;
        use prost::Message as _;
        let log_records: Vec<LogRecord> = logs.into_iter().cloned().collect();
        let bytes = opentelemetry_proto::tonic::collector::logs::v1::ExportLogsServiceRequest {
            resource_logs: vec![ResourceLogs {
                resource: (!resource_attrs.is_empty()).then_some(Resource {
                    attributes: resource_attrs,
                    ..Default::default()
                }),
                scope_logs: vec![ScopeLogs {
                    log_records,
                    ..Default::default()
                }],
                ..Default::default()
            }],
        }
        .encode_to_vec();
        encoder.encode_logs_from_view(&RawLogsData::new(&bytes), metadata, None)
    }

    fn string_attr(key: &str, value: &str) -> KeyValue {
        KeyValue {
            key: key.to_string(),
            value: Some(AnyValue {
                value: Some(Value::StringValue(value.to_string())),
            }),
        }
    }

    fn int_attr(key: &str, value: i64) -> KeyValue {
        KeyValue {
            key: key.to_string(),
            value: Some(AnyValue {
                value: Some(Value::IntValue(value)),
            }),
        }
    }

    fn bool_attr(key: &str, value: bool) -> KeyValue {
        KeyValue {
            key: key.to_string(),
            value: Some(AnyValue {
                value: Some(Value::BoolValue(value)),
            }),
        }
    }

    fn double_attr(key: &str, value: f64) -> KeyValue {
        KeyValue {
            key: key.to_string(),
            value: Some(AnyValue {
                value: Some(Value::DoubleValue(value)),
            }),
        }
    }

    /// Wraps raw LogRecord proto bytes into a minimal ExportLogsServiceRequest encoding.
    /// Useful for injecting hand-crafted or otherwise non-prost-encodable bytes.
    fn wrap_log_record_bytes(log_bytes: &[u8]) -> Vec<u8> {
        fn len_delim(tag: u8, data: &[u8]) -> Vec<u8> {
            let mut out = vec![tag];
            let mut v = data.len();
            loop {
                let b = (v & 0x7F) as u8;
                v >>= 7;
                if v == 0 {
                    out.push(b);
                    break;
                }
                out.push(b | 0x80);
            }
            out.extend_from_slice(data);
            out
        }
        // ScopeLogs.log_records = field 2 (LEN): tag 0x12
        let scope = len_delim(0x12, log_bytes);
        // ResourceLogs.scope_logs = field 2 (LEN): tag 0x12
        let rl = len_delim(0x12, &scope);
        // ExportLogsServiceRequest.resource_logs = field 1 (LEN): tag 0x0A
        len_delim(0x0A, &rl)
    }

    fn assert_single_batch_equal(left: &[EncodedBatch], right: &[EncodedBatch]) {
        assert_eq!(left.len(), 1);
        assert_eq!(right.len(), 1);

        let lhs = &left[0];
        let rhs = &right[0];

        assert_eq!(lhs.event_name, rhs.event_name);
        assert_eq!(lhs.data, rhs.data);
        assert_eq!(lhs.metadata.start_time, rhs.metadata.start_time);
        assert_eq!(lhs.metadata.end_time, rhs.metadata.end_time);
        assert_eq!(lhs.metadata.schema_ids, rhs.metadata.schema_ids);
        assert_eq!(lhs.row_count, rhs.row_count);
    }

    #[test]
    fn test_encoding() {
        let encoder = OtlpEncoder::new();

        let mut log = LogRecord {
            observed_time_unix_nano: 1_700_000_000_000_000_000,
            event_name: "test_event".to_string(),
            severity_number: 9,
            severity_text: "INFO".to_string(),
            ..Default::default()
        };

        // Add some attributes
        log.attributes.push(KeyValue {
            key: "user_id".to_string(),
            value: Some(AnyValue {
                value: Some(Value::StringValue("user123".to_string())),
            }),
        });

        log.attributes.push(KeyValue {
            key: "request_count".to_string(),
            value: Some(AnyValue {
                value: Some(Value::IntValue(42)),
            }),
        });

        let metadata = make_metadata("testNamespace");
        let result = encode_log_batch_via_proto(&encoder, [log].iter(), &metadata).unwrap();

        assert!(!result.is_empty());
    }

    #[test]
    fn test_multiple_schemas_per_batch() {
        let encoder = OtlpEncoder::new();

        // Create multiple log records with different schema structures
        // to test that multiple schemas can exist within the same batch
        let log1 = LogRecord {
            observed_time_unix_nano: 1_700_000_000_000_000_000,
            event_name: "user_action".to_string(),
            severity_number: 9,
            severity_text: "INFO".to_string(),
            ..Default::default()
        };

        // Schema 2: Same event_name but with trace_id (different schema)
        let mut log2 = LogRecord {
            event_name: "user_action".to_string(),
            observed_time_unix_nano: 1_700_000_001_000_000_000,
            severity_number: 10,
            severity_text: "WARN".to_string(),
            ..Default::default()
        };
        log2.trace_id = vec![1; 16];

        // Schema 3: Same event_name but with attributes (different schema)
        let mut log3 = LogRecord {
            event_name: "user_action".to_string(),
            observed_time_unix_nano: 1_700_000_002_000_000_000,
            severity_number: 11,
            severity_text: "ERROR".to_string(),
            ..Default::default()
        };
        log3.attributes.push(KeyValue {
            key: "user_id".to_string(),
            value: Some(AnyValue {
                value: Some(Value::StringValue("user123".to_string())),
            }),
        });

        let metadata = make_metadata("test");

        // Encode multiple log records with different schema structures but same event_name
        let result =
            encode_log_batch_via_proto(&encoder, [log1, log2, log3].iter(), &metadata).unwrap();

        // Should create one batch (same event_name = "user_action")
        assert_eq!(result.len(), 1);
        let batch = &result[0];
        assert_eq!(batch.event_name, "user_action");

        // Verify that multiple schemas were created within the same batch
        // schema_ids should contain multiple semicolon-separated MD5 hashes
        let schema_ids = &batch.metadata.schema_ids;
        assert!(!schema_ids.is_empty());

        // Split by semicolon to get individual schema IDs
        let schema_id_list: Vec<&str> = schema_ids.split(';').collect();

        // Should have 3 different schema IDs (one per unique schema structure)
        assert_eq!(
            schema_id_list.len(),
            3,
            "Expected 3 schema IDs but found {}: {}",
            schema_id_list.len(),
            schema_ids
        );

        // Verify each schema ID is a valid MD5 hash (32 hex characters)
        for schema_id in schema_id_list {
            assert_eq!(
                schema_id.len(),
                32,
                "Schema ID should be 32 hex characters: {schema_id}"
            );
            assert!(
                schema_id.chars().all(|c| c.is_ascii_hexdigit()),
                "Schema ID should contain only hex characters: {schema_id}"
            );
        }
    }

    #[test]
    fn test_single_event_single_schema() {
        let encoder = OtlpEncoder::new();

        let log = LogRecord {
            observed_time_unix_nano: 1_700_000_000_000_000_000,
            event_name: "test_event".to_string(),
            severity_number: 9,
            ..Default::default()
        };

        let metadata = make_metadata("test");
        let result = encode_log_batch_via_proto(&encoder, [log].iter(), &metadata).unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].event_name, "test_event");
        assert_eq!(result[0].compressed_size(), result[0].data.len());
        assert!(result[0].compressed_size() > 0);
    }

    #[test]
    fn test_whitespace_event_name_defaults_to_log() {
        let encoder = OtlpEncoder::new();

        let log = LogRecord {
            event_name: "   \t".to_string(),
            severity_number: 9,
            ..Default::default()
        };

        let metadata = make_metadata("test");
        let result = encode_log_batch_via_proto(&encoder, [log].iter(), &metadata).unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].event_name, "Log");
    }

    #[test]
    fn test_proto_view_encoding_smoke() {
        let encoder = OtlpEncoder::new();
        let metadata = make_metadata("view-parity");

        let mut log = LogRecord {
            observed_time_unix_nano: 1_700_000_000_123_456_789,
            event_name: "view_event".to_string(),
            severity_number: 13,
            severity_text: "WARN".to_string(),
            body: Some(AnyValue {
                value: Some(Value::StringValue("hello".to_string())),
            }),
            trace_id: vec![0x11; 16],
            span_id: vec![0x22; 8],
            flags: 1,
            ..Default::default()
        };
        log.attributes.push(KeyValue {
            key: "user".to_string(),
            value: Some(AnyValue {
                value: Some(Value::StringValue("alice".to_string())),
            }),
        });
        log.attributes.push(KeyValue {
            key: "attempt".to_string(),
            value: Some(AnyValue {
                value: Some(Value::IntValue(2)),
            }),
        });

        let encoded =
            encode_log_batch_via_proto(&encoder, [log.clone()].iter(), &metadata).unwrap();
        // Smoke test: a rich proto-backed view encodes to a single non-empty batch.
        assert_eq!(encoded.len(), 1);
        assert_eq!(encoded[0].event_name, "view_event");
        assert!(!encoded[0].data.is_empty());
    }

    #[test]
    fn test_common_schema_log_matches_equivalent_canonical_log() {
        let encoder = OtlpEncoder::new();
        let metadata = make_metadata("cs-parity");
        let timestamp = parse_rfc3339_nanos("2024-06-15T06:00:00Z").unwrap();

        let canonical = LogRecord {
            time_unix_nano: timestamp,
            event_name: "CheckoutFailure".to_string(),
            severity_number: 17,
            severity_text: "ERROR".to_string(),
            body: Some(AnyValue {
                value: Some(Value::StringValue("failed".to_string())),
            }),
            trace_id: hex::decode("0102030405060708090a0b0c0d0e0f10").unwrap(),
            span_id: hex::decode("a1b2c3d4e5f60718").unwrap(),
            flags: 1,
            attributes: vec![
                int_attr("eventId", 42),
                string_attr("iKey", "AIM-abcd-1234"),
                int_attr("result", 127),
                double_attr("duration", 53.16),
                bool_attr("success", false),
            ],
            ..Default::default()
        };

        let common_schema = LogRecord {
            observed_time_unix_nano: 1,
            attributes: vec![
                int_attr(KEY_CSVER, CS_VERSION_4),
                string_attr(KEY_PARTB_TYPENAME, CS_LOG_TYPENAME),
                string_attr("PartA.time", "2024-06-15T06:00:00Z"),
                string_attr("PartA.ext_dt_traceId", "0102030405060708090a0b0c0d0e0f10"),
                string_attr("PartA.ext_dt_spanId", "a1b2c3d4e5f60718"),
                int_attr("PartA.ext_dt_traceFlags", 1),
                string_attr("PartB.body", "failed"),
                int_attr("PartB.severityNumber", 17),
                string_attr("PartB.severityText", "ERROR"),
                string_attr("PartB.name", "CheckoutFailure"),
                int_attr("PartB.eventId", 42),
                string_attr("PartA.iKey", "AIM-abcd-1234"),
                int_attr("PartC.result", 127),
                double_attr("PartC.duration", 53.16),
                bool_attr("PartC.success", false),
            ],
            ..Default::default()
        };

        let canonical_encoded =
            encode_log_batch_via_proto(&encoder, std::iter::once(&canonical), &metadata).unwrap();
        let common_schema_encoded =
            encode_log_batch_via_proto(&encoder, std::iter::once(&common_schema), &metadata)
                .unwrap();

        assert_single_batch_equal(&canonical_encoded, &common_schema_encoded);
    }

    #[test]
    fn test_common_schema_role_overrides_match_resource_service_attributes() {
        let encoder = OtlpEncoder::new();
        let metadata = make_metadata("cs-role-parity");

        let canonical = LogRecord {
            time_unix_nano: parse_rfc3339_nanos("2024-06-15T06:00:00Z").unwrap(),
            event_name: "RoleEvent".to_string(),
            severity_number: 9,
            body: Some(AnyValue {
                value: Some(Value::StringValue("body".to_string())),
            }),
            ..Default::default()
        };

        let common_schema = LogRecord {
            attributes: vec![
                int_attr(KEY_CSVER, CS_VERSION_4),
                string_attr(KEY_PARTB_TYPENAME, CS_LOG_TYPENAME),
                string_attr("PartA.time", "2024-06-15T06:00:00Z"),
                string_attr("PartA.ext_cloud_role", "checkout"),
                string_attr("PartA.ext_cloud_roleInstance", "instance-1"),
                string_attr("PartB.name", "RoleEvent"),
                int_attr("PartB.severityNumber", 9),
                string_attr("PartB.body", "body"),
            ],
            ..Default::default()
        };

        let canonical_encoded = encode_log_batch_with_resource_attrs(
            &encoder,
            std::iter::once(&canonical),
            vec![
                string_attr("service.name", "checkout"),
                string_attr("service.instance.id", "instance-1"),
            ],
            &metadata,
        )
        .unwrap();
        let common_schema_encoded =
            encode_log_batch_via_proto(&encoder, std::iter::once(&common_schema), &metadata)
                .unwrap();

        assert_single_batch_equal(&canonical_encoded, &common_schema_encoded);
    }

    #[test]
    fn test_common_schema_role_overrides_split_batches() {
        let encoder = OtlpEncoder::new();
        let metadata = make_metadata("cs-role-split");

        let checkout = LogRecord {
            attributes: vec![
                int_attr(KEY_CSVER, CS_VERSION_4),
                string_attr(KEY_PARTB_TYPENAME, CS_LOG_TYPENAME),
                string_attr("PartA.ext_cloud_role", "checkout"),
                string_attr("PartA.ext_cloud_roleInstance", "instance-1"),
                string_attr("PartB.name", "SharedEvent"),
                string_attr("PartB.body", "checkout"),
            ],
            ..Default::default()
        };
        let billing = LogRecord {
            attributes: vec![
                int_attr(KEY_CSVER, CS_VERSION_4),
                string_attr(KEY_PARTB_TYPENAME, CS_LOG_TYPENAME),
                string_attr("PartA.ext_cloud_role", "billing"),
                string_attr("PartA.ext_cloud_roleInstance", "instance-2"),
                string_attr("PartB.name", "SharedEvent"),
                string_attr("PartB.body", "billing"),
            ],
            ..Default::default()
        };

        let encoded =
            encode_log_batch_via_proto(&encoder, [&checkout, &billing], &metadata).unwrap();

        assert_eq!(encoded.len(), 2);
        assert!(encoded
            .iter()
            .all(|batch| batch.event_name == "SharedEvent" && batch.row_count == 1));
    }

    #[test]
    fn test_common_schema_non_string_body_is_preserved_as_part_c_body() {
        let encoder = OtlpEncoder::new();
        let metadata = make_metadata("cs-non-string-body");

        let canonical = LogRecord {
            event_name: "BodyEvent".to_string(),
            body: Some(AnyValue {
                value: Some(Value::IntValue(42)),
            }),
            ..Default::default()
        };

        let common_schema = LogRecord {
            attributes: vec![
                int_attr(KEY_CSVER, CS_VERSION_4),
                string_attr(KEY_PARTB_TYPENAME, CS_LOG_TYPENAME),
                string_attr("PartB.name", "BodyEvent"),
                int_attr("PartB.body", 42),
            ],
            ..Default::default()
        };

        let canonical_encoded =
            encode_log_batch_via_proto(&encoder, std::iter::once(&canonical), &metadata).unwrap();
        let common_schema_encoded =
            encode_log_batch_via_proto(&encoder, std::iter::once(&common_schema), &metadata)
                .unwrap();

        assert_single_batch_equal(&canonical_encoded, &common_schema_encoded);
    }

    #[test]
    fn test_common_schema_malformed_trace_context_is_omitted_like_canonical() {
        let encoder = OtlpEncoder::new();
        let metadata = make_metadata("cs-malformed-trace-context");

        let canonical = LogRecord {
            event_name: "MalformedIds".to_string(),
            body: Some(AnyValue {
                value: Some(Value::StringValue("body".to_string())),
            }),
            ..Default::default()
        };

        let common_schema = LogRecord {
            attributes: vec![
                int_attr(KEY_CSVER, CS_VERSION_4),
                string_attr(KEY_PARTB_TYPENAME, CS_LOG_TYPENAME),
                string_attr("PartB.name", "MalformedIds"),
                string_attr("PartB.body", "body"),
                string_attr("PartA.ext_dt_traceId", "not-a-trace-id"),
                string_attr("PartA.ext_dt_spanId", "   "),
            ],
            ..Default::default()
        };

        let canonical_encoded =
            encode_log_batch_via_proto(&encoder, std::iter::once(&canonical), &metadata).unwrap();
        let common_schema_encoded =
            encode_log_batch_via_proto(&encoder, std::iter::once(&common_schema), &metadata)
                .unwrap();

        assert_single_batch_equal(&canonical_encoded, &common_schema_encoded);
    }

    #[test]
    fn test_common_schema_out_of_range_severity_matches_canonical() {
        let encoder = OtlpEncoder::new();
        let metadata = make_metadata("cs-severity-parity");

        let canonical = LogRecord {
            event_name: "SeverityEvent".to_string(),
            severity_number: 50,
            body: Some(AnyValue {
                value: Some(Value::StringValue("body".to_string())),
            }),
            ..Default::default()
        };

        let common_schema = LogRecord {
            attributes: vec![
                int_attr(KEY_CSVER, CS_VERSION_4),
                string_attr(KEY_PARTB_TYPENAME, CS_LOG_TYPENAME),
                string_attr("PartB.name", "SeverityEvent"),
                int_attr("PartB.severityNumber", 50),
                string_attr("PartB.body", "body"),
            ],
            ..Default::default()
        };

        let canonical_encoded =
            encode_log_batch_via_proto(&encoder, std::iter::once(&canonical), &metadata).unwrap();
        let common_schema_encoded =
            encode_log_batch_via_proto(&encoder, std::iter::once(&common_schema), &metadata)
                .unwrap();

        assert_single_batch_equal(&canonical_encoded, &common_schema_encoded);
    }

    #[test]
    fn test_common_schema_blank_role_matches_blank_resource_service_attributes() {
        let encoder = OtlpEncoder::new();
        let metadata = make_metadata("cs-blank-role-parity");

        let canonical = LogRecord {
            event_name: "BlankRole".to_string(),
            body: Some(AnyValue {
                value: Some(Value::StringValue("body".to_string())),
            }),
            ..Default::default()
        };

        let common_schema = LogRecord {
            attributes: vec![
                int_attr(KEY_CSVER, CS_VERSION_4),
                string_attr(KEY_PARTB_TYPENAME, CS_LOG_TYPENAME),
                string_attr("PartA.ext_cloud_role", ""),
                string_attr("PartA.ext_cloud_roleInstance", "   "),
                string_attr("PartB.name", "BlankRole"),
                string_attr("PartB.body", "body"),
            ],
            ..Default::default()
        };

        let canonical_encoded = encode_log_batch_with_resource_attrs(
            &encoder,
            std::iter::once(&canonical),
            vec![
                string_attr("service.name", ""),
                string_attr("service.instance.id", "   "),
            ],
            &metadata,
        )
        .unwrap();
        let common_schema_encoded =
            encode_log_batch_via_proto(&encoder, std::iter::once(&common_schema), &metadata)
                .unwrap();

        assert_single_batch_equal(&canonical_encoded, &common_schema_encoded);
    }

    #[test]
    fn test_common_schema_and_canonical_records_share_batch_by_route_and_role() {
        let encoder = OtlpEncoder::new();
        let metadata = make_metadata("cs-canonical-co-batch");

        let canonical = LogRecord {
            event_name: "SharedEvent".to_string(),
            body: Some(AnyValue {
                value: Some(Value::StringValue("canonical".to_string())),
            }),
            attributes: vec![int_attr("result", 1)],
            ..Default::default()
        };

        let common_schema = LogRecord {
            attributes: vec![
                int_attr(KEY_CSVER, CS_VERSION_4),
                string_attr(KEY_PARTB_TYPENAME, CS_LOG_TYPENAME),
                string_attr("PartA.ext_cloud_role", "checkout"),
                string_attr("PartA.ext_cloud_roleInstance", "instance-1"),
                string_attr("PartB.name", "SharedEvent"),
                string_attr("PartB.body", "common-schema"),
                int_attr("PartC.result", 2),
            ],
            ..Default::default()
        };

        let encoded = encode_log_batch_with_resource_attrs(
            &encoder,
            [&canonical, &common_schema],
            vec![
                string_attr("service.name", "checkout"),
                string_attr("service.instance.id", "instance-1"),
            ],
            &metadata,
        )
        .unwrap();

        assert_eq!(encoded.len(), 1);
        assert_eq!(encoded[0].event_name, "SharedEvent");
        assert_eq!(encoded[0].row_count, 2);
        assert!(!encoded[0].metadata.schema_ids.is_empty());
    }

    #[test]
    fn test_view_empty_severity_text_matches_otlp() {
        // A log with an empty severity_text string should produce the same
        // output as one with no severity_text (proto skips empty strings).
        let encoder = OtlpEncoder::new();
        let metadata = make_metadata("view-empty-severity");

        let log_with_empty = LogRecord {
            observed_time_unix_nano: 1_700_000_000_222_333_444,
            event_name: "empty_severity".to_string(),
            severity_number: 9,
            severity_text: String::new(),
            ..Default::default()
        };
        let log_without = LogRecord {
            severity_text: String::new(),
            ..log_with_empty.clone()
        };

        let with_empty =
            encode_log_batch_via_proto(&encoder, [log_with_empty].iter(), &metadata).unwrap();
        let without =
            encode_log_batch_via_proto(&encoder, [log_without].iter(), &metadata).unwrap();

        assert_single_batch_equal(&with_empty, &without);
    }

    #[test]
    fn test_view_invalid_utf8_body_omits_body_field() {
        // A log record whose body contains invalid UTF-8 bytes should produce
        // the same encoded output as a log record with no body at all.
        use otap_df_pdata::views::otlp::bytes::logs::RawLogsData;
        use prost::Message as _;

        let encoder = OtlpEncoder::new();
        let metadata = make_metadata("view-invalid-body");

        let log = LogRecord {
            observed_time_unix_nano: 1_700_000_000_333_444_555,
            event_name: "invalid_body".to_string(),
            severity_number: 5,
            ..Default::default()
        };

        // Expected: encode a log without body
        let expected =
            encode_log_batch_via_proto(&encoder, [log.clone()].iter(), &metadata).unwrap();

        // Inject invalid UTF-8 into the body field via raw proto wire bytes.
        // body = AnyValue { string_value: [0xFF] }
        // AnyValue bytes: field 1 (string_value, LEN) → [0x0A, 0x01, 0xFF]
        // LogRecord body field (field 5, LEN, 3 bytes) → [0x2A, 0x03, 0x0A, 0x01, 0xFF]
        let mut log_bytes = log.encode_to_vec();
        log_bytes.extend_from_slice(&[0x2A, 0x03, 0x0A, 0x01, 0xFF]);
        let export_bytes = wrap_log_record_bytes(&log_bytes);

        let actual = encoder
            .encode_logs_from_view(&RawLogsData::new(&export_bytes), &metadata, None)
            .unwrap();
        assert_single_batch_equal(&expected, &actual);
    }

    #[test]
    fn test_view_invalid_utf8_severity_text_omits_field() {
        // A log record whose severity_text contains invalid UTF-8 bytes should
        // produce the same encoded output as a log record with no severity_text.
        use otap_df_pdata::views::otlp::bytes::logs::RawLogsData;
        use prost::Message as _;

        let encoder = OtlpEncoder::new();
        let metadata = make_metadata("view-invalid-severity-text");

        let log = LogRecord {
            observed_time_unix_nano: 1_700_000_000_333_444_557,
            event_name: "invalid_severity_text".to_string(),
            severity_number: 5,
            ..Default::default()
        };

        let expected =
            encode_log_batch_via_proto(&encoder, [log.clone()].iter(), &metadata).unwrap();

        // Inject severity_text = [0xFF] directly into the LogRecord wire bytes.
        // severity_text field = 3 (LEN) => [0x1A, 0x01, 0xFF]
        let mut log_bytes = log.encode_to_vec();
        log_bytes.extend_from_slice(&[0x1A, 0x01, 0xFF]);
        let export_bytes = wrap_log_record_bytes(&log_bytes);

        let actual = encoder
            .encode_logs_from_view(&RawLogsData::new(&export_bytes), &metadata, None)
            .unwrap();
        assert_single_batch_equal(&expected, &actual);
    }

    #[test]
    fn test_view_invalid_utf8_attribute_key_omits_attribute() {
        // A log record whose attribute key contains invalid UTF-8 should
        // produce the same encoded output as a log record with no attribute.
        use otap_df_pdata::views::otlp::bytes::logs::RawLogsData;
        use prost::Message as _;

        let encoder = OtlpEncoder::new();
        let metadata = make_metadata("view-invalid-attr-key");

        let log = LogRecord {
            observed_time_unix_nano: 1_700_000_000_333_444_556,
            event_name: "invalid_attr_key".to_string(),
            severity_number: 5,
            ..Default::default()
        };

        let expected =
            encode_log_batch_via_proto(&encoder, [log.clone()].iter(), &metadata).unwrap();

        // Inject attributes = [ KeyValue { key: [0xFF], value: AnyValue { string_value: "a" } } ]
        // KeyValue bytes:
        //   key   = field 1 (LEN) => [0x0A, 0x01, 0xFF]
        //   value = field 2 (LEN) => [0x12, 0x03, 0x0A, 0x01, 0x61]
        // LogRecord attributes field = field 6 (LEN) => [0x32, 0x08, ...]
        let mut log_bytes = log.encode_to_vec();
        log_bytes.extend_from_slice(&[0x32, 0x08, 0x0A, 0x01, 0xFF, 0x12, 0x03, 0x0A, 0x01, 0x61]);
        let export_bytes = wrap_log_record_bytes(&log_bytes);

        let actual = encoder
            .encode_logs_from_view(&RawLogsData::new(&export_bytes), &metadata, None)
            .unwrap();
        assert_single_batch_equal(&expected, &actual);
    }

    #[test]
    fn test_duplicate_attributes_write_once_per_field() {
        let metadata = make_metadata("duplicate-attrs");

        let base_log = LogRecord {
            observed_time_unix_nano: 1_700_000_000_444_555_666,
            event_name: "dup_attr".to_string(),
            severity_number: 7,
            ..Default::default()
        };

        let mut dup_log = base_log.clone();
        dup_log.attributes.push(KeyValue {
            key: "dup".to_string(),
            value: Some(AnyValue {
                value: Some(Value::IntValue(1)),
            }),
        });
        dup_log.attributes.push(KeyValue {
            key: "dup".to_string(),
            value: Some(AnyValue {
                value: Some(Value::IntValue(2)),
            }),
        });

        use otap_df_pdata::views::otlp::bytes::logs::RawLogRecord;
        use prost::Message as _;
        let base_bytes = base_log.encode_to_vec();
        let base_ref = RawLogRecord::new(&base_bytes);
        let (base_fields, base_dynamic_fields_start) = OtlpEncoder::determine_fields(&base_ref);
        let base_row = OtlpEncoder::write_row_data(
            &base_ref,
            &base_fields,
            base_dynamic_fields_start,
            &metadata,
        );

        let dup_bytes = dup_log.encode_to_vec();
        let dup_ref = RawLogRecord::new(&dup_bytes);
        let (dup_fields, dup_dynamic_fields_start) = OtlpEncoder::determine_fields(&dup_ref);
        let dup_row =
            OtlpEncoder::write_row_data(&dup_ref, &dup_fields, dup_dynamic_fields_start, &metadata);

        assert_eq!(dup_fields.len() - dup_dynamic_fields_start, 2);
        assert_eq!(dup_row.len() - base_row.len(), 16);
    }

    #[test]
    fn test_same_event_name_multiple_schemas() {
        let encoder = OtlpEncoder::new();

        // Schema 1: Basic log
        let log1 = LogRecord {
            event_name: "user_action".to_string(),
            severity_number: 9,
            ..Default::default()
        };

        // Schema 2: With trace_id
        let mut log2 = LogRecord {
            event_name: "user_action".to_string(),
            severity_number: 10,
            ..Default::default()
        };
        log2.trace_id = vec![1; 16];

        // Schema 3: With attributes
        let mut log3 = LogRecord {
            event_name: "user_action".to_string(),
            severity_number: 11,
            ..Default::default()
        };
        log3.attributes.push(KeyValue {
            key: "user_id".to_string(),
            value: Some(AnyValue {
                value: Some(Value::StringValue("user123".to_string())),
            }),
        });

        let metadata = make_metadata("test");
        let result =
            encode_log_batch_via_proto(&encoder, [log1, log2, log3].iter(), &metadata).unwrap();

        // All should be in one batch with same event_name
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].event_name, "user_action");
        assert!(!result[0].data.is_empty());
        // Should have 3 different schema IDs (semicolon-separated)
        assert_eq!(result[0].metadata.schema_ids.matches(';').count(), 2); // 3 schemas = 2 semicolons
    }

    #[test]
    fn test_different_event_names() {
        let encoder = OtlpEncoder::new();

        let log1 = LogRecord {
            event_name: "login".to_string(),
            severity_number: 9,
            ..Default::default()
        };

        let log2 = LogRecord {
            event_name: "logout".to_string(),
            severity_number: 10,
            ..Default::default()
        };

        let metadata = make_metadata("test");
        let result = encode_log_batch_via_proto(&encoder, [log1, log2].iter(), &metadata).unwrap();

        // Should create 2 separate batches
        assert_eq!(result.len(), 2);

        let event_names: Vec<&String> = result.iter().map(|batch| &batch.event_name).collect();
        assert!(event_names.contains(&&"login".to_string()));
        assert!(event_names.contains(&&"logout".to_string()));

        assert!(result.iter().all(|batch| !batch.data.is_empty()));
    }

    #[test]
    fn test_empty_event_name_defaults_to_log() {
        let encoder = OtlpEncoder::new();

        let log = LogRecord {
            event_name: "".to_string(),
            severity_number: 9,
            ..Default::default()
        };

        let metadata = make_metadata("test");
        let result = encode_log_batch_via_proto(&encoder, [log].iter(), &metadata).unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].event_name, "Log"); // Should default to "Log"
        assert!(!result[0].data.is_empty());
    }

    #[test]
    fn test_mixed_scenario() {
        let encoder = OtlpEncoder::new();

        // event_name1 with schema1
        let log1 = LogRecord {
            event_name: "user_action".to_string(),
            severity_number: 9,
            ..Default::default()
        };

        // event_name1 with schema2 (different schema, same event)
        let mut log2 = LogRecord {
            event_name: "user_action".to_string(),
            severity_number: 10,
            ..Default::default()
        };
        log2.trace_id = vec![1; 16];

        // event_name2 with schema3
        let log3 = LogRecord {
            event_name: "system_alert".to_string(),
            severity_number: 11,
            ..Default::default()
        };

        // empty event_name (defaults to "Log") with schema4
        let mut log4 = LogRecord {
            event_name: "".to_string(),
            severity_number: 12,
            ..Default::default()
        };
        log4.attributes.push(KeyValue {
            key: "error_code".to_string(),
            value: Some(AnyValue {
                value: Some(Value::IntValue(404)),
            }),
        });

        let metadata = make_metadata("test");
        let result =
            encode_log_batch_via_proto(&encoder, [log1, log2, log3, log4].iter(), &metadata)
                .unwrap();

        // Should create 3 batches: "user_action", "system_alert", "Log"
        assert_eq!(result.len(), 3);

        let user_action = result
            .iter()
            .find(|batch| batch.event_name == "user_action")
            .unwrap();
        let system_alert = result
            .iter()
            .find(|batch| batch.event_name == "system_alert")
            .unwrap();
        let log_batch = result
            .iter()
            .find(|batch| batch.event_name == "Log")
            .unwrap();

        assert!(!user_action.data.is_empty());
        assert_eq!(user_action.metadata.schema_ids.matches(';').count(), 1); // 2 schemas = 1 semicolon
        assert!(!system_alert.data.is_empty());
        assert_eq!(system_alert.metadata.schema_ids.matches(';').count(), 0); // 1 schema = 0 semicolons
        assert!(!log_batch.data.is_empty());
        assert_eq!(log_batch.metadata.schema_ids.matches(';').count(), 0); // 1 schema = 0 semicolons
    }

    #[test]
    fn test_span_encoding() {
        let encoder = OtlpEncoder::new();

        let mut span = Span {
            trace_id: vec![1; 16],
            span_id: vec![2; 8],
            parent_span_id: vec![3; 8],
            name: "test_span".to_string(),
            kind: 1,
            start_time_unix_nano: 1_700_000_000_000_000_000,
            end_time_unix_nano: 1_700_000_001_000_000_000,
            flags: 1,
            trace_state: "key=value".to_string(),
            ..Default::default()
        };

        span.attributes.push(KeyValue {
            key: "http.method".to_string(),
            value: Some(AnyValue {
                value: Some(Value::StringValue("GET".to_string())),
            }),
        });

        span.attributes.push(KeyValue {
            key: "http.status_code".to_string(),
            value: Some(AnyValue {
                value: Some(Value::IntValue(200)),
            }),
        });

        let metadata = make_metadata("testNamespace");
        let result = encoder
            .encode_span_batch([span].iter(), &metadata, None)
            .unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].event_name, "Span");
        assert!(!result[0].data.is_empty());
    }

    #[test]
    fn test_span_with_links() {
        use opentelemetry_proto::tonic::trace::v1::span::Link;

        let encoder = OtlpEncoder::new();

        let mut span = Span {
            trace_id: vec![1; 16],
            span_id: vec![2; 8],
            name: "linked_span".to_string(),
            kind: 2,
            start_time_unix_nano: 1_700_000_000_000_000_000,
            end_time_unix_nano: 1_700_000_001_000_000_000,
            ..Default::default()
        };

        span.links.push(Link {
            trace_id: vec![4; 16],
            span_id: vec![5; 8],
            ..Default::default()
        });

        span.links.push(Link {
            trace_id: vec![6; 16],
            span_id: vec![7; 8],
            ..Default::default()
        });

        let metadata = make_metadata("test");
        let result = encoder
            .encode_span_batch([span].iter(), &metadata, None)
            .unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].event_name, "Span");
        assert!(!result[0].data.is_empty());
    }

    #[test]
    fn test_span_with_status() {
        use opentelemetry_proto::tonic::trace::v1::{status::StatusCode, Status};

        let encoder = OtlpEncoder::new();

        let mut span = Span {
            trace_id: vec![1; 16],
            span_id: vec![2; 8],
            name: "error_span".to_string(),
            kind: 1,
            start_time_unix_nano: 1_700_000_000_000_000_000,
            end_time_unix_nano: 1_700_000_001_000_000_000,
            ..Default::default()
        };

        span.status = Some(Status {
            message: "Something went wrong".to_string(),
            code: StatusCode::Error as i32,
        });

        let metadata = make_metadata("test");
        let result = encoder
            .encode_span_batch([span].iter(), &metadata, None)
            .unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].event_name, "Span");
        assert!(!result[0].data.is_empty());
    }

    #[test]
    fn test_multiple_spans_same_name() {
        let encoder = OtlpEncoder::new();

        let span1 = Span {
            trace_id: vec![1; 16],
            span_id: vec![2; 8],
            name: "database_query".to_string(),
            kind: 3,
            start_time_unix_nano: 1_700_000_000_000_000_000,
            end_time_unix_nano: 1_700_000_001_000_000_000,
            ..Default::default()
        };

        let span2 = Span {
            trace_id: vec![3; 16],
            span_id: vec![4; 8],
            name: "database_query".to_string(),
            kind: 3,
            start_time_unix_nano: 1_700_000_002_000_000_000,
            end_time_unix_nano: 1_700_000_003_000_000_000,
            ..Default::default()
        };

        let fields1 = OtlpEncoder::determine_span_fields(&span1, "Span", None);
        assert!(
            fields1.iter().any(|f| f.name.as_ref() == FIELD_NAME),
            "Span with non-empty name should include 'name' field in schema"
        );

        let fields2 = OtlpEncoder::determine_span_fields(&span2, "Span", None);
        assert!(
            fields2.iter().any(|f| f.name.as_ref() == FIELD_NAME),
            "Span with non-empty name should include 'name' field in schema"
        );

        let metadata = make_metadata("test");
        let result = encoder
            .encode_span_batch([span1, span2].iter(), &metadata, None)
            .unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].event_name, "Span");
        assert!(!result[0].data.is_empty());
        assert_eq!(result[0].metadata.schema_ids.matches(';').count(), 0); // 1 schema = 0 semicolons
    }

    #[test]
    fn test_optimized_links_serialization() {
        use opentelemetry_proto::tonic::trace::v1::span::Link;

        let links = vec![
            Link {
                trace_id: vec![
                    0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89,
                    0xab, 0xcd, 0xef,
                ],
                span_id: vec![0xfe, 0xdc, 0xba, 0x98, 0x76, 0x54, 0x32, 0x10],
                ..Default::default()
            },
            Link {
                trace_id: vec![
                    0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd,
                    0xee, 0xff, 0x00,
                ],
                span_id: vec![0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77],
                ..Default::default()
            },
        ];

        let result = OtlpEncoder::serialize_links(&links);

        assert!(result.starts_with('['));
        assert!(result.ends_with(']'));
        assert!(result.contains("toSpanId"));
        assert!(result.contains("toTraceId"));
        assert!(result.contains("fedcba9876543210"));
        assert!(result.contains("0123456789abcdef0123456789abcdef"));
        assert!(result.contains("0011223344556677"));
        assert!(result.contains("112233445566778899aabbccddeeff00"));

        let empty_result = OtlpEncoder::serialize_links(&[]);
        assert_eq!(empty_result, "[]");

        let single_link = vec![Link {
            trace_id: vec![0x12; 16],
            span_id: vec![0x34; 8],
            ..Default::default()
        }];
        let single_result = OtlpEncoder::serialize_links(&single_link);
        assert!(single_result.contains("3434343434343434"));
        assert!(single_result.contains("12121212121212121212121212121212"));
        assert_eq!(single_result.matches(',').count(), 1);
        assert!(single_result.starts_with('['));
        assert!(single_result.ends_with(']'));
    }

    #[test]
    fn test_row_count_in_encoded_batch() {
        let encoder = OtlpEncoder::new();

        let logs = [
            LogRecord {
                observed_time_unix_nano: 1_700_000_000_000_000_000,
                event_name: "test_event".to_string(),
                severity_number: 9,
                ..Default::default()
            },
            LogRecord {
                observed_time_unix_nano: 1_700_000_001_000_000_000,
                event_name: "test_event".to_string(),
                severity_number: 10,
                ..Default::default()
            },
            LogRecord {
                observed_time_unix_nano: 1_700_000_002_000_000_000,
                event_name: "test_event".to_string(),
                severity_number: 11,
                ..Default::default()
            },
        ];

        let metadata = make_metadata("test");
        let result = encode_log_batch_via_proto(&encoder, logs.iter(), &metadata).unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].row_count, 3);

        let spans = [
            Span {
                start_time_unix_nano: 1_700_000_000_000_000_000,
                end_time_unix_nano: 1_700_000_001_000_000_000,
                ..Default::default()
            },
            Span {
                start_time_unix_nano: 1_700_000_002_000_000_000,
                end_time_unix_nano: 1_700_000_003_000_000_000,
                ..Default::default()
            },
        ];

        let span_result = encoder
            .encode_span_batch(spans.iter(), &metadata, None)
            .unwrap();

        assert_eq!(span_result.len(), 1);
        assert_eq!(span_result[0].row_count, 2);
    }

    #[test]
    fn test_view_timestamp_priority() {
        // When both time_unix_nano and observed_time_unix_nano are non-zero,
        // the view adapter must prefer time_unix_nano, matching the OTLP path.
        let encoder = OtlpEncoder::new();
        let metadata = make_metadata("ts-priority");

        let log = LogRecord {
            time_unix_nano: 1_000_000_000,
            observed_time_unix_nano: 2_000_000_000,
            event_name: "ts_event".to_string(),
            severity_number: 9,
            ..Default::default()
        };

        let encoded =
            encode_log_batch_via_proto(&encoder, [log.clone()].iter(), &metadata).unwrap();

        // Confirm the selected timestamp is time_unix_nano, not observed_time_unix_nano
        assert_eq!(encoded[0].metadata.start_time, 1_000_000_000);
        assert_eq!(encoded[0].metadata.end_time, 1_000_000_000);
    }

    #[test]
    fn test_view_zero_timestamp_does_not_pin_batch_start_time() {
        let encoder = OtlpEncoder::new();
        let metadata = make_metadata("zero-ts-start");
        use prost::Message as _;

        let bytes = opentelemetry_proto::tonic::collector::logs::v1::ExportLogsServiceRequest {
            resource_logs: vec![ResourceLogs {
                scope_logs: vec![ScopeLogs {
                    log_records: vec![
                        LogRecord {
                            event_name: "ts_event".to_string(),
                            severity_number: 9,
                            ..Default::default()
                        },
                        LogRecord {
                            observed_time_unix_nano: 2_000_000_000,
                            event_name: "ts_event".to_string(),
                            severity_number: 9,
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                }],
                ..Default::default()
            }],
        }
        .encode_to_vec();

        use otap_df_pdata::views::otlp::bytes::logs::RawLogsData;
        let encoded = encoder
            .encode_logs_from_view(&RawLogsData::new(&bytes), &metadata, None)
            .unwrap();

        assert_eq!(encoded.len(), 1);
        assert_eq!(encoded[0].metadata.start_time, 2_000_000_000);
        assert_eq!(encoded[0].metadata.end_time, 2_000_000_000);
    }

    #[test]
    fn test_view_multi_resource_multi_scope() {
        // encode_logs_from_view must accumulate records across all resources and scopes.
        let encoder = OtlpEncoder::new();
        let metadata = make_metadata("multi-res");

        // Two resources, each with two scopes, each scope with one log record.
        // Records alternate between two event names to verify per-event batching.
        use otap_df_pdata::views::otlp::bytes::logs::RawLogsData;
        use prost::Message as _;
        let bytes = opentelemetry_proto::tonic::collector::logs::v1::ExportLogsServiceRequest {
            resource_logs: vec![
                ResourceLogs {
                    scope_logs: vec![
                        ScopeLogs {
                            log_records: vec![LogRecord {
                                observed_time_unix_nano: 1_000,
                                event_name: "alpha".to_string(),
                                severity_number: 9,
                                ..Default::default()
                            }],
                            ..Default::default()
                        },
                        ScopeLogs {
                            log_records: vec![LogRecord {
                                observed_time_unix_nano: 2_000,
                                event_name: "beta".to_string(),
                                severity_number: 10,
                                ..Default::default()
                            }],
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                },
                ResourceLogs {
                    scope_logs: vec![
                        ScopeLogs {
                            log_records: vec![LogRecord {
                                observed_time_unix_nano: 3_000,
                                event_name: "alpha".to_string(),
                                severity_number: 11,
                                ..Default::default()
                            }],
                            ..Default::default()
                        },
                        ScopeLogs {
                            log_records: vec![LogRecord {
                                observed_time_unix_nano: 4_000,
                                event_name: "beta".to_string(),
                                severity_number: 12,
                                ..Default::default()
                            }],
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                },
            ],
        }
        .encode_to_vec();

        let result = encoder
            .encode_logs_from_view(&RawLogsData::new(&bytes), &metadata, None)
            .unwrap();

        // Two event names → two batches
        assert_eq!(result.len(), 2);

        let alpha = result.iter().find(|b| b.event_name == "alpha").unwrap();
        let beta = result.iter().find(|b| b.event_name == "beta").unwrap();

        // Each batch should contain records from both resources
        assert_eq!(alpha.row_count, 2);
        assert_eq!(beta.row_count, 2);

        // Timestamp ranges should span across resources
        assert_eq!(alpha.metadata.start_time, 1_000);
        assert_eq!(alpha.metadata.end_time, 3_000);
        assert_eq!(beta.metadata.start_time, 2_000);
        assert_eq!(beta.metadata.end_time, 4_000);
    }
}
