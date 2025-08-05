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
    GENEVA_INTERNAL_ERROR = 5
} GenevaError;

// Creates a new Geneva client
// Returns opaque handle or NULL on error
GenevaClientHandle* geneva_client_new(const GenevaConfig* config);

// Uploads logs to Geneva
// data should be protobuf-encoded ResourceLogs data
GenevaError geneva_upload_logs(GenevaClientHandle* handle, const uint8_t* data, size_t data_len);

// Frees a Geneva client handle
void geneva_client_free(GenevaClientHandle* handle);

// Gets the last error message (for debugging)
// Returns a C string that should not be freed by the caller
const char* geneva_get_last_error(void);

#ifdef __cplusplus
}
#endif

#endif // GENEVA_FFI_H
