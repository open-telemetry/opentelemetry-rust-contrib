/*
 * opentelemetry_c/trace_exporter.h
 *
 * The generic **trace exporter** handle (`otel_trace_exporter_t`) — the opaque object a span
 * processor builder consumes. A concrete exporter is produced by an exporter builder (today
 * only the OTLP HTTP/protobuf exporter; see otlp_trace_exporter.h). This header is the stable
 * extension point: additional exporter kinds can be added later behind the same opaque handle
 * without breaking the ABI. No custom-callback exporter is provided yet.
 *
 * Ownership: a trace exporter is owned by the caller until it is transferred into a span
 * processor builder via otel_batch_span_processor_builder_set_exporter() (ownership moves on
 * OTEL_STATUS_OK). If never transferred, release it with otel_trace_exporter_destroy().
 *
 * Part of `libopentelemetry_c_sdk`. Requires linking the SDK alongside the API.
 */
#ifndef OPENTELEMETRY_C_TRACE_EXPORTER_H
#define OPENTELEMETRY_C_TRACE_EXPORTER_H

#ifdef __cplusplus
extern "C" {
#endif

/* Opaque trace-exporter handle. */
typedef struct otel_trace_exporter_t otel_trace_exporter_t;

/*
 * Destroy a trace-exporter handle (no-op on NULL). Do NOT call this on an exporter that was
 * successfully transferred into a span processor builder (that builder owns it now). Must not
 * race with any other use of the same handle.
 */
void otel_trace_exporter_destroy(otel_trace_exporter_t* exporter);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* OPENTELEMETRY_C_TRACE_EXPORTER_H */
