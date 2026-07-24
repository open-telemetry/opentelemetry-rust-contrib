// Copyright The OpenTelemetry Authors
// SPDX-License-Identifier: Apache-2.0

//! Shared event-name routing for logs and spans.
//!
//! This module owns the public routing configuration types
//! ([`LogsEventNameMapping`], [`SpanEventNameMapping`], and their routing keys)
//! together with both resolution paths: one over the pdata `LogRecordView`
//! traits (logs) and one over the tonic proto `Span` types (spans). Keeping the
//! two implementations side by side makes the shared lookup semantics explicit
//! and reduces the chance of the paths drifting.

use std::borrow::Cow;
use std::collections::HashMap;
use tracing::debug;

use opentelemetry_proto::tonic::common::v1::{
    any_value::Value as ProtoAnyValue, InstrumentationScope, KeyValue,
};
use opentelemetry_proto::tonic::resource::v1::Resource;
use opentelemetry_proto::tonic::trace::v1::Span;
use otap_df_pdata_views::views::common::{
    AnyValueView, AttributeView, InstrumentationScopeView, ValueType,
};
use otap_df_pdata_views::views::logs::LogRecordView;
use otap_df_pdata_views::views::resource::ResourceView;

/// Routing key used to resolve per-record destination event names for logs.
#[derive(Clone, Debug)]
pub enum LogsEventNameRoutingKey {
    EventName,
    ResourceAttribute(String),
    ScopeAttribute(String),
    LogRecordAttribute(String),
}

impl LogsEventNameRoutingKey {
    /// Attribute/scope key name for attribute-based routing kinds; `None` for `EventName`.
    fn attribute_key(&self) -> Option<&str> {
        match self {
            LogsEventNameRoutingKey::EventName => None,
            LogsEventNameRoutingKey::ResourceAttribute(key)
            | LogsEventNameRoutingKey::ScopeAttribute(key)
            | LogsEventNameRoutingKey::LogRecordAttribute(key) => Some(key.as_str()),
        }
    }
}

/// Optional logs routing config. When configured, each log record can be routed
/// to a destination event based on `routing_key` and `events` lookup.
#[derive(Clone, Debug)]
pub struct LogsEventNameMapping {
    pub routing_key: LogsEventNameRoutingKey,
    pub events: HashMap<String, Option<String>>,
}

impl LogsEventNameMapping {
    /// Rejects mappings that can never route: empty `events`, blank source keys,
    /// or a blank attribute routing-key name.
    pub(crate) fn validate(&self) -> Result<(), String> {
        validate_event_mapping(
            "LogsConfig.event_name_mapping",
            self.routing_key.attribute_key(),
            &self.events,
        )
    }
}

/// Routing key used to resolve per-record destination event names for spans.
#[derive(Clone, Debug)]
pub enum SpanEventNameRoutingKey {
    ResourceAttribute(String),
    ScopeAttribute(String),
    SpanAttribute(String),
}

impl SpanEventNameRoutingKey {
    /// Attribute/scope key name for the (always attribute-based) span routing kinds.
    fn attribute_key(&self) -> Option<&str> {
        match self {
            SpanEventNameRoutingKey::ResourceAttribute(key)
            | SpanEventNameRoutingKey::ScopeAttribute(key)
            | SpanEventNameRoutingKey::SpanAttribute(key) => Some(key.as_str()),
        }
    }
}

/// Optional span routing config. When configured, each span can be routed
/// to a destination event based on `routing_key` and `events` lookup.
#[derive(Clone, Debug)]
pub struct SpanEventNameMapping {
    pub routing_key: SpanEventNameRoutingKey,
    pub events: HashMap<String, Option<String>>,
}

impl SpanEventNameMapping {
    /// Rejects mappings that can never route: empty `events`, blank source keys,
    /// or a blank attribute routing-key name.
    pub(crate) fn validate(&self) -> Result<(), String> {
        validate_event_mapping(
            "TracesConfig.event_name_mapping",
            self.routing_key.attribute_key(),
            &self.events,
        )
    }
}

/// Shared validation for logs/spans routing mappings. `routing_key_name` is the
/// attribute/scope key for attribute-based kinds, or `None` for event-name routing.
fn validate_event_mapping(
    ctx: &str,
    routing_key_name: Option<&str>,
    events: &HashMap<String, Option<String>>,
) -> Result<(), String> {
    if events.is_empty() {
        return Err(format!(
            "{ctx}.events must be non-empty when routing is configured"
        ));
    }
    if let Some(name) = routing_key_name {
        if name.trim().is_empty() {
            return Err(format!(
                "{ctx}.routing_key attribute name must not be blank"
            ));
        }
    }
    if events.keys().any(|key| key.trim().is_empty()) {
        return Err(format!("{ctx}.events source keys must not be blank"));
    }
    Ok(())
}

fn non_blank_utf8(bytes: &[u8]) -> Option<&str> {
    let s = std::str::from_utf8(bytes).ok()?;
    (!s.trim().is_empty()).then_some(s)
}

pub(crate) fn normalized_event_name(record: &impl LogRecordView) -> Option<&str> {
    record.event_name().and_then(non_blank_utf8)
}

/// Reserved routing key that selects the instrumentation scope name.
pub(crate) const SCOPE_NAME_ROUTING_KEY: &str = "scope.name";
/// Reserved routing key that selects the instrumentation scope version.
pub(crate) const SCOPE_VERSION_ROUTING_KEY: &str = "scope.version";

/// Resolves a routing `source_value` against a mapping's `events` table.
///
/// Returns:
/// - `Some(destination)` when the source value has a non-empty destination
///   (borrowed from the `events` table, so the common lookup allocates nothing),
/// - `Some(source_value)` when the source value maps to `None`/empty (passthrough),
/// - `None` when there is no entry for the source value (caller falls back to the
///   configured default/table name).
///
/// Shared by logs and spans routing to keep the lookup semantics identical.
pub(crate) fn resolve_mapped_destination<'a>(
    events: &'a HashMap<String, Option<String>>,
    source_value: &str,
) -> Option<Cow<'a, str>> {
    let mapped = events.get(source_value)?;
    match mapped.as_deref() {
        Some(destination) if !destination.trim().is_empty() => Some(Cow::Borrowed(destination)),
        _ => Some(Cow::Owned(source_value.to_string())),
    }
}

/// A primitive attribute value eligible for event-name routing. Non-primitive
/// kinds (bytes/array/kvlist) are not routable.
pub(crate) enum RoutingPrimitive<'a> {
    Str(&'a str),
    Int(i64),
    Double(f64),
    Bool(bool),
}

/// Stringify a routing primitive using the shared routing rules: string values
/// are treated as absent when blank/whitespace and otherwise passed through;
/// numeric/bool values are formatted via `to_string`.
///
/// Kept in one place so the logs path (over pdata-view traits) and the spans
/// path (over proto types) can't drift on value handling.
pub(crate) fn stringify_routing_primitive(value: RoutingPrimitive<'_>) -> Option<String> {
    match value {
        RoutingPrimitive::Str(s) => (!s.trim().is_empty()).then(|| s.to_string()),
        RoutingPrimitive::Int(v) => Some(v.to_string()),
        RoutingPrimitive::Double(v) => Some(v.to_string()),
        RoutingPrimitive::Bool(v) => Some(v.to_string()),
    }
}

// ---------------------------------------------------------------------------
// Log routing (over the pdata `LogRecordView` traits)
// ---------------------------------------------------------------------------

fn routing_value_from_attributes<'a, A, I>(attributes: I, key: &str) -> Option<String>
where
    A: AttributeView + 'a,
    I: IntoIterator<Item = A>,
{
    for attr in attributes {
        let Ok(attr_key) = std::str::from_utf8(attr.key()) else {
            continue;
        };
        if attr_key != key {
            continue;
        }
        let value = attr.value()?;

        return match value.value_type() {
            ValueType::String => value
                .as_string()
                .and_then(|b| std::str::from_utf8(b).ok())
                .and_then(|s| stringify_routing_primitive(RoutingPrimitive::Str(s))),
            ValueType::Int64 => value
                .as_int64()
                .and_then(|v| stringify_routing_primitive(RoutingPrimitive::Int(v))),
            ValueType::Double => value
                .as_double()
                .and_then(|v| stringify_routing_primitive(RoutingPrimitive::Double(v))),
            ValueType::Bool => value
                .as_bool()
                .and_then(|v| stringify_routing_primitive(RoutingPrimitive::Bool(v))),
            _ => None,
        };
    }
    None
}

fn routing_value_from_scope<SV>(scope: &SV, key: &str) -> Option<String>
where
    SV: InstrumentationScopeView,
{
    match key {
        SCOPE_NAME_ROUTING_KEY => scope
            .name()
            .and_then(|b| std::str::from_utf8(b).ok())
            .and_then(|s| stringify_routing_primitive(RoutingPrimitive::Str(s))),
        SCOPE_VERSION_ROUTING_KEY => scope
            .version()
            .and_then(|b| std::str::from_utf8(b).ok())
            .and_then(|s| stringify_routing_primitive(RoutingPrimitive::Str(s))),
        _ => routing_value_from_attributes(scope.attributes(), key),
    }
}

/// Log routing resolved once per instrumentation scope.
pub(crate) enum LogScopeRouting<'a> {
    /// No routing mapping configured; records use the configured table name.
    None,
    /// The routed event name is constant for every record in the scope (a
    /// resource-/scope-level routing key). Resolved once.
    Fixed(String),
    /// Routing depends on each record (`EventName` or `LogRecordAttribute`).
    PerRecord(&'a LogsEventNameMapping),
}

/// Resolve a mapped source value to the final event name, falling back to the
/// configured table name when the source is absent or unmapped.
fn resolve_log_event_name_from_value<'a>(
    mapping: &'a LogsEventNameMapping,
    source_value: Option<&str>,
    table_name: &'a str,
) -> Cow<'a, str> {
    if let Some(source_value) = source_value {
        if let Some(mapped_event_name) = resolve_mapped_destination(&mapping.events, source_value) {
            debug!(
                name: "otlp_encoder.log_event_routing",
                target: "geneva-uploader",
                source_value = %source_value,
                routed_event_name = %mapped_event_name,
                "Resolved log event routing from mapping"
            );
            return mapped_event_name;
        }

        debug!(
            name: "otlp_encoder.log_event_routing",
            target: "geneva-uploader",
            source_value = %source_value,
            fallback_event_name = %table_name,
            "No mapping entry for routing source; using fallback event name"
        );
        return Cow::Borrowed(table_name);
    }

    debug!(
        name: "otlp_encoder.log_event_routing",
        target: "geneva-uploader",
        fallback_event_name = %table_name,
        "Routing source value not found on log record; using fallback event name"
    );
    Cow::Borrowed(table_name)
}

/// Resolve the scope-invariant part of log routing exactly once per scope.
///
/// Resource-/scope-level routing keys resolve to a value that is constant across
/// every record in the scope, so their destination event name is computed here
/// and reused. `EventName`/`LogRecordAttribute` routing is deferred per record.
pub(crate) fn resolve_log_scope_routing<'a, RV, SV>(
    resource: Option<&RV>,
    scope: Option<&SV>,
    table_name: &str,
    event_name_mapping: Option<&'a LogsEventNameMapping>,
) -> LogScopeRouting<'a>
where
    RV: ResourceView,
    SV: InstrumentationScopeView,
{
    let Some(mapping) = event_name_mapping else {
        return LogScopeRouting::None;
    };

    match &mapping.routing_key {
        LogsEventNameRoutingKey::ResourceAttribute(key) => {
            let value = resource
                .and_then(|res| routing_value_from_attributes(res.attributes(), key.as_str()));
            LogScopeRouting::Fixed(
                resolve_log_event_name_from_value(mapping, value.as_deref(), table_name)
                    .into_owned(),
            )
        }
        LogsEventNameRoutingKey::ScopeAttribute(key) => {
            let value = scope.and_then(|scope| routing_value_from_scope(scope, key.as_str()));
            LogScopeRouting::Fixed(
                resolve_log_event_name_from_value(mapping, value.as_deref(), table_name)
                    .into_owned(),
            )
        }
        LogsEventNameRoutingKey::EventName | LogsEventNameRoutingKey::LogRecordAttribute(_) => {
            LogScopeRouting::PerRecord(mapping)
        }
    }
}

/// Resolve the routed event name for a single record whose scope routing depends
/// on per-record data (`EventName` or `LogRecordAttribute`).
pub(crate) fn resolve_log_record_routing_event_name<'a, R>(
    record: &R,
    mapping: &'a LogsEventNameMapping,
    table_name: &'a str,
) -> Cow<'a, str>
where
    R: LogRecordView,
{
    let routing_value = match &mapping.routing_key {
        LogsEventNameRoutingKey::EventName => normalized_event_name(record).map(str::to_owned),
        LogsEventNameRoutingKey::LogRecordAttribute(key) => {
            routing_value_from_attributes(record.attributes(), key.as_str())
        }
        // Resource-/scope-level keys are precomputed in `resolve_log_scope_routing`.
        LogsEventNameRoutingKey::ResourceAttribute(_)
        | LogsEventNameRoutingKey::ScopeAttribute(_) => None,
    };

    resolve_log_event_name_from_value(mapping, routing_value.as_deref(), table_name)
}

// ---------------------------------------------------------------------------
// Span routing (over the tonic proto `Span` types)
// ---------------------------------------------------------------------------

pub(crate) fn span_routing_value_from_attributes(
    attributes: &[KeyValue],
    key: &str,
) -> Option<String> {
    for attr in attributes {
        if attr.key != key {
            continue;
        }
        let value = attr.value.as_ref()?.value.as_ref()?;
        return match value {
            ProtoAnyValue::StringValue(value) => {
                stringify_routing_primitive(RoutingPrimitive::Str(value))
            }
            ProtoAnyValue::IntValue(value) => {
                stringify_routing_primitive(RoutingPrimitive::Int(*value))
            }
            ProtoAnyValue::DoubleValue(value) => {
                stringify_routing_primitive(RoutingPrimitive::Double(*value))
            }
            ProtoAnyValue::BoolValue(value) => {
                stringify_routing_primitive(RoutingPrimitive::Bool(*value))
            }
            _ => None,
        };
    }
    None
}

pub(crate) fn span_routing_value_from_scope(
    scope: Option<&InstrumentationScope>,
    key: &str,
) -> Option<String> {
    let scope = scope?;
    match key {
        SCOPE_NAME_ROUTING_KEY => stringify_routing_primitive(RoutingPrimitive::Str(&scope.name)),
        SCOPE_VERSION_ROUTING_KEY => {
            stringify_routing_primitive(RoutingPrimitive::Str(&scope.version))
        }
        _ => span_routing_value_from_attributes(&scope.attributes, key),
    }
}

pub(crate) fn span_routing_value_from_resource(
    resource: Option<&Resource>,
    key: &str,
) -> Option<String> {
    resource.and_then(|resource| span_routing_value_from_attributes(&resource.attributes, key))
}

/// Span routing resolved once per instrumentation scope.
pub(crate) enum SpanScopeRouting<'a> {
    /// The routed event name is constant for every span in the scope (no mapping
    /// configured, or a resource-/scope-level routing key). Resolved once.
    Fixed(String),
    /// Routing depends on each span's own attributes; only `SpanAttribute` keys
    /// need per-span resolution.
    PerSpan {
        mapping: &'a SpanEventNameMapping,
        key: &'a str,
    },
}

/// Resolve the scope-invariant part of span routing exactly once per scope.
///
/// For resource-/scope-level routing keys the source value is constant across
/// every span in the scope, so the destination event name is computed here and
/// reused; only `SpanAttribute` routing is deferred to per-span resolution.
pub(crate) fn resolve_span_scope_routing<'a>(
    resource: Option<&Resource>,
    scope: Option<&InstrumentationScope>,
    table_name: &str,
    event_name_mapping: Option<&'a SpanEventNameMapping>,
) -> SpanScopeRouting<'a> {
    let Some(mapping) = event_name_mapping else {
        return SpanScopeRouting::Fixed(table_name.to_string());
    };

    match &mapping.routing_key {
        SpanEventNameRoutingKey::ResourceAttribute(key) => SpanScopeRouting::Fixed(
            span_routing_value_from_resource(resource, key)
                .and_then(|value| resolve_mapped_destination(&mapping.events, &value))
                .map(Cow::into_owned)
                .unwrap_or_else(|| table_name.to_string()),
        ),
        SpanEventNameRoutingKey::ScopeAttribute(key) => SpanScopeRouting::Fixed(
            span_routing_value_from_scope(scope, key)
                .and_then(|value| resolve_mapped_destination(&mapping.events, &value))
                .map(Cow::into_owned)
                .unwrap_or_else(|| table_name.to_string()),
        ),
        SpanEventNameRoutingKey::SpanAttribute(key) => SpanScopeRouting::PerSpan { mapping, key },
    }
}

/// Resolve the routed event name for a single span given its scope's routing.
///
/// Returns a [`Cow`] so the common paths (a resource-/scope-level `Fixed` name,
/// or the no-mapping default) borrow the scope-invariant name instead of
/// allocating per span; only a per-span mapping hit owns a new `String`.
pub(crate) fn resolve_span_event_name_in_scope<'a>(
    scope_routing: &'a SpanScopeRouting<'a>,
    span: &Span,
    table_name: &'a str,
) -> Cow<'a, str> {
    match scope_routing {
        SpanScopeRouting::Fixed(name) => Cow::Borrowed(name.as_str()),
        SpanScopeRouting::PerSpan { mapping, key } => {
            match span_routing_value_from_attributes(&span.attributes, key)
                .and_then(|value| resolve_mapped_destination(&mapping.events, &value))
            {
                Some(name) => name,
                None => Cow::Borrowed(table_name),
            }
        }
    }
}

/// Resolve span routing for a single span end-to-end. Retained for unit tests;
/// the hot path precomputes scope routing via [`resolve_span_scope_routing`].
#[cfg(test)]
pub(crate) fn resolve_span_event_name(
    resource: Option<&Resource>,
    scope: Option<&InstrumentationScope>,
    span: &Span,
    table_name: &str,
    event_name_mapping: Option<&SpanEventNameMapping>,
) -> String {
    let scope_routing = resolve_span_scope_routing(resource, scope, table_name, event_name_mapping);
    resolve_span_event_name_in_scope(&scope_routing, span, table_name).into_owned()
}
