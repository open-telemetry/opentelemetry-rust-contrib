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
   The active member is determined by 'auth_method' in GenevaConfig.

   NOTE: When auth_method is GENEVA_AUTH_SYSTEM_MANAGED_IDENTITY (0),
   the union is not accessed and can be zero-initialized. */
typedef union {
    GenevaCertAuthConfig cert;                                              /* Valid when auth_method == GENEVA_AUTH_CERTIFICATE */
    GenevaWorkloadIdentityAuthConfig workload_identity;                     /* Valid when auth_method == GENEVA_AUTH_WORKLOAD_IDENTITY */
    GenevaUserManagedIdentityAuthConfig user_msi;                           /* Valid when auth_method == GENEVA_AUTH_USER_MANAGED_IDENTITY */
    GenevaUserManagedIdentityByObjectIdAuthConfig user_msi_objid;           /* Valid when auth_method == GENEVA_AUTH_USER_MANAGED_IDENTITY_BY_OBJECT_ID */
    GenevaUserManagedIdentityByResourceIdAuthConfig user_msi_resid;         /* Valid when auth_method == GENEVA_AUTH_USER_MANAGED_IDENTITY_BY_RESOURCE_ID */
} GenevaAuthConfig;

/* Configuration structure for Geneva client (C-compatible, tagged union)
 *
 * IMPORTANT - Resource/Scope Configuration:
 * Different auth methods require different resource configuration:
 *
 * - SystemManagedIdentity (0): Requires msi_resource field
 * - Certificate (1): No resource needed (uses mTLS)
 * - WorkloadIdentity (2): Requires auth.workload_identity.resource field
 * - UserManagedIdentity by client ID (3): Requires msi_resource field
 * - UserManagedIdentity by object ID (4): Requires msi_resource field
 * - UserManagedIdentity by resource ID (5): Requires msi_resource field
 *
 * The msi_resource field specifies the Azure AD resource URI for token acquisition
 * (e.g., "https://monitor.azure.com" for Azure Monitor in Public Cloud).
 *
 * Note: For user-assigned identities (3, 4, 5), the auth struct specifies WHICH
 * identity to use (client_id/object_id/resource_id), while msi_resource specifies
 * WHAT Azure resource to request tokens FOR. These are separate concerns.
 */
typedef struct {
    const char* endpoint;
    const char* environment;
    const char* account;
    const char* namespace_name;
    const char* region;
    uint32_t config_major_version;
    uint32_t auth_method; /* 0 = System MSI, 1 = Certificate, 2 = Workload Identity, 3 = User MSI by client ID, 4 = User MSI by object ID, 5 = User MSI by resource ID */
    const char* tenant;
    const char* role_name;
    const char* role_instance;
    GenevaAuthConfig auth; /* Active member selected by auth_method */
    const char* msi_resource; /* Azure AD resource URI for MSI auth (auth methods 0, 3, 4, 5). Not used for auth methods 1, 2. Nullable. */
} GenevaConfig;

/* Create a new Geneva client.
   - On success returns GENEVA_SUCCESS and writes *out_handle.
   - On failure returns an error code and optionally writes diagnostic message to err_msg_out.

   Parameters:
   - config: Configuration structure (required)
   - out_handle: Receives the client handle on success (required)
   - err_msg_out: Optional buffer to receive error message (can be NULL).
                  Message will be NUL-terminated and truncated if buffer too small.
                  Recommended size: >= 256 bytes for full diagnostics.
   - err_msg_len: Size of err_msg_out buffer in bytes (ignored if err_msg_out is NULL)

   IMPORTANT: Caller must call geneva_client_free() on the returned handle
   to avoid memory leaks. All strings in config are copied; caller retains
   ownership of config strings and may free them after this call returns. */
GenevaError geneva_client_new(const GenevaConfig* config,
                              GenevaClientHandle** out_handle,
                              char* err_msg_out,
                              size_t err_msg_len);


/* 1) Encode and compress logs into batches (synchronous).
      `data` is a protobuf-encoded ExportLogsServiceRequest.
      - On success returns GENEVA_SUCCESS and writes *out_batches.
      - On failure returns an error code and optionally writes diagnostic message to err_msg_out.

      Parameters:
      - handle: Client handle from geneva_client_new (required)
      - data: Protobuf-encoded ExportLogsServiceRequest (required)
      - data_len: Length of data buffer (required)
      - out_batches: Receives the batches handle on success (required)
      - err_msg_out: Optional buffer to receive error message (can be NULL).
                     Message will be NUL-terminated and truncated if buffer too small.
                     Recommended size: >= 256 bytes.
      - err_msg_len: Size of err_msg_out buffer in bytes (ignored if err_msg_out is NULL)

      Caller must free *out_batches with geneva_batches_free. */
GenevaError geneva_encode_and_compress_logs(GenevaClientHandle* handle,
                                            const uint8_t* data,
                                            size_t data_len,
                                            EncodedBatchesHandle** out_batches,
                                            char* err_msg_out,
                                            size_t err_msg_len);

/* 1.1) Encode and compress spans into batches (synchronous).
      `data` is a protobuf-encoded ExportTraceServiceRequest.
      - On success returns GENEVA_SUCCESS and writes *out_batches.
      - On failure returns an error code and optionally writes diagnostic message to err_msg_out.

      Parameters:
      - handle: Client handle from geneva_client_new (required)
      - data: Protobuf-encoded ExportTraceServiceRequest (required)
      - data_len: Length of data buffer (required)
      - out_batches: Receives the batches handle on success (required)
      - err_msg_out: Optional buffer to receive error message (can be NULL).
                     Message will be NUL-terminated and truncated if buffer too small.
                     Recommended size: >= 256 bytes.
      - err_msg_len: Size of err_msg_out buffer in bytes (ignored if err_msg_out is NULL)

      Caller must free *out_batches with geneva_batches_free. */
GenevaError geneva_encode_and_compress_spans(GenevaClientHandle* handle,
                                            const uint8_t* data,
                                            size_t data_len,
                                            EncodedBatchesHandle** out_batches,
                                            char* err_msg_out,
                                            size_t err_msg_len);

// 2) Query number of batches.
size_t geneva_batches_len(const EncodedBatchesHandle* batches);

/* 3) Upload a single batch by index (synchronous).
      - On success returns GENEVA_SUCCESS.
      - On failure returns an error code and optionally writes diagnostic message to err_msg_out.

      Parameters:
      - handle: Client handle from geneva_client_new (required)
      - batches: Batches handle from encode/compress function (required)
      - index: Index of batch to upload (must be < geneva_batches_len(batches))
      - err_msg_out: Optional buffer to receive error message (can be NULL).
                     Message will be NUL-terminated and truncated if buffer too small.
                     Recommended size: >= 256 bytes.
      - err_msg_len: Size of err_msg_out buffer in bytes (ignored if err_msg_out is NULL) */
GenevaError geneva_upload_batch_sync(GenevaClientHandle* handle,
                                     const EncodedBatchesHandle* batches,
                                     size_t index,
                                     char* err_msg_out,
                                     size_t err_msg_len);


/* 5) Free the batches handle. */
void geneva_batches_free(EncodedBatchesHandle* batches);



/* Frees a Geneva client handle and all associated resources.

   IMPORTANT: This must be called for every handle returned by geneva_client_new()
   to avoid memory leaks. After calling this function, the handle must not be used.

   Safe to call with NULL (no-op). */
void geneva_client_free(GenevaClientHandle* handle);


#ifdef __cplusplus
}
#endif

#endif // GENEVA_FFI_H
