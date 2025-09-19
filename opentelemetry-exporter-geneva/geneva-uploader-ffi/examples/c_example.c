/*
 * Geneva FFI C Example (synchronous only)
 *
 * This example demonstrates:
 * - Reading configuration from environment
 * - Creating a Geneva client via geneva_client_new (out-param)
 * - Encoding/compressing ResourceLogs and ResourceSpans
 * - Uploading batches synchronously with geneva_upload_batch_sync
 * - Testing both logs and spans functionality
 *
 * Note: The non-blocking callback-based mechanism has been removed.
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <stdint.h>
#include "../include/geneva_ffi.h"

/* Prototypes from the example-only builder dylib (otlp_builder) */
extern int geneva_build_otlp_logs_minimal(const char* body_utf8,
                                          const char* resource_key,
                                          const char* resource_value,
                                          uint8_t** out_ptr,
                                          size_t* out_len);
extern int geneva_build_otlp_spans_minimal(const char* span_name,
                                           const char* resource_key,
                                           const char* resource_value,
                                           uint8_t** out_ptr,
                                           size_t* out_len);
extern void geneva_free_buffer(uint8_t* ptr, size_t len);

/* Helper to read env or default */
static const char* get_env_or_default(const char* name, const char* defval) {
    const char* v = getenv(name);
    return v ? v : defval;
}


int main(void) {
    printf("Geneva FFI Example (synchronous API)\n");
    printf("====================================\n\n");

    /* Required env */
    const char* endpoint      = getenv("GENEVA_ENDPOINT");
    const char* environment   = getenv("GENEVA_ENVIRONMENT");
    const char* account       = getenv("GENEVA_ACCOUNT");
    const char* namespaceName = getenv("GENEVA_NAMESPACE");
    const char* region        = getenv("GENEVA_REGION");
    const char* cfg_ver_str   = getenv("GENEVA_CONFIG_MAJOR_VERSION");

    if (!endpoint || !environment || !account || !namespaceName || !region || !cfg_ver_str) {
        printf("Missing required environment variables!\n");
        printf("  GENEVA_ENDPOINT\n");
        printf("  GENEVA_ENVIRONMENT\n");
        printf("  GENEVA_ACCOUNT\n");
        printf("  GENEVA_NAMESPACE\n");
        printf("  GENEVA_REGION\n");
        printf("  GENEVA_CONFIG_MAJOR_VERSION\n");
        return 1;
    }

    int cfg_ver = atoi(cfg_ver_str);
    if (cfg_ver <= 0) {
        printf("Invalid GENEVA_CONFIG_MAJOR_VERSION: %s\n", cfg_ver_str);
        return 1;
    }

    /* Optional env with defaults */
    const char* tenant       = get_env_or_default("GENEVA_TENANT", "default-tenant");
    const char* role_name    = get_env_or_default("GENEVA_ROLE_NAME", "default-role");
    const char* role_instance= get_env_or_default("GENEVA_ROLE_INSTANCE", "default-instance");

    /* Certificate auth if both provided; otherwise managed identity */
    const char* cert_path     = getenv("GENEVA_CERT_PATH");
    const char* cert_password = getenv("GENEVA_CERT_PASSWORD");
    int32_t auth_method = (cert_path && cert_password) ? GENEVA_AUTH_CERTIFICATE : GENEVA_AUTH_MANAGED_IDENTITY;

    printf("Configuration:\n");
    printf("  Endpoint: %s\n", endpoint);
    printf("  Environment: %s\n", environment);
    printf("  Account: %s\n", account);
    printf("  Namespace: %s\n", namespaceName);
    printf("  Region: %s\n", region);
    printf("  Config Major Version: %d\n", cfg_ver);
    printf("  Tenant: %s\n", tenant);
    printf("  Role Name: %s\n", role_name);
    printf("  Role Instance: %s\n", role_instance);
    printf("  Auth Method: %s\n", auth_method == GENEVA_AUTH_CERTIFICATE ? "Certificate" : "Managed Identity");
    if (auth_method == GENEVA_AUTH_CERTIFICATE) {
        printf("  Cert Path: %s\n", cert_path);
    }
    printf("\n");

    /* Build config */
    GenevaConfig cfg = {
        .endpoint = endpoint,
        .environment = environment,
        .account = account,
        .namespace_name = namespaceName,
        .region = region,
        .config_major_version = (uint32_t)cfg_ver,
        .auth_method = auth_method,
        .tenant = tenant,
        .role_name = role_name,
        .role_instance = role_instance,
    };
    if (auth_method == GENEVA_AUTH_CERTIFICATE) {
        cfg.auth.cert.cert_path = cert_path;
        cfg.auth.cert.cert_password = cert_password;
    } else {
        cfg.auth.msi.objid = NULL;
    }

    /* Create client */
    GenevaClientHandle* client = NULL;
    GenevaError rc = geneva_client_new(&cfg, &client);
    if (rc != GENEVA_SUCCESS || client == NULL) {
        printf("Failed to create Geneva client (code=%d)\n", rc);
        return 1;
    }
    printf("Geneva client created.\n");

    /* Test logs functionality */
    printf("\n=== Testing Logs ===\n");
    size_t logs_data_len = 0;
    uint8_t* logs_data = NULL;
    GenevaError logs_brc = geneva_build_otlp_logs_minimal("hello from c ffi", "service.name", "c-ffi-example", &logs_data, &logs_data_len);
    if (logs_brc != GENEVA_SUCCESS || logs_data == NULL || logs_data_len == 0) {
        printf("Failed to build OTLP logs payload (code=%d)\n", logs_brc);
        geneva_client_free(client);
        return 1;
    }

    /* Encode and compress logs to batches */
    EncodedBatchesHandle* logs_batches = NULL;
    GenevaError logs_enc_rc = geneva_encode_and_compress_logs(client, logs_data, logs_data_len, &logs_batches);
    if (logs_enc_rc != GENEVA_SUCCESS || logs_batches == NULL) {
        printf("Logs encode/compress failed (code=%d)\n", logs_enc_rc);
        geneva_free_buffer(logs_data, logs_data_len);
        geneva_client_free(client);
        return 1;
    }

    size_t logs_n = geneva_batches_len(logs_batches);
    printf("Encoded %zu log batch(es)\n", logs_n);

    /* Upload logs synchronously */
    GenevaError logs_first_err = GENEVA_SUCCESS;
    for (size_t i = 0; i < logs_n; i++) {
        GenevaError r = geneva_upload_batch_sync(client, logs_batches, i);
        if (r != GENEVA_SUCCESS) {
            logs_first_err = r;
            printf("Log batch %zu upload failed with error %d\n", i, r);
            break;
        }
    }

    /* Cleanup logs */
    geneva_batches_free(logs_batches);
    geneva_free_buffer(logs_data, logs_data_len);

    if (logs_first_err == GENEVA_SUCCESS) {
        printf("All log batches uploaded successfully.\n");
    } else {
        printf("Log upload finished with error code: %d\n", logs_first_err);
    }

    /* Test spans functionality */
    printf("\n=== Testing Spans ===\n");
    size_t spans_data_len = 0;
    uint8_t* spans_data = NULL;
    GenevaError spans_brc = geneva_build_otlp_spans_minimal("test-span", "service.name", "c-ffi-example", &spans_data, &spans_data_len);
    if (spans_brc != GENEVA_SUCCESS || spans_data == NULL || spans_data_len == 0) {
        printf("Failed to build OTLP spans payload (code=%d)\n", spans_brc);
        geneva_client_free(client);
        return 1;
    }

    /* Encode and compress spans to batches */
    EncodedBatchesHandle* spans_batches = NULL;
    GenevaError spans_enc_rc = geneva_encode_and_compress_spans(client, spans_data, spans_data_len, &spans_batches);
    if (spans_enc_rc != GENEVA_SUCCESS || spans_batches == NULL) {
        printf("Spans encode/compress failed (code=%d)\n", spans_enc_rc);
        geneva_free_buffer(spans_data, spans_data_len);
        geneva_client_free(client);
        return 1;
    }

    size_t spans_n = geneva_batches_len(spans_batches);
    printf("Encoded %zu span batch(es)\n", spans_n);

    /* Upload spans synchronously */
    GenevaError spans_first_err = GENEVA_SUCCESS;
    for (size_t i = 0; i < spans_n; i++) {
        GenevaError r = geneva_upload_batch_sync(client, spans_batches, i);
        if (r != GENEVA_SUCCESS) {
            spans_first_err = r;
            printf("Span batch %zu upload failed with error %d\n", i, r);
            break;
        }
    }

    /* Cleanup spans */
    geneva_batches_free(spans_batches);
    geneva_free_buffer(spans_data, spans_data_len);
    geneva_client_free(client);

    if (spans_first_err == GENEVA_SUCCESS) {
        printf("All span batches uploaded successfully.\n");
    } else {
        printf("Span upload finished with error code: %d\n", spans_first_err);
    }

    /* Final result */
    if (logs_first_err == GENEVA_SUCCESS && spans_first_err == GENEVA_SUCCESS) {
        printf("\n=== All uploads completed successfully! ===\n");
        return 0;
    }
    printf("\n=== Some uploads failed ===\n");
    return 1;
}
