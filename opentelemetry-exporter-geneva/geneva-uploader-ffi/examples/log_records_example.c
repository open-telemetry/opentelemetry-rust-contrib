/*
 * Geneva FFI — zero-copy log record example
 *
 * Demonstrates geneva_encode_and_compress_log_records(): log records are
 * passed as a flat C array and encoded directly to Geneva Bond format
 * without any intermediate OTLP serialisation.
 *
 * Required environment variables:
 *   GENEVA_ENDPOINT              e.g. https://<gcs-host>
 *   GENEVA_ENVIRONMENT           e.g. Test
 *   GENEVA_ACCOUNT               e.g. myaccount
 *   GENEVA_NAMESPACE             e.g. mynamespace
 *   GENEVA_REGION                e.g. eastus
 *   GENEVA_CONFIG_MAJOR_VERSION  e.g. 2
 *
 * Optional (certificate auth — falls back to system managed identity):
 *   GENEVA_CERT_PATH             e.g. /path/to/client.p12
 *   GENEVA_CERT_PASSWORD         e.g. secret
 *
 * Optional identity:
 *   GENEVA_TENANT                default: "default-tenant"
 *   GENEVA_ROLE_NAME             default: "default-role"
 *   GENEVA_ROLE_INSTANCE         default: "default-instance"
 *
 * Build and run:
 *   cd examples && make log-records-example
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <stdint.h>
#include "../include/geneva_ffi.h"

static const char* get_env_or_default(const char* name, const char* defval) {
    const char* v = getenv(name);
    return v ? v : defval;
}

/* Return nanoseconds since Unix epoch. */
static uint64_t now_nanos(void) {
    struct timespec ts;
    clock_gettime(CLOCK_REALTIME, &ts);
    return (uint64_t)ts.tv_sec * 1000000000ULL + (uint64_t)ts.tv_nsec;
}

int main(void) {
    printf("Geneva FFI — zero-copy log record example\n");
    printf("==========================================\n\n");

    /* ------------------------------------------------------------------ */
    /* 1. Read configuration from environment                              */
    /* ------------------------------------------------------------------ */

    const char* endpoint      = getenv("GENEVA_ENDPOINT");
    const char* environment   = getenv("GENEVA_ENVIRONMENT");
    const char* account       = getenv("GENEVA_ACCOUNT");
    const char* namespaceName = getenv("GENEVA_NAMESPACE");
    const char* region        = getenv("GENEVA_REGION");
    const char* cfg_ver_str   = getenv("GENEVA_CONFIG_MAJOR_VERSION");

    if (!endpoint || !environment || !account || !namespaceName || !region || !cfg_ver_str) {
        fprintf(stderr, "Missing required environment variables:\n"
                        "  GENEVA_ENDPOINT\n"
                        "  GENEVA_ENVIRONMENT\n"
                        "  GENEVA_ACCOUNT\n"
                        "  GENEVA_NAMESPACE\n"
                        "  GENEVA_REGION\n"
                        "  GENEVA_CONFIG_MAJOR_VERSION\n");
        return 1;
    }

    int cfg_ver = atoi(cfg_ver_str);
    if (cfg_ver <= 0) {
        fprintf(stderr, "Invalid GENEVA_CONFIG_MAJOR_VERSION: %s\n", cfg_ver_str);
        return 1;
    }

    const char* tenant        = get_env_or_default("GENEVA_TENANT",        "default-tenant");
    const char* role_name     = get_env_or_default("GENEVA_ROLE_NAME",     "default-role");
    const char* role_instance = get_env_or_default("GENEVA_ROLE_INSTANCE", "default-instance");
    const char* cert_path     = getenv("GENEVA_CERT_PATH");
    const char* cert_password = getenv("GENEVA_CERT_PASSWORD");

    uint32_t auth_method = (cert_path && cert_password)
                               ? GENEVA_AUTH_CERTIFICATE
                               : GENEVA_AUTH_SYSTEM_MANAGED_IDENTITY;

    printf("  Endpoint:    %s\n", endpoint);
    printf("  Environment: %s\n", environment);
    printf("  Namespace:   %s\n", namespaceName);
    printf("  Auth:        %s\n",
           auth_method == GENEVA_AUTH_CERTIFICATE ? "Certificate" : "System MSI");
    printf("\n");

    /* ------------------------------------------------------------------ */
    /* 2. Create Geneva client                                             */
    /* ------------------------------------------------------------------ */

    GenevaConfig cfg = {
        .endpoint             = endpoint,
        .environment          = environment,
        .account              = account,
        .namespace_name       = namespaceName,
        .region               = region,
        .config_major_version = (uint32_t)cfg_ver,
        .auth_method          = auth_method,
        .tenant               = tenant,
        .role_name            = role_name,
        .role_instance        = role_instance,
        .msi_resource         = NULL,
    };
    if (auth_method == GENEVA_AUTH_CERTIFICATE) {
        cfg.auth.cert.cert_path     = cert_path;
        cfg.auth.cert.cert_password = cert_password;
    }

    GenevaClientHandle* client = NULL;
    char err_buf[512];
    GenevaError rc = geneva_client_new(&cfg, &client, err_buf, sizeof(err_buf));
    if (rc != GENEVA_SUCCESS || !client) {
        fprintf(stderr, "Failed to create Geneva client (code=%d): %s\n", rc, err_buf);
        return 1;
    }
    printf("Geneva client created.\n\n");

    /* ------------------------------------------------------------------ */
    /* 3. Build log records on the stack                                   */
    /*                                                                     */
    /* The records array (and all pointed-to strings) only need to stay    */
    /* alive for the duration of the encode call below.                    */
    /* ------------------------------------------------------------------ */

    uint64_t t = now_nanos();

    /* Attribute arrays for record 1 (request log with string + int attrs) */
    const char* r1_keys[]   = { "http.method", "http.status_code", "http.url" };
    GenevaAttrValueC r1_vals[] = {
        { GENEVA_ATTR_STRING, { .str_val   = "GET"                          } },
        { GENEVA_ATTR_INT64,  { .int64_val = 200                            } },
        { GENEVA_ATTR_STRING, { .str_val   = "/api/v1/health"               } },
    };

    /* Attribute arrays for record 2 (error log with bool attr) */
    const char* r2_keys[]   = { "error", "component" };
    GenevaAttrValueC r2_vals[] = {
        { GENEVA_ATTR_BOOL,   { .bool_val  = 1           } },
        { GENEVA_ATTR_STRING, { .str_val   = "auth"      } },
    };

    /* Fake trace context for record 2 */
    uint8_t trace_id[16] = {
        0x01,0x02,0x03,0x04,0x05,0x06,0x07,0x08,
        0x09,0x0a,0x0b,0x0c,0x0d,0x0e,0x0f,0x10
    };
    uint8_t span_id[8] = { 0x11,0x12,0x13,0x14,0x15,0x16,0x17,0x18 };

    GenevaLogRecordC records[] = {
        /* Record 0 — INFO, default event name "Log", no trace context */
        {
            .event_name              = "Log",
            .time_unix_nano          = t,
            .observed_time_unix_nano = 0,
            .severity_number         = 9,   /* SEVERITY_NUMBER_INFO */
            .severity_text           = "INFO",
            .body                    = "incoming request handled successfully",
            .trace_id_present        = 0,
            .span_id_present         = 0,
            .flags_present           = 0,
            .attr_keys               = r1_keys,
            .attr_values             = r1_vals,
            .attr_count              = 3,
        },
        /* Record 1 — ERROR with trace context */
        {
            .event_name              = "Log",
            .time_unix_nano          = t + 500000,  /* 0.5 ms later */
            .observed_time_unix_nano = 0,
            .severity_number         = 17,  /* SEVERITY_NUMBER_ERROR */
            .severity_text           = "ERROR",
            .body                    = "token validation failed",
            .trace_id                = { /* initialised below */ },
            .trace_id_present        = 1,
            .span_id                 = { /* initialised below */ },
            .span_id_present         = 1,
            .flags                   = 1,
            .flags_present           = 1,
            .attr_keys               = r2_keys,
            .attr_values             = r2_vals,
            .attr_count              = 2,
        },
        /* Record 2 — WARN, no attributes */
        {
            .event_name              = "Log",
            .time_unix_nano          = t + 1000000, /* 1 ms later */
            .observed_time_unix_nano = 0,
            .severity_number         = 13,  /* SEVERITY_NUMBER_WARN */
            .severity_text           = "WARN",
            .body                    = "rate limit approaching threshold",
            .trace_id_present        = 0,
            .span_id_present         = 0,
            .flags_present           = 0,
            .attr_keys               = NULL,
            .attr_values             = NULL,
            .attr_count              = 0,
        },
    };

    /* Copy trace/span IDs into the struct (they are fixed-size arrays) */
    memcpy(records[1].trace_id, trace_id, sizeof(trace_id));
    memcpy(records[1].span_id,  span_id,  sizeof(span_id));

    size_t record_count = sizeof(records) / sizeof(records[0]);
    printf("Encoding %zu log record(s)...\n", record_count);

    /* ------------------------------------------------------------------ */
    /* 4. Encode records → Bond+LZ4 batches (zero-copy, synchronous)       */
    /* ------------------------------------------------------------------ */

    EncodedBatchesHandle* batches = NULL;
    GenevaError enc_rc = geneva_encode_and_compress_log_records(
        client, records, record_count, &batches, err_buf, sizeof(err_buf));

    if (enc_rc != GENEVA_SUCCESS || !batches) {
        fprintf(stderr, "Encode failed (code=%d): %s\n", enc_rc, err_buf);
        geneva_client_free(client);
        return 1;
    }

    /* Records array (and all its string pointers) may be reused / freed  */
    /* here — encode_and_compress_log_records already returned.           */

    size_t n = geneva_batches_len(batches);
    printf("Encoded into %zu batch(es).\n\n", n);

    /* ------------------------------------------------------------------ */
    /* 5. Upload each batch synchronously                                  */
    /* ------------------------------------------------------------------ */

    GenevaError first_err = GENEVA_SUCCESS;
    for (size_t i = 0; i < n; i++) {
        GenevaError up_rc = geneva_upload_batch_sync(
            client, batches, i, err_buf, sizeof(err_buf));
        if (up_rc != GENEVA_SUCCESS) {
            first_err = up_rc;
            fprintf(stderr, "Batch %zu upload failed (code=%d): %s\n",
                    i, up_rc, err_buf);
            break;
        }
        printf("  batch %zu uploaded.\n", i);
    }

    /* ------------------------------------------------------------------ */
    /* 6. Cleanup                                                          */
    /* ------------------------------------------------------------------ */

    geneva_batches_free(batches);
    geneva_client_free(client);

    if (first_err == GENEVA_SUCCESS) {
        printf("\nAll batches uploaded successfully.\n");
        return 0;
    }
    printf("\nFinished with error code: %d\n", first_err);
    return 1;
}
