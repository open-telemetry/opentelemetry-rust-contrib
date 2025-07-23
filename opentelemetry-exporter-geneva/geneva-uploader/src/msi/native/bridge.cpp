// Copyright (c) Microsoft Corporation. All rights reserved.
// Licensed under the MIT license.

#include "IMSIToken.h"
#include "StringUtils.h"
#include "XPlatErrors.h"

extern "C" {

// Simple wrapper for the existing C function
XPLATRESULT rust_get_msi_access_token(
    const char* resource,
    const char* managed_id_identifier,
    const char* managed_id_value,
    bool is_ant_mds,
    char** token
) {
    return GetMSIAccessToken(resource, managed_id_identifier, managed_id_value, is_ant_mds, token);
}

// Create MSI Token Source
void* rust_create_imsi_token_source() {
    return CreateIMSITokenSource();
}

// Initialize MSI Token Source
XPLATRESULT rust_imsi_token_source_initialize(
    void* token_source,
    const char* resource,
    const char* managed_id_identifier,
    const char* managed_id_value,
    bool fallback_to_default,
    bool is_ant_mds
) {
    if (!token_source) return XPLAT_INITIALIZATION_FAILED;
    
    auto* source = static_cast<IMSITokenSource*>(token_source);
    xplat_string_t x_resource = XPlatUtils::string_to_string_t(std::string(resource ? resource : ""));
    xplat_string_t x_managed_id_identifier = XPlatUtils::string_to_string_t(std::string(managed_id_identifier ? managed_id_identifier : ""));
    xplat_string_t x_managed_id_value = XPlatUtils::string_to_string_t(std::string(managed_id_value ? managed_id_value : ""));
    
    return source->Initialize(x_resource, x_managed_id_identifier, x_managed_id_value, fallback_to_default, is_ant_mds);
}

// Get Access Token
XPLATRESULT rust_imsi_token_source_get_access_token(
    void* token_source,
    bool force_refresh,
    char** access_token
) {
    if (!token_source || !access_token) return XPLAT_FAIL;
    
    auto* source = static_cast<IMSITokenSource*>(token_source);
    xplat_string_t token;
    
    XPLATRESULT result = source->GetAccessToken(token, force_refresh);
    if (SUCCEEDED(result)) {
        std::string token_str = XPlatUtils::string_t_to_string(token);
        *access_token = new char[token_str.length() + 1];
        strcpy(*access_token, token_str.c_str());
    } else {
        *access_token = nullptr;
    }
    
    return result;
}

// Get Expires On Seconds
XPLATRESULT rust_imsi_token_source_get_expires_on_seconds(
    void* token_source,
    long int* expires_on_seconds
) {
    if (!token_source || !expires_on_seconds) return XPLAT_FAIL;
    
    auto* source = static_cast<IMSITokenSource*>(token_source);
    return source->GetExpiresOnSeconds(*expires_on_seconds);
}

// Set IMDS Host Address
XPLATRESULT rust_imsi_token_source_set_imds_host_address(
    void* token_source,
    const char* host_address,
    int endpoint_type
) {
    if (!token_source || !host_address) return XPLAT_FAIL;
    
    auto* source = static_cast<IMSITokenSource*>(token_source);
    xplat_string_t x_host_address = XPlatUtils::string_to_string_t(std::string(host_address));
    ImdsEndpointType x_endpoint_type = static_cast<ImdsEndpointType>(endpoint_type);
    
    return source->SetImdsHostAddress(x_host_address, x_endpoint_type);
}

// Get IMDS Host Address
XPLATRESULT rust_imsi_token_source_get_imds_host_address(
    void* token_source,
    char** host_address
) {
    if (!token_source || !host_address) return XPLAT_FAIL;
    
    auto* source = static_cast<IMSITokenSource*>(token_source);
    xplat_string_t address = source->GetImdsHostAddress();
    std::string address_str = XPlatUtils::string_t_to_string(address);
    
    *host_address = new char[address_str.length() + 1];
    strcpy(*host_address, address_str.c_str());
    
    return XPLAT_NO_ERROR;
}

// Stop Token Source
void rust_imsi_token_source_stop(void* token_source) {
    if (token_source) {
        auto* source = static_cast<IMSITokenSource*>(token_source);
        source->Stop();
    }
}

// Destroy Token Source
void rust_destroy_imsi_token_source(void* token_source) {
    if (token_source) {
        delete static_cast<IMSITokenSource*>(token_source);
    }
}

// Free string allocated by the library
void rust_free_string(char* str) {
    if (str) {
        delete[] str;
    }
}

} // extern "C"
