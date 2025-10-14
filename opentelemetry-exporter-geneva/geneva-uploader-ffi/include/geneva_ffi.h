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
#define GENEVA_AUTH_SYSTEM_MANAGED_IDENTITY 0
#define GENEVA_AUTH_CERTIFICATE 1
#define GENEVA_AUTH_WORKLOAD_IDENTITY 2
#define GENEVA_AUTH_USER_MANAGED_IDENTITY 3
#define GENEVA_AUTH_USER_MANAGED_IDENTITY_BY_OBJECT_ID 4
#define GENEVA_AUTH_USER_MANAGED_IDENTITY_BY_RESOURCE_ID 5

/* Configuration for certificate auth (valid only when auth_method == GENEVA_AUTH_CERTIFICATE) */
typedef struct {
    const char* cert_path;      /* Path to certificate file */
    const char* cert_password;  /* Certificate password */
} GenevaCertAuthConfig;

/* Configuration for Workload Identity auth (valid only when auth_method == GENEVA_AUTH_WORKLOAD_IDENTITY) */
typedef struct {
    const char* resource;       /* Azure AD resource URI (e.g., "https://monitor.azure.com") */
} GenevaWorkloadIdentityAuthConfig;

/* Configuration for User-assigned Managed Identity by client ID (valid only when auth_method == GENEVA_AUTH_USER_MANAGED_IDENTITY) */
typedef struct {
    const char* client_id;      /* Azure AD client ID */
} GenevaUserManagedIdentityAuthConfig;

/* Configuration for User-assigned Managed Identity by object ID (valid only when auth_method == GENEVA_AUTH_USER_MANAGED_IDENTITY_BY_OBJECT_ID) */
typedef struct {
    const char* object_id;      /* Azure AD object ID */
} GenevaUserManagedIdentityByObjectIdAuthConfig;

/* Configuration for User-assigned Managed Identity by resource ID (valid only when auth_method == GENEVA_AUTH_USER_MANAGED_IDENTITY_BY_RESOURCE_ID) */
typedef struct {
    const char* resource_id;    /* Azure resource ID */
} GenevaUserManagedIdentityByResourceIdAuthConfig;

/* Tagged union for auth-specific configuration.
   The active member is determined by 'auth_method' in GenevaConfig. */
typedef union {
    GenevaCertAuthConfig cert;                                              /* Valid when auth_method == GENEVA_AUTH_CERTIFICATE */
    GenevaWorkloadIdentityAuthConfig workload_identity;                     /* Valid when auth_method == GENEVA_AUTH_WORKLOAD_IDENTITY */
    GenevaUserManagedIdentityAuthConfig user_msi;                           /* Valid when auth_method == GENEVA_AUTH_USER_MANAGED_IDENTITY */
    GenevaUserManagedIdentityByObjectIdAuthConfig user_msi_objid;           /* Valid when auth_method == GENEVA_AUTH_USER_MANAGED_IDENTITY_BY_OBJECT_ID */
    GenevaUserManagedIdentityByResourceIdAuthConfig user_msi_resid;         /* Valid when auth_method == GENEVA_AUTH_USER_MANAGED_IDENTITY_BY_RESOURCE_ID */
} GenevaAuthConfig;

/* Configuration structure for Geneva client (C-compatible, tagged union) */
typedef struct {
    const char* endpoint;
    const char* environment;
    const char* account;
    const char* namespace_name;
    const char* region;
    uint32_t config_major_version;
    int32_t auth_method; /* 0 = System MSI, 1 = Certificate, 2 = Workload Identity, 3 = User MSI by client ID, 4 = User MSI by object ID, 5 = User MSI by resource ID */
    const char* tenant;
    const char* role_name;
    const char* role_instance;
    GenevaAuthConfig auth; /* Active member selected by auth_method */
    const char* msi_resource; /* Optional: MSI resource for auth methods 0, 3, 4, 5 (nullable) */
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
