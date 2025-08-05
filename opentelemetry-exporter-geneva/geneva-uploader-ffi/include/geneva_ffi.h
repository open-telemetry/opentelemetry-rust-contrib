#ifndef GENEVA_FFI_H
#define GENEVA_FFI_H

#include <stdint.h>
#include <stdlib.h>

#ifdef __cplusplus
extern "C" {
#endif

// Opaque handle for GenevaClient
typedef struct GenevaClientHandle GenevaClientHandle;

// Authentication method constants
#define GENEVA_AUTH_MANAGED_IDENTITY 0
#define GENEVA_AUTH_CERTIFICATE 1

// Configuration structure for Geneva client (C-compatible)
typedef struct {
    const char* endpoint;
    const char* environment;
    const char* account;
    const char* namespace_name;
    const char* region;
    uint32_t config_major_version;
    int32_t auth_method; // 0 = Managed Identity, 1 = Certificate
    const char* tenant;
    const char* role_name;
    const char* role_instance;
    int32_t max_concurrent_uploads; // -1 for default
    // Certificate auth fields (only used when auth_method == 1)
    const char* cert_path;      // Path to certificate file
    const char* cert_password;  // Certificate password
} GenevaConfig;

// Error codes returned by FFI functions
typedef enum {
    GENEVA_SUCCESS = 0,
    GENEVA_INVALID_CONFIG = 1,
    GENEVA_INITIALIZATION_FAILED = 2,
    GENEVA_UPLOAD_FAILED = 3,
    GENEVA_INVALID_DATA = 4,
    GENEVA_INTERNAL_ERROR = 5,
    GENEVA_ASYNC_OPERATION_PENDING = 6
} GenevaError;

// Callback function type for async upload completion
// Parameters: error_code, user_data
typedef void (*UploadCallback)(GenevaError error_code, void* user_data);

// Creates a new Geneva client
// Returns opaque handle or NULL on error
GenevaClientHandle* geneva_client_new(const GenevaConfig* config);

// Uploads logs to Geneva synchronously (blocks until complete)
// data should be protobuf-encoded ResourceLogs data
// Note: This function blocks the calling thread. For high-performance scenarios,
// consider using geneva_upload_logs_async instead.
GenevaError geneva_upload_logs_sync(GenevaClientHandle* handle, const uint8_t* data, size_t data_len);

// Uploads logs to Geneva asynchronously with callback notification
// data should be protobuf-encoded ResourceLogs data
// callback will be called when the operation completes
// user_data will be passed to the callback
// Returns GENEVA_ASYNC_OPERATION_PENDING if queued successfully, or error code for immediate failures
GenevaError geneva_upload_logs_async(GenevaClientHandle* handle, const uint8_t* data, size_t data_len, 
                                     UploadCallback callback, void* user_data);

// Main upload function - non-blocking with callback
// data should be protobuf-encoded ResourceLogs data
// callback will be called when the operation completes
// user_data will be passed to the callback
GenevaError geneva_upload_logs(GenevaClientHandle* handle, const uint8_t* data, size_t data_len,
                               UploadCallback callback, void* user_data);

// Frees a Geneva client handle
void geneva_client_free(GenevaClientHandle* handle);

// Gets the last error message (for debugging)
// Returns a C string that should not be freed by the caller
const char* geneva_get_last_error(void);

#ifdef __cplusplus
}
#endif

#endif // GENEVA_FFI_H
