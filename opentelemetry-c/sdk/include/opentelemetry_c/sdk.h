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
 *   otel_sdk_builder_t* b = otel_sdk_builder_new();
 *   otel_sdk_builder_set_service_name(b, otel_cstr("my-service"));
 *   otel_sdk_builder_set_otlp_endpoint(b, otel_cstr("http://localhost:4318/v1/traces"));
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

/* Destroy an SDK builder (no-op on NULL). Safe to call after otel_sdk_build(); the
 * builder is only read, never consumed, by build. */
void otel_sdk_builder_destroy(otel_sdk_builder_t* builder);

/* ---- Resource ------------------------------------------------------------- */

/* Set the `service.name` resource attribute. */
otel_status_t otel_sdk_builder_set_service_name(otel_sdk_builder_t* builder,
                                                otel_string_view_t name);

/* Add an arbitrary resource attribute. */
otel_status_t otel_sdk_builder_add_resource_attribute(otel_sdk_builder_t* builder,
                                                      otel_key_value_t attribute);

/* ---- OTLP exporter (HTTP/protobuf) ---------------------------------------- */

/*
 * Set the full OTLP traces endpoint URL, used as-is (no path is appended), e.g.
 * "http://localhost:4318/v1/traces". Remember to include the "/v1/traces" path.
 *
 * If unset, the exporter falls back to (in order): the
 * OTEL_EXPORTER_OTLP_TRACES_ENDPOINT environment variable (used as-is), the
 * OTEL_EXPORTER_OTLP_ENDPOINT environment variable (with "/v1/traces" appended), and
 * finally the OTLP default "http://localhost:4318/v1/traces". Programmatic
 * configuration takes precedence over the environment variables.
 */
otel_status_t otel_sdk_builder_set_otlp_endpoint(otel_sdk_builder_t* builder,
                                                 otel_string_view_t endpoint);

/* Add an HTTP header sent with every export request (e.g. for authentication). */
otel_status_t otel_sdk_builder_add_otlp_header(otel_sdk_builder_t* builder,
                                               otel_string_view_t key,
                                               otel_string_view_t value);

/* Set the per-request export timeout in milliseconds (0 => exporter default). */
otel_status_t otel_sdk_builder_set_otlp_timeout_millis(otel_sdk_builder_t* builder,
                                                       uint64_t timeout_millis);

/* ---- Batch span processor options (0 => spec default) --------------------- */

/*
 * Maximum queue size (default 2048). Bounded: a non-zero value larger than an internal
 * maximum is rejected with OTEL_STATUS_INVALID_ARGUMENT (not silently clamped), since the
 * processor preallocates a channel of this capacity.
 */
otel_status_t otel_sdk_builder_set_batch_max_queue_size(otel_sdk_builder_t* builder,
                                                        size_t max_queue_size);
/* Scheduled delay between exports, milliseconds (default 5000). */
otel_status_t otel_sdk_builder_set_batch_scheduled_delay_millis(otel_sdk_builder_t* builder,
                                                                uint64_t delay_millis);
/*
 * Maximum spans per export batch (default 512). Bounded like the queue size above: an
 * oversized non-zero value is rejected with OTEL_STATUS_INVALID_ARGUMENT. The effective
 * value is additionally capped by the SDK at the max queue size.
 */
otel_status_t otel_sdk_builder_set_batch_max_export_batch_size(otel_sdk_builder_t* builder,
                                                               size_t max_export_batch_size);
/*
 * Per-export timeout, milliseconds (default 30000). When set, this is applied as the OTLP
 * HTTP request timeout unless otel_sdk_builder_set_otlp_timeout_millis() is also set (which
 * then wins).
 */
otel_status_t otel_sdk_builder_set_batch_export_timeout_millis(otel_sdk_builder_t* builder,
                                                               uint64_t timeout_millis);

/* ---- Build ---------------------------------------------------------------- */

/*
 * Build an SDK from the accumulated configuration. On success writes a non-NULL handle
 * to *out_sdk and returns OTEL_STATUS_OK. On failure returns an error status, sets
 * *out_sdk to NULL, and records a message retrievable via otel_last_error_message().
 * The builder is not consumed and must still be destroyed by the caller.
 */
otel_status_t otel_sdk_build(const otel_sdk_builder_t* builder, otel_sdk_t** out_sdk);

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
