# Changelog

## vNext

- Add `links` field to Part B (serialized as JSON array of `{toTraceId, toSpanId}`)
- Add `statusMessage` field to Part B for error spans with descriptions
- Add `rpcSystem` and `rpcGrpcStatusCode` to well-known attribute mappings
- Update well-known attribute keys to stable semantic conventions
  (`db.system.name`, `db.namespace`, `db.query.text`, `messaging.destination.name`)
- Fix `PartA.time` and `PartB.startTime` to use UTC ISO 8601 with trailing `Z`
  instead of `+00:00`

## v0.5.0

Released 2026-May-13

- Bump opentelemetry and opentelemetry_sdk versions to 0.32
- Bump eventheader and eventheader_dynamic versions to 0.5.0
- **Breaking** `UserEventsSpanExporter::shutdown` now takes `&self` instead of
  `&mut self`, matching the upstream `SpanExporter` trait change in 0.32.

## v0.4.0

- Bump opentelemetry and opentelemetry_sdk versions to 0.31

## v0.3.0

Released 2025-July-24

- Add support for RoleName,RoleInstance population from Resource.
- Add mapping of well-known (OTel Semantic Conventions) attributes to PartB.
- Only export parentId field when parent span ID is valid (not INVALID).

## v0.2.0

Released 2025-May-27

- Bump opentelemetry and opentelemetry_sdk versions to 0.30

## v0.1.0

### Added

- Initial Alpha implementation
