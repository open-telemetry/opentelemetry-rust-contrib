#ifndef GENEVA_FFI_H
#define GENEVA_FFI_H

#include <stdint.h>
#include <stdlib.h>
#include "geneva_errors.h"

#ifdef __cplusplus
extern "C" {
#endif

// Opaque handles
typedef struct GenevaClientHandle GenevaClientHandle;
typedef struct EncodedBatchesHandle EncodedBatchesHandle;

// Authentication method constants
#define GENEVA_AUTH_MANAGED_IDENTITY 0
#define GENEVA_AUTH_CERTIFICATE 1

/* Configuration for certificate auth (valid only when auth_method == GENEVA_AUTH_CERTIFICATE) */
typedef struct {
    const char* cert_path;      /* Path to certificate file */
    const char* cert_password;  /* Certificate password */
} GenevaCertAuthConfig;

/* Configuration for managed identity auth (valid only when auth_method == GENEVA_AUTH_MANAGED_IDENTITY) */
typedef struct {
    const char* objid; /* Optional: Azure AD object ID as NUL-terminated GUID string
                          e.g. "00000000-0000-0000-0000-000000000000" */
} GenevaMSIAuthConfig;

/* Tagged union for auth-specific configuration.
   The active member is determined by 'auth_method' in GenevaConfig. */
typedef union {
    GenevaMSIAuthConfig msi;    /* Valid when auth_method == GENEVA_AUTH_MANAGED_IDENTITY */
    GenevaCertAuthConfig cert;  /* Valid when auth_method == GENEVA_AUTH_CERTIFICATE */
} GenevaAuthConfig;

/* Configuration structure for Geneva client (C-compatible, tagged union) */
typedef struct {
    const char* endpoint;
    const char* environment;
    const char* account;
    const char* namespace_name;
    const char* region;
    uint32_t config_major_version;
    int32_t auth_method; /* 0 = Managed Identity, 1 = Certificate */
    const char* tenant;
    const char* role_name;
    const char* role_instance;
    GenevaAuthConfig auth; /* Active member selected by auth_method */
} GenevaConfig;

/* Create a new Geneva client.
   - On success returns GENEVA_SUCCESS and writes *out_handle.
   - On failure returns an error code. */
GenevaError geneva_client_new(const GenevaConfig* config,
                              GenevaClientHandle** out_handle);


/* 1) Encode and compress logs into batches (synchronous).
      `data` is a protobuf-encoded ExportLogsServiceRequest.
      - On success returns GENEVA_SUCCESS and writes *out_batches.
      - On failure returns an error code.
      Caller must free *out_batches with geneva_batches_free. */
GenevaError geneva_encode_and_compress_logs(GenevaClientHandle* handle,
                                            const uint8_t* data,
                                            size_t data_len,
                                            EncodedBatchesHandle** out_batches);

/* 1.1) Encode and compress spans into batches (synchronous).
      `data` is a protobuf-encoded ExportTraceServiceRequest.
      - On success returns GENEVA_SUCCESS and writes *out_batches.
      - On failure returns an error code.
      Caller must free *out_batches with geneva_batches_free. */
GenevaError geneva_encode_and_compress_spans(GenevaClientHandle* handle,
                                            const uint8_t* data,
                                            size_t data_len,
                                            EncodedBatchesHandle** out_batches);

// 2) Query number of batches.
size_t geneva_batches_len(const EncodedBatchesHandle* batches);

/* 3) Upload a single batch by index (synchronous).
      - On success returns GENEVA_SUCCESS.
      - On failure returns an error code. */
GenevaError geneva_upload_batch_sync(GenevaClientHandle* handle,
                                     const EncodedBatchesHandle* batches,
                                     size_t index);


/* 5) Free the batches handle. */
void geneva_batches_free(EncodedBatchesHandle* batches);



/* Frees a Geneva client handle */
void geneva_client_free(GenevaClientHandle* handle);


#ifdef __cplusplus
}
#endif

#endif // GENEVA_FFI_H
