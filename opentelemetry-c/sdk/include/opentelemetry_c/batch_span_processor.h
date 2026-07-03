/*
 * opentelemetry_c/batch_span_processor.h
 *
 * Builder for the **batch span processor**, the one concrete span processor currently
 * implemented. It consumes a trace exporter (see trace_exporter.h) and produces a generic
 * otel_span_processor_t (see span_processor.h) that the SDK builder then consumes.
 *
 * The processor buffers finished spans and exports them in batches on a dedicated OS thread,
 * per the OpenTelemetry batch span processor spec.
 *
 * Part of `libopentelemetry_c_sdk`. Requires linking the SDK alongside the API.
 */
#ifndef OPENTELEMETRY_C_BATCH_SPAN_PROCESSOR_H
#define OPENTELEMETRY_C_BATCH_SPAN_PROCESSOR_H

#include <opentelemetry_c/common.h>
#include <opentelemetry_c/span_processor.h>
#include <opentelemetry_c/trace_exporter.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Opaque batch span processor builder. Not thread-safe; confine to one thread. */
typedef struct otel_batch_span_processor_builder_t otel_batch_span_processor_builder_t;

/* Create a new batch span processor builder. NULL only on allocation failure. Release with
 * otel_batch_span_processor_builder_destroy(). */
otel_batch_span_processor_builder_t* otel_batch_span_processor_builder_new(void);

/*
 * Destroy a batch span processor builder (no-op on NULL). Frees a transferred exporter that
 * was not yet consumed by otel_batch_span_processor_builder_build().
 */
void otel_batch_span_processor_builder_destroy(otel_batch_span_processor_builder_t* builder);

/*
 * Set (transfer) the trace exporter this processor exports through.
 *
 * Ownership: on OTEL_STATUS_OK, ownership of `exporter` transfers to the builder and the
 * caller must NOT call otel_trace_exporter_destroy() on it. On failure (invalid builder or
 * exporter), the caller still owns `exporter`. Setting an exporter when one was already set
 * frees the previous one.
 */
otel_status_t otel_batch_span_processor_builder_set_exporter(
    otel_batch_span_processor_builder_t* builder, otel_trace_exporter_t* exporter);

/* ---- Batch options (0 => spec default) ------------------------------------ */

/*
 * Maximum queue size (default 2048). Bounded: a non-zero value larger than an internal
 * maximum is rejected with OTEL_STATUS_INVALID_ARGUMENT (not silently clamped), since the
 * processor preallocates a channel of this capacity.
 */
otel_status_t otel_batch_span_processor_builder_set_max_queue_size(
    otel_batch_span_processor_builder_t* builder, size_t max_queue_size);

/* Scheduled delay between exports, milliseconds (default 5000). */
otel_status_t otel_batch_span_processor_builder_set_scheduled_delay_millis(
    otel_batch_span_processor_builder_t* builder, uint64_t delay_millis);

/*
 * Maximum spans per export batch (default 512). Bounded like the queue size above: an
 * oversized non-zero value is rejected with OTEL_STATUS_INVALID_ARGUMENT. The effective
 * value is additionally capped by the SDK at the max queue size.
 */
otel_status_t otel_batch_span_processor_builder_set_max_export_batch_size(
    otel_batch_span_processor_builder_t* builder, size_t max_export_batch_size);

/*
 * Per-export timeout, milliseconds (default 30000). Accepted and validated for a stable API
 * shape. Note: the current stable synchronous batch span processor does not apply a
 * programmatic per-export timeout; it uses the SDK default (overridable via the
 * OTEL_BSP_EXPORT_TIMEOUT environment variable).
 */
otel_status_t otel_batch_span_processor_builder_set_export_timeout_millis(
    otel_batch_span_processor_builder_t* builder, uint64_t timeout_millis);

/* ---- Build ---------------------------------------------------------------- */

/*
 * Build a span processor from the accumulated configuration. Requires an exporter set via
 * otel_batch_span_processor_builder_set_exporter() (otherwise OTEL_STATUS_INVALID_CONFIG).
 *
 * On OTEL_STATUS_OK writes a new otel_span_processor_t handle to *out (owned by the caller)
 * and returns OTEL_STATUS_OK; the exporter previously transferred here moves into the built
 * processor. On failure sets *out to NULL, returns an error status, and records a message
 * retrievable via otel_last_error_message(). The builder is not consumed and must still be
 * destroyed.
 *
 * Ownership of *out: release it with otel_span_processor_destroy(), or transfer it into the
 * SDK builder via otel_sdk_builder_add_span_processor().
 */
otel_status_t otel_batch_span_processor_builder_build(
    otel_batch_span_processor_builder_t* builder, otel_span_processor_t** out);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* OPENTELEMETRY_C_BATCH_SPAN_PROCESSOR_H */
