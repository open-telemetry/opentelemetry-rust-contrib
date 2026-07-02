/*
 * c-basic-traces: a minimal C program that uses the opentelemetry-c SDK to emit a
 * couple of spans over OTLP HTTP/protobuf.
 *
 * It demonstrates the full lifecycle:
 *   1. build an SDK (resource + OTLP endpoint + batch options)
 *   2. install it as the global provider
 *   3. get a tracer from the global provider
 *   4. start a span, set typed attributes, add an event, set status
 *   5. start a child span using the parent option
 *   6. end the spans
 *   7. force flush and shut down
 *
 * By default it exports to http://localhost:4318/v1/traces. Override with the
 * OTEL_EXPORTER_OTLP_TRACES_ENDPOINT environment variable, e.g. point it at a local
 * OpenTelemetry Collector. If nothing is listening, the SDK logs export errors but the
 * program still exits cleanly (runtime export failures never crash the process).
 *
 * Build & run: see this directory's Makefile and README.md.
 */
#include <opentelemetry_c/api.h>

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* Print the last error message (if any) recorded on this thread. */
static void print_last_error(const char* context) {
    otel_string_view_t msg = otel_last_error_message();
    if (msg.ptr != NULL && msg.len > 0) {
        fprintf(stderr, "%s: %.*s\n", context, (int)msg.len, msg.ptr);
    } else {
        fprintf(stderr, "%s: (no detail)\n", context);
    }
}

int main(void) {
    otel_string_view_t version = otel_version_string();
    printf("opentelemetry-c version %.*s\n", (int)version.len, version.ptr);

    /* ---- 1. Configure and build the SDK ---------------------------------- */
    otel_sdk_builder_t* builder = otel_sdk_builder_new();
    if (builder == NULL) {
        fprintf(stderr, "failed to allocate SDK builder\n");
        return EXIT_FAILURE;
    }

    otel_sdk_builder_set_service_name(builder, otel_cstr("c-basic-traces"));

    /* Arbitrary resource attribute. */
    otel_key_value_t deployment;
    deployment.key = otel_cstr("deployment.environment");
    deployment.value_type = OTEL_ATTRIBUTE_TYPE_STRING;
    deployment.value.string_value = otel_cstr("demo");
    otel_sdk_builder_add_resource_attribute(builder, deployment);

    /* Endpoint: full traces URL, used as-is. Prefer the standard env var if set. */
    const char* endpoint = getenv("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT");
    if (endpoint == NULL || endpoint[0] == '\0') {
        endpoint = "http://localhost:4318/v1/traces";
    }
    otel_sdk_builder_set_otlp_endpoint(builder, otel_cstr(endpoint));
    printf("exporting to %s\n", endpoint);

    /* Batch processor tuning (all optional; 0 would mean "use spec default"). */
    otel_sdk_builder_set_batch_max_queue_size(builder, 2048);
    otel_sdk_builder_set_batch_scheduled_delay_millis(builder, 1000);
    otel_sdk_builder_set_batch_max_export_batch_size(builder, 512);
    otel_sdk_builder_set_otlp_timeout_millis(builder, 10000);

    otel_sdk_t* sdk = NULL;
    otel_status_t status = otel_sdk_build(builder, &sdk);
    /* The builder is only read by build(); we can destroy it now. */
    otel_sdk_builder_destroy(builder);
    if (status != OTEL_STATUS_OK || sdk == NULL) {
        print_last_error("otel_sdk_build failed");
        return EXIT_FAILURE;
    }

    /* ---- 2. Install as the global provider ------------------------------- */
    if (otel_sdk_set_as_global(sdk) != OTEL_STATUS_OK) {
        print_last_error("otel_sdk_set_as_global failed");
        otel_sdk_destroy(sdk);
        return EXIT_FAILURE;
    }

    /* ---- 3. Get a tracer from the global provider ------------------------ */
    otel_tracer_provider_t* provider = otel_global_tracer_provider();
    otel_tracer_t* tracer = otel_tracer_provider_get_tracer(
        provider,
        otel_cstr("c-basic-traces"),          /* instrumentation scope name    */
        otel_cstr("0.1.0"),                    /* scope version                 */
        otel_string_view_empty());             /* schema URL: omitted           */
    if (tracer == NULL) {
        print_last_error("otel_tracer_provider_get_tracer failed");
        otel_tracer_provider_destroy(provider);
        otel_sdk_destroy(sdk);
        return EXIT_FAILURE;
    }

    /* ---- 4. Start a parent span with attributes, an event, and status ---- */
    otel_span_start_options_t parent_opts;
    parent_opts.kind = OTEL_SPAN_KIND_SERVER;
    parent_opts.parent = NULL; /* root span */
    otel_span_t* parent = otel_tracer_start_span(tracer, otel_cstr("handle-request"), &parent_opts);
    if (parent == NULL) {
        print_last_error("otel_tracer_start_span (parent) failed");
        otel_tracer_destroy(tracer);
        otel_tracer_provider_destroy(provider);
        otel_sdk_destroy(sdk);
        return EXIT_FAILURE;
    }

    otel_span_set_string_attribute(parent, otel_cstr("http.request.method"), otel_cstr("GET"));
    otel_span_set_int64_attribute(parent, otel_cstr("http.response.status_code"), 200);
    otel_span_set_bool_attribute(parent, otel_cstr("cache.hit"), OTEL_FALSE);
    otel_span_set_double_attribute(parent, otel_cstr("duration.seconds"), 0.0123);

    otel_key_value_t event_attrs[1];
    event_attrs[0].key = otel_cstr("worker.id");
    event_attrs[0].value_type = OTEL_ATTRIBUTE_TYPE_INT64;
    event_attrs[0].value.int64_value = 7;
    otel_span_add_event(parent, otel_cstr("dispatch"), event_attrs, 1);

    /* ---- 5. Start a child span using the parent option ------------------- */
    otel_span_start_options_t child_opts;
    child_opts.kind = OTEL_SPAN_KIND_CLIENT;
    child_opts.parent = parent; /* link as a child of `parent` */
    otel_span_t* child = otel_tracer_start_span(tracer, otel_cstr("query-database"), &child_opts);
    if (child != NULL) {
        otel_span_set_string_attribute(child, otel_cstr("db.system"), otel_cstr("postgresql"));
        otel_span_set_status(child, OTEL_SPAN_STATUS_OK, otel_string_view_empty());
        otel_span_end(child);
        otel_span_destroy(child);
    } else {
        print_last_error("otel_tracer_start_span (child) failed");
    }

    otel_span_set_status(parent, OTEL_SPAN_STATUS_OK, otel_string_view_empty());
    otel_span_end(parent);
    otel_span_destroy(parent);

    /* ---- 6. Flush and shut down ------------------------------------------ */
    otel_tracer_destroy(tracer);
    otel_tracer_provider_destroy(provider);

    status = otel_sdk_force_flush(sdk, 5000);
    if (status != OTEL_STATUS_OK) {
        print_last_error("otel_sdk_force_flush warning");
    }

    status = otel_sdk_shutdown(sdk, 5000);
    if (status != OTEL_STATUS_OK) {
        print_last_error("otel_sdk_shutdown warning");
    }

    otel_sdk_destroy(sdk);

    printf("done\n");
    return EXIT_SUCCESS;
}
