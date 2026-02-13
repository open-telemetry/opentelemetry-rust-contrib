/*
 * Geneva FFI C Spans Example (synchronous only)
 *
 * This example demonstrates:
 * - Reading configuration from environment
 * - Creating a Geneva client via geneva_client_new (out-param)
 * - Encoding/compressing ResourceSpans
 * - Uploading batches synchronously with geneva_upload_batch_sync
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
    printf("Geneva FFI Spans Example (synchronous API)\n");
    printf("==========================================\n\n");

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

    /* Certificate auth if both provided; otherwise system managed identity */
    const char* cert_path     = getenv("GENEVA_CERT_PATH");
    const char* cert_password = getenv("GENEVA_CERT_PASSWORD");
    uint32_t auth_method = (cert_path && cert_password) ? GENEVA_AUTH_CERTIFICATE : GENEVA_AUTH_SYSTEM_MANAGED_IDENTITY;

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
    printf("  Auth Method: %s\n", auth_method == GENEVA_AUTH_CERTIFICATE ? "Certificate" : "System Managed Identity");
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
        .msi_resource = NULL, /* Optional MSI resource - can be set via environment if needed */
    };
    if (auth_method == GENEVA_AUTH_CERTIFICATE) {
        cfg.auth.cert.cert_path = cert_path;
        cfg.auth.cert.cert_password = cert_password;
    }

    /* Create client */
    GenevaClientHandle* client = NULL;
    char err_buf[512];
    GenevaError rc = geneva_client_new(&cfg, &client, err_buf, sizeof(err_buf));
    if (rc != GENEVA_SUCCESS || client == NULL) {
        printf("Failed to create Geneva client (code=%d): %s\n", rc, err_buf);
        return 1;
    }
    printf("Geneva client created.\n");

    /* Create ExportSpansServiceRequest bytes via FFI builder */
    size_t data_len = 0;
    uint8_t* data = NULL;
    GenevaError brc = geneva_build_otlp_spans_minimal("test-span", "service.name", "c-ffi-spans-example", &data, &data_len);
    if (brc != GENEVA_SUCCESS || data == NULL || data_len == 0) {
        printf("Failed to build OTLP spans payload (code=%d)\n", brc);
        geneva_client_free(client);
        return 1;
    }

    /* Encode and compress spans to batches */
    EncodedBatchesHandle* batches = NULL;
    GenevaError enc_rc = geneva_encode_and_compress_spans(client, data, data_len, &batches, err_buf, sizeof(err_buf));
    if (enc_rc != GENEVA_SUCCESS || batches == NULL) {
        printf("Spans encode/compress failed (code=%d): %s\n", enc_rc, err_buf);
        geneva_free_buffer(data, data_len);
        geneva_client_free(client);
        return 1;
    }

    size_t n = geneva_batches_len(batches);
    printf("Encoded %zu span batch(es)\n", n);

    /* Upload spans synchronously, batch by batch */
    GenevaError first_err = GENEVA_SUCCESS;
    for (size_t i = 0; i < n; i++) {
        GenevaError r = geneva_upload_batch_sync(client, batches, i, err_buf, sizeof(err_buf));
        if (r != GENEVA_SUCCESS) {
            first_err = r;
            printf("Span batch %zu upload failed with error %d: %s\n", i, r, err_buf);
            break;
        }
    }

    /* Cleanup */
    geneva_batches_free(batches);
    geneva_free_buffer(data, data_len);
    geneva_client_free(client);

    if (first_err == GENEVA_SUCCESS) {
        printf("All span batches uploaded successfully.\n");
        return 0;
    }
    printf("Span upload finished with error code: %d\n", first_err);
    return 1;
}