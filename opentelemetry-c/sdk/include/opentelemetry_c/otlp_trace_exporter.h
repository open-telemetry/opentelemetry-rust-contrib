/*
 * opentelemetry_c/otlp_trace_exporter.h
 *
 * Builder for the OTLP **HTTP/protobuf** trace exporter, the one concrete trace exporter
 * currently implemented. It produces a generic otel_trace_exporter_t (see trace_exporter.h)
 * that a span processor builder then consumes.
 *
 * The exporter owns its own blocking HTTP client, so no user-managed async runtime is
 * required. HTTPS is available via a selectable TLS backend chosen at compile time with the
 * crate's cargo features: `native-tls` (default; the platform TLS stack) or `rustls-tls`.
 *
 * Part of `libopentelemetry_c_sdk`. Requires linking the SDK alongside the API.
 */
#ifndef OPENTELEMETRY_C_OTLP_TRACE_EXPORTER_H
#define OPENTELEMETRY_C_OTLP_TRACE_EXPORTER_H

#include <opentelemetry_c/common.h>
#include <opentelemetry_c/trace_exporter.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Opaque OTLP trace exporter builder. Not thread-safe; confine to one thread. */
typedef struct otel_otlp_trace_exporter_builder_t otel_otlp_trace_exporter_builder_t;

/* Create a new OTLP trace exporter builder. NULL only on allocation failure. Release with
 * otel_otlp_trace_exporter_builder_destroy(). */
otel_otlp_trace_exporter_builder_t* otel_otlp_trace_exporter_builder_new(void);

/* Destroy an OTLP trace exporter builder (no-op on NULL). */
void otel_otlp_trace_exporter_builder_destroy(otel_otlp_trace_exporter_builder_t* builder);

/*
 * Set the full OTLP traces endpoint URL, used as-is (no path is appended), e.g.
 * "http://localhost:4318/v1/traces". Remember to include the "/v1/traces" path.
 *
 * If unset, the exporter falls back to (in order): the
 * OTEL_EXPORTER_OTLP_TRACES_ENDPOINT environment variable (used as-is), the
 * OTEL_EXPORTER_OTLP_ENDPOINT environment variable (with "/v1/traces" appended), and
 * finally the OTLP default "http://localhost:4318/v1/traces". Programmatic configuration
 * takes precedence over the environment variables.
 */
otel_status_t otel_otlp_trace_exporter_builder_set_endpoint(
    otel_otlp_trace_exporter_builder_t* builder, otel_string_view_t endpoint);

/*
 * Add an HTTP header sent with every export request (e.g. for authentication).
 *
 * Duplicate keys are rejected case-insensitively: adding a key that matches an already-added
 * key under ASCII case-insensitive comparison (e.g. "Authorization" vs "authorization")
 * returns OTEL_STATUS_INVALID_ARGUMENT (with a message via otel_last_error_message()) and
 * leaves the builder unchanged, rather than silently overwriting the earlier value.
 */
otel_status_t otel_otlp_trace_exporter_builder_add_header(
    otel_otlp_trace_exporter_builder_t* builder, otel_string_view_t key,
    otel_string_view_t value);

/* Set the per-request export timeout in milliseconds (0 => exporter default). */
otel_status_t otel_otlp_trace_exporter_builder_set_timeout_millis(
    otel_otlp_trace_exporter_builder_t* builder, uint64_t timeout_millis);

/*
 * Build a trace exporter from the accumulated configuration. On OTEL_STATUS_OK writes a new
 * otel_trace_exporter_t handle to *out (owned by the caller) and returns OTEL_STATUS_OK. On
 * failure sets *out to NULL, returns an error status, and records a message retrievable via
 * otel_last_error_message(). The builder is not consumed and must still be destroyed.
 *
 * Ownership of *out: release it with otel_trace_exporter_destroy(), or transfer it into a
 * span processor builder via otel_batch_span_processor_builder_set_exporter().
 */
otel_status_t otel_otlp_trace_exporter_builder_build(
    const otel_otlp_trace_exporter_builder_t* builder, otel_trace_exporter_t** out);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* OPENTELEMETRY_C_OTLP_TRACE_EXPORTER_H */
