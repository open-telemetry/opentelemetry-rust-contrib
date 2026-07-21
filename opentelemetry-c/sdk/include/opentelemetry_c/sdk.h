/*
 * opentelemetry_c/sdk.h
 *
 * SDK configuration and lifecycle: build a tracer provider with an OTLP HTTP/protobuf
 * exporter and a batch span processor, install it globally, flush, and shut down.
 *
 * The SDK owns all of its own threading (a dedicated batch-processor OS thread and the
 * blocking HTTP client). No user-managed async runtime is required, and the library
 * never invokes any C callback.
 *
 * Threading & lifecycle contract
 * ------------------------------
 *   - An otel_sdk_t handle may be used concurrently from multiple threads:
 *     otel_sdk_set_as_global(), otel_sdk_force_flush(), otel_sdk_shutdown(), and
 *     otel_sdk_get_tracer_provider() are all safe to call at the same time on one handle.
 *   - otel_sdk_shutdown() runs the underlying shutdown at most once. The first call wins;
 *     concurrent or later calls return OTEL_STATUS_ALREADY_SHUTDOWN. After shutdown,
 *     force_flush / set_as_global return OTEL_STATUS_ALREADY_SHUTDOWN and span creation
 *     becomes a no-op. A concurrent set_as_global and shutdown may linearize in either
 *     order: set_as_global may still publish the provider if it observes the SDK as
 *     not-yet-shut-down (which then becomes a no-op once shutdown completes); once
 *     shutdown is observed, set_as_global returns OTEL_STATUS_ALREADY_SHUTDOWN.
 *   - A timed otel_sdk_force_flush() runs the flush on a helper thread; at most one such
 *     helper exists at a time (a concurrent timed flush returns OTEL_STATUS_TIMEOUT
 *     rather than spawning another). A blocking flush (timeout 0) uses the calling
 *     thread. See the function comment for details.
 *   - otel_sdk_destroy() must NOT race with any other call on the same handle; ensure all
 *     other SDK calls have returned (and, for a global install, that no spans are still
 *     being created) before destroying.
 *   - An otel_sdk_builder_t is NOT thread-safe; confine a builder to a single thread.
 *
 * Linking model
 * -------------
 * This header belongs to `libopentelemetry_c_sdk`. Applications link BOTH
 * `libopentelemetry_c_sdk` and `libopentelemetry_c_api` (and compile with both include
 * directories on the search path, since this header includes the API's common.h/trace.h).
 * Installing the SDK registers it into the API library's single global provider slot, so
 * instrumentation that links only `libopentelemetry_c_api` observes it.
 *
 * The shared global provider is guaranteed ONLY under dynamic linking with exactly one
 * loaded `libopentelemetry_c_api`. Statically linking the API into multiple artifacts
 * creates separate global slots and is NOT the shared-global model.
 *
 * Platform status: verified on Unix-like dynamic linking (Linux, macOS). Windows is not yet
 * verified — the SDK's undefined otel_api_* symbols need the API's import library at link
 * time; see the API README's "Platform support" section.
 *
 * Library lifetime
 * ----------------
 * Once otel_sdk_set_as_global() succeeds, it publishes this library's static implementation
 * vtable and an SDK-owned provider object into the API global slot. otel_sdk_shutdown() and
 * otel_sdk_destroy() do NOT clear that slot (they stop/free the otel_sdk_t handle only); the
 * slot is cleared only when another provider replaces it. Therefore, after a global install,
 * `libopentelemetry_c_sdk` must remain loaded until process exit OR until another provider
 * replaces the global slot. Shutdown+destroy does NOT make unloading the SDK safe. Any live
 * SDK-backed handles (tracer provider, tracer, span) must also be destroyed before unload.
 *
 * Typical lifecycle
 * -----------------
 *   Build an exporter, wrap it in a span processor, then hand the processor to the SDK
 *   builder (see otlp_trace_exporter.h and batch_span_processor.h for the pipeline pieces):
 *
 *   otel_otlp_trace_exporter_builder_t* eb = otel_otlp_trace_exporter_builder_new();
 *   otel_otlp_trace_exporter_builder_set_endpoint(eb, otel_cstr("http://localhost:4318/v1/traces"));
 *   otel_trace_exporter_t* exporter = NULL;
 *   otel_otlp_trace_exporter_builder_build(eb, &exporter);
 *   otel_otlp_trace_exporter_builder_destroy(eb);
 *
 *   otel_batch_span_processor_builder_t* pb = otel_batch_span_processor_builder_new();
 *   otel_batch_span_processor_builder_set_exporter(pb, exporter); // ownership transfers on OK
 *   otel_span_processor_t* processor = NULL;
 *   otel_batch_span_processor_builder_build(pb, &processor);
 *   otel_batch_span_processor_builder_destroy(pb);
 *
 *   otel_sdk_builder_t* b = otel_sdk_builder_new();
 *   otel_sdk_builder_set_service_name(b, otel_cstr("my-service"));
 *   otel_sdk_builder_add_span_processor(b, processor); // ownership transfers on OK
 *   otel_sdk_t* sdk = NULL;
 *   if (otel_sdk_build(b, &sdk) == OTEL_STATUS_OK) {
 *       otel_sdk_set_as_global(sdk);
 *       ... create spans via the API ...
 *       otel_sdk_shutdown(sdk, 5000);
 *       otel_sdk_destroy(sdk);
 *   }
 *   otel_sdk_builder_destroy(b);
 */
#ifndef OPENTELEMETRY_C_SDK_H
#define OPENTELEMETRY_C_SDK_H

#include <opentelemetry_c/common.h>
#include <opentelemetry_c/trace.h>
#include <opentelemetry_c/span_processor.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Opaque handles. */
typedef struct otel_sdk_builder_t otel_sdk_builder_t;
typedef struct otel_sdk_t otel_sdk_t;

/* ---- Builder lifecycle ---------------------------------------------------- */

/* Create a new SDK builder with spec-default settings. NULL only on allocation
 * failure. Release with otel_sdk_builder_destroy(). */
otel_sdk_builder_t* otel_sdk_builder_new(void);

/* Destroy an SDK builder (no-op on NULL). Frees any span processors that were transferred
 * in via otel_sdk_builder_add_span_processor() but not yet consumed by otel_sdk_build(). */
void otel_sdk_builder_destroy(otel_sdk_builder_t* builder);

/* ---- Resource ------------------------------------------------------------- */

/* Set the `service.name` resource attribute. */
otel_status_t otel_sdk_builder_set_service_name(otel_sdk_builder_t* builder,
                                                otel_string_view_t name);

/* Add an arbitrary resource attribute. */
otel_status_t otel_sdk_builder_add_resource_attribute(otel_sdk_builder_t* builder,
                                                      otel_key_value_t attribute);

/* ---- Span processors ------------------------------------------------------ */

/*
 * Add (transfer) a span processor to the SDK's trace pipeline. Build the processor with a
 * span-processor builder (e.g. batch_span_processor.h), which in turn consumes a trace
 * exporter (e.g. otlp_trace_exporter.h).
 *
 * Ownership: on OTEL_STATUS_OK, ownership of `processor` transfers to the SDK builder and
 * the caller must NOT call otel_span_processor_destroy() on it. On failure (invalid builder
 * or processor), the caller still owns `processor`. Add more than one processor to fan spans
 * out to multiple pipelines. A builder with no span processor still builds a valid SDK whose
 * spans are simply not exported.
 */
otel_status_t otel_sdk_builder_add_span_processor(otel_sdk_builder_t* builder,
                                                  otel_span_processor_t* processor);

/* ---- Build ---------------------------------------------------------------- */

/*
 * Build an SDK from the accumulated configuration. On success writes a non-NULL handle
 * to *out_sdk and returns OTEL_STATUS_OK. On failure returns an error status, sets
 * *out_sdk to NULL, and records a message retrievable via otel_last_error_message().
 *
 * The span processors transferred to the builder move into the built SDK; the builder
 * remains owned by the caller and must still be destroyed. Note that a second build on the
 * same builder produces an SDK with no processors (they were consumed by the first build).
 */
otel_status_t otel_sdk_build(otel_sdk_builder_t* builder, otel_sdk_t** out_sdk);

/* ---- Provider access and global installation ------------------------------ */

/*
 * Return an owned tracer-provider handle backed by this SDK. Independent of the SDK
 * handle's lifetime; release with otel_tracer_provider_destroy(). NULL if `sdk` is
 * invalid.
 */
otel_tracer_provider_t* otel_sdk_get_tracer_provider(const otel_sdk_t* sdk);

/*
 * Install this SDK's provider as the process-global provider. May be called more than
 * once; the most recent call wins. Returns OTEL_STATUS_ALREADY_SHUTDOWN if the SDK has
 * been shut down.
 *
 * A concurrent set_as_global and otel_sdk_shutdown() may linearize in either order: if
 * set_as_global observes the SDK as not-yet-shut-down it may publish the provider (which
 * then becomes a no-op once shutdown completes); once shutdown is observed, set_as_global
 * returns OTEL_STATUS_ALREADY_SHUTDOWN.
 */
otel_status_t otel_sdk_set_as_global(otel_sdk_t* sdk);

/* ---- Lifecycle ------------------------------------------------------------ */

/*
 * Flush buffered spans.
 *   - timeout_millis == 0: block on the calling thread until the flush completes.
 *   - timeout_millis  > 0: run the flush on a helper thread and return
 *     OTEL_STATUS_TIMEOUT if it does not finish in time (the flush continues in the
 *     background). At most one timed-flush helper thread runs at a time: while one is in
 *     flight, a concurrent timed flush returns OTEL_STATUS_TIMEOUT immediately instead of
 *     spawning another thread. Returns OTEL_STATUS_INTERNAL_ERROR if the helper thread
 *     cannot be spawned, or OTEL_STATUS_ALREADY_SHUTDOWN after shutdown.
 */
otel_status_t otel_sdk_force_flush(otel_sdk_t* sdk, uint64_t timeout_millis);

/*
 * Shut down the SDK, flushing and stopping the pipeline. The underlying shutdown runs at
 * most once: the first call performs it and returns its result; concurrent or subsequent
 * calls return OTEL_STATUS_ALREADY_SHUTDOWN without side effects. `timeout_millis` of 0
 * uses the SDK default (5s). After shutdown, span creation through this SDK becomes a
 * no-op. This should be called before process exit to avoid losing buffered spans.
 */
otel_status_t otel_sdk_shutdown(otel_sdk_t* sdk, uint64_t timeout_millis);

/*
 * Destroy an SDK handle (no-op on NULL). If not already shut down, dropping the SDK
 * triggers a best-effort shutdown; prefer calling otel_sdk_shutdown() explicitly. Must
 * not race with any other call on the same SDK handle.
 */
void otel_sdk_destroy(otel_sdk_t* sdk);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* OPENTELEMETRY_C_SDK_H */
