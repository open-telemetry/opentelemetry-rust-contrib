// Copyright (c) Microsoft Corporation. All rights reserved.
// Licensed under the MIT license.

#ifndef RUST_MSI_TOKEN_WRAPPER_H
#define RUST_MSI_TOKEN_WRAPPER_H

#ifdef __cplusplus
extern "C" {
#endif

// Platform-specific types
#ifdef _WIN32
typedef long XPLATRESULT;
#else
typedef int XPLATRESULT;
#endif

// Error codes (from XPlatErrors.h)
extern const XPLATRESULT XPLAT_NO_ERROR;
extern const XPLATRESULT XPLAT_FAIL;
extern const XPLATRESULT XPLAT_INITIALIZED;
extern const XPLATRESULT XPLAT_INITIALIZATION_FAILED;
extern const XPLATRESULT XPLAT_AZURE_MSI_FAILED;
extern const XPLATRESULT XPLAT_ARC_MSI_FAILED;
extern const XPLATRESULT XPLAT_ANTMDS_MSI_FAILED;
extern const XPLATRESULT XPLAT_IMDS_ENDPOINT_ERROR;

// Endpoint types
typedef enum {
    Custom_Endpoint = 0,
    ARC_Endpoint = 1,
    Azure_Endpoint = 2,
    AntMds_Endpoint = 3
} ImdsEndpointType;

// Simple wrapper for the existing C function
XPLATRESULT rust_get_msi_access_token(
    const char* resource,
    const char* managed_id_identifier,
    const char* managed_id_value,
    bool is_ant_mds,
    char** token
);

// Create MSI Token Source
void* rust_create_imsi_token_source(void);

// Initialize MSI Token Source
XPLATRESULT rust_imsi_token_source_initialize(
    void* token_source,
    const char* resource,
    const char* managed_id_identifier,
    const char* managed_id_value,
    bool fallback_to_default,
    bool is_ant_mds
);

// Get Access Token
XPLATRESULT rust_imsi_token_source_get_access_token(
    void* token_source,
    bool force_refresh,
    char** access_token
);

// Get Expires On Seconds
XPLATRESULT rust_imsi_token_source_get_expires_on_seconds(
    void* token_source,
    long int* expires_on_seconds
);

// Set IMDS Host Address
XPLATRESULT rust_imsi_token_source_set_imds_host_address(
    void* token_source,
    const char* host_address,
    int endpoint_type
);

// Get IMDS Host Address
XPLATRESULT rust_imsi_token_source_get_imds_host_address(
    void* token_source,
    char** host_address
);

// Stop Token Source
void rust_imsi_token_source_stop(void* token_source);

// Destroy Token Source
void rust_destroy_imsi_token_source(void* token_source);

// Free string allocated by the library
void rust_free_string(char* str);

#ifdef __cplusplus
}
#endif

#endif // RUST_MSI_TOKEN_WRAPPER_H
