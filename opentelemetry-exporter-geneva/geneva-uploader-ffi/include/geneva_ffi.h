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
      This symbol is only exported when the Rust library is built with the
      `otlp_bytes` feature. Calling code should enable the same feature when
      linking against a library that provides this entry point.
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


/* =========================================================================
 * Zero-copy log record path (no intermediate OTLP serialisation)
 * =========================================================================
 *
 * Use these types and geneva_encode_and_compress_log_records() when you
 * already have log records in memory and want to encode them directly to
 * Geneva Bond format without serialising to OTLP protobuf first.
 *
 * If your records carry meaningful resource or instrumentation-scope
 * attributes (service name, host, etc.) use the OTLP path instead:
 * geneva_encode_and_compress_logs().
 * =========================================================================
 */

/* Attribute value type tag — set GenevaAttrValueC.tag to one of these. */
#define GENEVA_ATTR_STRING 0
#define GENEVA_ATTR_INT64  1
#define GENEVA_ATTR_DOUBLE 2
#define GENEVA_ATTR_BOOL   3

/* Attribute value data.  Only the member matching the tag is read. */
typedef union {
    const char* str_val;   /* GENEVA_ATTR_STRING: null-terminated UTF-8 */
    int64_t     int64_val; /* GENEVA_ATTR_INT64  */
    double      double_val;/* GENEVA_ATTR_DOUBLE */
    uint8_t     bool_val;  /* GENEVA_ATTR_BOOL: 0 = false, else true */
} GenevaAttrData;

/* Tagged attribute value.  Set tag, then populate the matching data field. */
typedef struct {
    uint8_t       tag;  /* One of GENEVA_ATTR_* constants */
    GenevaAttrData data;
} GenevaAttrValueC;

/*
 * A single log record for zero-copy ingestion.
 *
 * Memory ownership
 * ----------------
 * Rust never takes ownership of any C memory.  All pointers are borrowed for
 * the duration of the geneva_encode_and_compress_log_records() call only.
 * After the call returns, every buffer may be freed or reused immediately.
 *
 * Zero-copy guarantee
 * -------------------
 * String fields (event_name, body, severity_text, attribute keys and string
 * values) are read directly from the pointers below — no intermediate heap
 * copy is made of the input data.  Fixed-size fields (trace_id, span_id,
 * numeric fields) are copied by value as normal struct field access.
 *
 * What does allocate
 * ------------------
 * The output (Bond-encoded + LZ4-compressed bytes) is heap-allocated inside
 * Rust and owned by the returned EncodedBatchesHandle.  Free it with
 * geneva_batches_free() when no longer needed.
 */
typedef struct {
    /* Event name (null-terminated). NULL or empty → default "Log". */
    const char* event_name;

    /* Primary timestamp (nanoseconds since Unix epoch). 0 = absent. */
    uint64_t time_unix_nano;

    /* Observation timestamp (nanoseconds since Unix epoch). 0 = absent. */
    uint64_t observed_time_unix_nano;

    /* OTLP severity number (0 = unspecified). */
    int32_t severity_number;

    /* Severity text (null-terminated). NULL = absent. */
    const char* severity_text;

    /* Log body as a null-terminated UTF-8 string. NULL = absent. */
    const char* body;

    /* 16-byte trace ID. Only used when trace_id_present != 0 and the ID is
       not all zeros. All-zero IDs are treated as absent to match OTLP view semantics. */
    uint8_t trace_id[16];
    uint8_t trace_id_present; /* Non-zero if trace_id is valid. */

    /* 8-byte span ID. Only used when span_id_present != 0 and the ID is
       not all zeros. All-zero IDs are treated as absent to match OTLP view semantics. */
    uint8_t span_id[8];
    uint8_t span_id_present;  /* Non-zero if span_id is valid. */

    /* Trace flags. Only used when flags_present != 0. */
    uint32_t flags;
    uint8_t  flags_present;   /* Non-zero if flags is meaningful. */

    /* Parallel attribute arrays of length attr_count.
       Pass NULL for both (and attr_count = 0) when there are no attributes. */
    const char* const*         attr_keys;   /* null-terminated attribute keys */
    const GenevaAttrValueC*    attr_values; /* one value per key              */
    size_t                     attr_count;
} GenevaLogRecordC;

/*
 * Encode a flat C array of log records into LZ4-compressed Geneva batches.
 *
 * This is the zero-copy path: records are read directly from the C array
 * without any intermediate OTLP serialisation.
 *
 * Parameters:
 *   handle        - Client handle from geneva_client_new (required).
 *   records       - Array of record_count initialised GenevaLogRecordC values.
 *                   All pointers inside each record must be valid for the
 *                   duration of this call.
 *   record_count  - Number of elements in records. Must be > 0.
 *   out_batches   - Receives a non-null EncodedBatchesHandle on success.
 *                   Free with geneva_batches_free() when done.
 *   err_msg_out   - Optional buffer for a diagnostic message (may be NULL).
 *   err_msg_len   - Byte capacity of err_msg_out (including NUL terminator).
 *
 * Returns GENEVA_SUCCESS on success; an error code otherwise.
 * Upload each batch with geneva_upload_batch_sync(), then free with
 * geneva_batches_free().
 */
GenevaError geneva_encode_and_compress_log_records(
    GenevaClientHandle*       handle,
    const GenevaLogRecordC*   records,
    size_t                    record_count,
    EncodedBatchesHandle**    out_batches,
    char*                     err_msg_out,
    size_t                    err_msg_len);

/* Layout assertions — must match the Rust-side compile-time checks.
   Only active on 64-bit targets where pointer size == 8. */
#if UINTPTR_MAX == 0xffffffffffffffffu
#include <stddef.h>
#include <assert.h>
static_assert(sizeof(GenevaAttrValueC)              == 16,  "GenevaAttrValueC size mismatch");
static_assert(sizeof(GenevaLogRecordC)              == 112, "GenevaLogRecordC size mismatch");
static_assert(offsetof(GenevaLogRecordC, attr_count)== 104, "GenevaLogRecordC attr_count offset mismatch");
#endif

#ifdef __cplusplus
}
#endif

#endif // GENEVA_FFI_H
