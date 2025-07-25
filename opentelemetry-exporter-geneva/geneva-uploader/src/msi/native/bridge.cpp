// Copyright (c) Microsoft Corporation. All rights reserved.
// Licensed under the MIT license.

#include <cstring>
#include "IMSIToken.h"
#include "StringUtils.h"
#include "XPlatErrors.h"
#include "ImdsEndpointFetcher.h"

extern "C" {

// C++ wrapper functions for Rust FFI to XPlatLib integration

// Initialize MSI Token Source (wrapper for IMSITokenSource::Initialize)
XPLATRESULT imsi_token_source_initialize(
    void* token_source,
    const char* resource,
    const char* managed_id_identifier,
    const char* managed_id_value,
    bool fallback_to_default
) {
    if (!token_source) return XPLAT_INITIALIZATION_FAILED;
    
    auto* source = static_cast<IMSITokenSource*>(token_source);
    xplat_string_t x_resource = XPlatUtils::string_to_string_t(std::string(resource ? resource : ""));
    xplat_string_t x_managed_id_identifier = XPlatUtils::string_to_string_t(std::string(managed_id_identifier ? managed_id_identifier : ""));
    xplat_string_t x_managed_id_value = XPlatUtils::string_to_string_t(std::string(managed_id_value ? managed_id_value : ""));
    
    return source->Initialize(x_resource, x_managed_id_identifier, x_managed_id_value, fallback_to_default);
}

// Get Access Token (wrapper for IMSITokenSource::GetAccessToken)
XPLATRESULT imsi_token_source_get_access_token(
    void* token_source,
    char** access_token,
    bool force_refresh
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

// Get Expires On Seconds (wrapper for IMSITokenSource::GetExpiresOnSeconds)
XPLATRESULT imsi_token_source_get_expires_on_seconds(
    void* token_source,
    long int* expires_on_seconds
) {
    if (!token_source || !expires_on_seconds) return XPLAT_FAIL;
    
    auto* source = static_cast<IMSITokenSource*>(token_source);
    return source->GetExpiresOnSeconds(*expires_on_seconds);
}

// Set IMDS Host Address (wrapper for IMSITokenSource::SetImdsHostAddress)
XPLATRESULT imsi_token_source_set_imds_host_address(
    void* token_source,
    const char* host_address,
    int endpoint_type
) {
    if (!token_source || !host_address) return XPLAT_FAIL;
    
    auto* source = static_cast<IMSITokenSource*>(token_source);
    xplat_string_t x_host_address = XPlatUtils::string_to_string_t(std::string(host_address));
    
    // Map from our enum values to XPlatLib enum values
    ImdsEndpointType x_endpoint_type;
    switch (endpoint_type) {
        case 0: x_endpoint_type = ImdsEndpointType::Custom_Endpoint; break;
        case 1: x_endpoint_type = ImdsEndpointType::ARC_Endpoint; break;
        case 2: x_endpoint_type = ImdsEndpointType::Azure_Endpoint; break;
        default: return XPLAT_FAIL;
    }
    
    return source->SetImdsHostAddress(x_host_address, x_endpoint_type);
}

// Get IMDS Host Address (wrapper for IMSITokenSource::GetImdsHostAddress)
XPLATRESULT imsi_token_source_get_imds_host_address(
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

// Stop Token Source (wrapper for IMSITokenSource::Stop)
void imsi_token_source_stop(void* token_source) {
    if (token_source) {
        auto* source = static_cast<IMSITokenSource*>(token_source);
        source->Stop();
    }
}

// Destroy Token Source (wrapper for delete operator)
void imsi_token_source_destroy(void* token_source) {
    if (token_source) {
        delete static_cast<IMSITokenSource*>(token_source);
    }
}

// Free string allocated by XPlatLib (using delete[])
void xplat_free_string(char* str) {
    if (str) {
        delete[] str;
    }
}

} // extern "C"
