/*
 * opentelemetry_c/trace.h
 *
 * Trace API: tracer providers, tracers, and spans, exposed as opaque handles.
 *
 * Handle ownership
 * ----------------
 * Every function that returns a handle transfers ownership to the caller, who must
 * release it with the matching *_destroy function. Passing NULL to any *_destroy is a
 * no-op. Handles must not be used after they are destroyed.
 *
 * Thread-safety
 * -------------
 * Providers and tracers are safe to share and use across threads. A single span handle
 * must NOT be used concurrently from multiple threads; use one span per thread (or
 * external synchronization). Distinct spans may be used concurrently. No *_destroy may
 * race with any other call on the same handle.
 */
#ifndef OPENTELEMETRY_C_TRACE_H
#define OPENTELEMETRY_C_TRACE_H

#include "common.h"

#ifdef __cplusplus
extern "C" {
#endif

/* Opaque handles. */
typedef struct otel_tracer_provider_t otel_tracer_provider_t;
typedef struct otel_tracer_t otel_tracer_t;
typedef struct otel_span_t otel_span_t;

/*
 * Span kind, mirroring the OpenTelemetry specification.
 *
 * Crosses the ABI as a fixed-width uint32_t (not a C enum). A value outside the range
 * below is treated as OTEL_SPAN_KIND_INTERNAL rather than producing an invalid value on
 * the Rust side. Use the OTEL_SPAN_KIND_* constants below.
 */
typedef uint32_t otel_span_kind_t;
enum {
    OTEL_SPAN_KIND_INTERNAL = 0, /* Default. */
    OTEL_SPAN_KIND_SERVER = 1,
    OTEL_SPAN_KIND_CLIENT = 2,
    OTEL_SPAN_KIND_PRODUCER = 3,
    OTEL_SPAN_KIND_CONSUMER = 4
};

/*
 * Span status code.
 *
 * Crosses the ABI as a fixed-width uint32_t (not a C enum). A value outside the range
 * below is rejected by otel_span_set_status() with OTEL_STATUS_INVALID_ARGUMENT. Use the
 * OTEL_SPAN_STATUS_* constants below.
 */
typedef uint32_t otel_span_status_code_t;
enum {
    OTEL_SPAN_STATUS_UNSET = 0, /* Default. */
    OTEL_SPAN_STATUS_OK = 1,
    OTEL_SPAN_STATUS_ERROR = 2
};

/*
 * Options for otel_tracer_start_span(). A NULL options pointer selects
 * OTEL_SPAN_KIND_INTERNAL and no explicit parent (a new root span).
 */
typedef struct otel_span_start_options_t {
    otel_span_kind_t kind;       /* The span kind. Unknown values fall back to
                                    OTEL_SPAN_KIND_INTERNAL. */
    const otel_span_t* parent;   /* Optional parent span; NULL => root span. */
} otel_span_start_options_t;

#if defined(__STDC_VERSION__) && (__STDC_VERSION__ >= 201112L) && \
    defined(UINTPTR_MAX) && (UINTPTR_MAX == 0xFFFFFFFFFFFFFFFFu)
_Static_assert(sizeof(otel_span_start_options_t) == 16,
               "otel_span_start_options_t ABI mismatch");
#endif

/* ---- Provider ------------------------------------------------------------- */

/*
 * Return an owned handle to the process-global tracer provider. Never NULL under
 * normal conditions. Release with otel_tracer_provider_destroy(). Tracers obtained
 * from it reflect whichever SDK is installed as global at the time of the request.
 */
otel_tracer_provider_t* otel_global_tracer_provider(void);

/*
 * Obtain a tracer from a provider.
 *
 *   name       - instrumentation scope name (required, non-empty recommended).
 *   version    - instrumentation scope version; pass an empty view to omit.
 *   schema_url - instrumentation schema URL; pass an empty view to omit.
 *
 * Return value:
 *   - Invalid provider handle: NULL.
 *   - No SDK installed (unbacked global provider): a valid no-op tracer.
 *   - A backed implementation whose tracer creation fails (e.g. a malformed string view or
 *     allocation failure): NULL, with the last-error set (see otel_last_error_message()) —
 *     NOT a no-op tracer.
 * Release with otel_tracer_destroy().
 */
otel_tracer_t* otel_tracer_provider_get_tracer(const otel_tracer_provider_t* provider,
                                               otel_string_view_t name,
                                               otel_string_view_t version,
                                               otel_string_view_t schema_url);

/*
 * Destroy a tracer-provider handle (no-op on NULL). Does NOT shut down the underlying
 * SDK; use otel_sdk_shutdown() for that.
 */
void otel_tracer_provider_destroy(otel_tracer_provider_t* provider);

/* ---- Tracer --------------------------------------------------------------- */

/*
 * Start a new span.
 *
 *   name    - span name (required).
 *   options - optional; NULL => internal-kind root span.
 *
 * Parenting: if options->parent is non-NULL it must be a live span handle. A parent span
 * produced by a DIFFERENT implementation (i.e. created via a different tracer/vtable than
 * this tracer) is treated as NO parent, so the new span becomes a root span.
 *
 * Return value:
 *   - Invalid tracer handle, or a non-NULL but invalid parent handle: NULL.
 *   - Unbacked (no-op) tracer: a valid no-op span.
 *   - A backed tracer whose span creation fails (e.g. a malformed name): NULL, with the
 *     last-error set — NOT a no-op span.
 * The returned span must be ended with otel_span_end() and released with
 * otel_span_destroy(). Destroying a span that was not explicitly ended performs a
 * best-effort end first.
 */
otel_span_t* otel_tracer_start_span(const otel_tracer_t* tracer,
                                    otel_string_view_t name,
                                    const otel_span_start_options_t* options);

/* Destroy a tracer handle (no-op on NULL). */
void otel_tracer_destroy(otel_tracer_t* tracer);

/* ---- Span ----------------------------------------------------------------- */

/* Set a typed attribute. Keys must be non-empty UTF-8. */
otel_status_t otel_span_set_string_attribute(otel_span_t* span,
                                             otel_string_view_t key,
                                             otel_string_view_t value);
otel_status_t otel_span_set_bool_attribute(otel_span_t* span,
                                           otel_string_view_t key,
                                           otel_bool_t value);
otel_status_t otel_span_set_int64_attribute(otel_span_t* span,
                                            otel_string_view_t key,
                                            int64_t value);
otel_status_t otel_span_set_double_attribute(otel_span_t* span,
                                             otel_string_view_t key,
                                             double value);

/* Set an attribute from a tagged key/value. */
otel_status_t otel_span_set_attribute(otel_span_t* span, otel_key_value_t attribute);

/*
 * Add a timestamped event with optional attributes. `attributes` may be NULL when
 * `attribute_count` is 0.
 */
otel_status_t otel_span_add_event(otel_span_t* span,
                                  otel_string_view_t name,
                                  const otel_key_value_t* attributes,
                                  size_t attribute_count);

/*
 * Set the span status. For OTEL_SPAN_STATUS_ERROR, `description` carries the error
 * message; for other codes it is ignored and may be an empty view. A `code` outside
 * otel_span_status_code_t is rejected with OTEL_STATUS_INVALID_ARGUMENT.
 */
otel_status_t otel_span_set_status(otel_span_t* span,
                                   otel_span_status_code_t code,
                                   otel_string_view_t description);

/* Rename a span. */
otel_status_t otel_span_update_name(otel_span_t* span, otel_string_view_t name);

/*
 * End a span, recording its end timestamp. Idempotent: calling more than once is safe
 * and returns OTEL_STATUS_OK without re-ending.
 */
otel_status_t otel_span_end(otel_span_t* span);

/*
 * Destroy a span handle (no-op on NULL). If the span was not explicitly ended, this
 * performs a best-effort end first.
 */
void otel_span_destroy(otel_span_t* span);

/* ---- Convenience helpers -------------------------------------------------- */

#if defined(__cplusplus) || (defined(__STDC_VERSION__) && __STDC_VERSION__ >= 199901L)
/*
 * Optional header-only shorthands over otel_span_set_status(). Each performs exactly one FFI
 * call — the same otel_span_set_status() the caller would make — and returns its status
 * unchanged; no allocation or copy is added. otel_span_set_ok() passes an empty description
 * (ignored for non-error codes). For otel_span_set_error() the `description` bytes are
 * BORROWED and must remain valid until the call returns.
 */
static inline otel_status_t otel_span_set_ok(otel_span_t* span) {
    return otel_span_set_status(span, OTEL_SPAN_STATUS_OK, otel_string_view_empty());
}
static inline otel_status_t otel_span_set_error(otel_span_t* span,
                                                otel_string_view_t description) {
    return otel_span_set_status(span, OTEL_SPAN_STATUS_ERROR, description);
}
#endif /* inline helpers */

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* OPENTELEMETRY_C_TRACE_H */
