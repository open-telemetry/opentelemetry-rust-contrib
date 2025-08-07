//! Low-level FFI bindings to the XPlatLib MSI token functionality

use std::os::raw::{c_char, c_int, c_long, c_void};

// Include the C++ wrapper header for MSI authentication
#[cfg(feature = "msi_auth")]
unsafe extern "C" {
    #[link_name = "src/msi/native/wrapper.h"]
    fn __include_wrapper_header();
}

// Platform-specific types
#[cfg(target_os = "windows")]
pub type XPLATRESULT = c_long;

#[cfg(not(target_os = "windows"))]
pub type XPLATRESULT = c_int;

/// IMDS Endpoint Types (matching XPlatLib's ImdsEndpointType)
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImdsEndpointType {
    /// Custom IMDS endpoint
    CustomEndpoint = 0,
    /// Azure Arc IMDS endpoint
    ArcEndpoint = 1,
    /// Standard Azure IMDS endpoint
    AzureEndpoint = 2,
}

// When native MSI library is available, use external functions from XPlatLib
#[cfg(all(feature = "msi_auth", msi_native_available))]
unsafe extern "C" {
    /// Error constants from XPlatErrors.h
    pub static XPLAT_NO_ERROR: XPLATRESULT;
    pub static XPLAT_FAIL: XPLATRESULT;
    pub static XPLAT_INITIALIZED: XPLATRESULT;
    pub static XPLAT_INITIALIZATION_FAILED: XPLATRESULT;
    pub static XPLAT_AZURE_MSI_FAILED: XPLATRESULT;
    pub static XPLAT_ARC_MSI_FAILED: XPLATRESULT;
    pub static XPLAT_IMDS_ENDPOINT_ERROR: XPLATRESULT;

    /// Direct function from IMSIToken.h - GetMSIAccessToken
    pub fn GetMSIAccessToken(
        resource: *const c_char,
        managed_id_identifier: *const c_char,
        managed_id_value: *const c_char,
        token: *mut *mut c_char,
    ) -> XPLATRESULT;

    /// Factory function from IMSIToken.h - CreateIMSITokenSource
    pub fn CreateIMSITokenSource() -> *mut c_void;

    /// Factory function from IMSIToken.h - CreateIMSITokenInfo  
    pub fn CreateIMSITokenInfo() -> *mut c_void;
}

// C++ class method wrappers that will be implemented in bridge.cpp
#[cfg(all(feature = "msi_auth", msi_native_available))]
unsafe extern "C" {
    /// Initialize MSI Token Source (wrapper for IMSITokenSource::Initialize)
    pub fn imsi_token_source_initialize(
        token_source: *mut c_void,
        resource: *const c_char,
        managed_id_identifier: *const c_char,
        managed_id_value: *const c_char,
        fallback_to_default: bool,
    ) -> XPLATRESULT;

    /// Get Access Token (wrapper for IMSITokenSource::GetAccessToken)
    pub fn imsi_token_source_get_access_token(
        token_source: *mut c_void,
        access_token: *mut *mut c_char,
        force_refresh: bool,
    ) -> XPLATRESULT;

    /// Get Expires On Seconds (wrapper for IMSITokenSource::GetExpiresOnSeconds)
    pub fn imsi_token_source_get_expires_on_seconds(
        token_source: *mut c_void,
        expires_on_seconds: *mut c_long,
    ) -> XPLATRESULT;

    /// Set IMDS Host Address (wrapper for IMSITokenSource::SetImdsHostAddress)
    pub fn imsi_token_source_set_imds_host_address(
        token_source: *mut c_void,
        host_address: *const c_char,
        endpoint_type: c_int,
    ) -> XPLATRESULT;

    /// Get IMDS Host Address (wrapper for IMSITokenSource::GetImdsHostAddress)
    pub fn imsi_token_source_get_imds_host_address(
        token_source: *mut c_void,
        host_address: *mut *mut c_char,
    ) -> XPLATRESULT;

    /// Stop Token Source (wrapper for IMSITokenSource::Stop)
    pub fn imsi_token_source_stop(token_source: *mut c_void);

    /// Destroy Token Source (wrapper for delete operator)
    pub fn imsi_token_source_destroy(token_source: *mut c_void);

    /// Free string allocated by XPlatLib (using delete[])
    pub fn xplat_free_string(str: *mut c_char);
}

// When MSI feature is enabled but native library is not available, provide stub implementations
#[cfg(all(feature = "msi_auth", not(msi_native_available)))]
mod stub_implementations {
    use super::*;
    use std::ptr;

    // Error constants (stub implementations)
    pub static XPLAT_NO_ERROR: XPLATRESULT = 0;
    pub static XPLAT_FAIL: XPLATRESULT = -1;
    pub static XPLAT_INITIALIZED: XPLATRESULT = 1;
    pub static XPLAT_INITIALIZATION_FAILED: XPLATRESULT = -2;
    pub static XPLAT_AZURE_MSI_FAILED: XPLATRESULT = -3;
    pub static XPLAT_ARC_MSI_FAILED: XPLATRESULT = -4;
    pub static XPLAT_ANTMDS_MSI_FAILED: XPLATRESULT = -5;
    pub static XPLAT_IMDS_ENDPOINT_ERROR: XPLATRESULT = -6;

    /// Stub implementation for getting MSI access token
    #[no_mangle]
    pub unsafe extern "C" fn rust_get_msi_access_token(
        _resource: *const c_char,
        _managed_id_identifier: *const c_char,
        _managed_id_value: *const c_char,
        _is_ant_mds: bool,
        token: *mut *mut c_char,
    ) -> XPLATRESULT {
        if !token.is_null() {
            *token = ptr::null_mut();
        }
        XPLAT_AZURE_MSI_FAILED
    }

    /// Stub implementation for creating MSI Token Source
    #[no_mangle]
    pub unsafe extern "C" fn rust_create_imsi_token_source() -> *mut c_void {
        ptr::null_mut()
    }

    /// Stub implementation for initializing MSI Token Source
    #[no_mangle]
    pub unsafe extern "C" fn rust_imsi_token_source_initialize(
        _token_source: *mut c_void,
        _resource: *const c_char,
        _managed_id_identifier: *const c_char,
        _managed_id_value: *const c_char,
        _fallback_to_default: bool,
        _is_ant_mds: bool,
    ) -> XPLATRESULT {
        XPLAT_INITIALIZATION_FAILED
    }

    /// Stub implementation for getting access token
    #[no_mangle]
    pub unsafe extern "C" fn rust_imsi_token_source_get_access_token(
        _token_source: *mut c_void,
        _force_refresh: bool,
        access_token: *mut *mut c_char,
    ) -> XPLATRESULT {
        if !access_token.is_null() {
            *access_token = ptr::null_mut();
        }
        XPLAT_AZURE_MSI_FAILED
    }

    /// Stub implementation for getting expires on seconds
    #[no_mangle]
    pub unsafe extern "C" fn rust_imsi_token_source_get_expires_on_seconds(
        _token_source: *mut c_void,
        expires_on_seconds: *mut c_long,
    ) -> XPLATRESULT {
        if !expires_on_seconds.is_null() {
            *expires_on_seconds = 0;
        }
        XPLAT_FAIL
    }

    /// Stub implementation for setting IMDS host address
    #[no_mangle]
    pub unsafe extern "C" fn rust_imsi_token_source_set_imds_host_address(
        _token_source: *mut c_void,
        _host_address: *const c_char,
        _endpoint_type: c_int,
    ) -> XPLATRESULT {
        XPLAT_FAIL
    }

    /// Stub implementation for getting IMDS host address
    #[no_mangle]
    pub unsafe extern "C" fn rust_imsi_token_source_get_imds_host_address(
        _token_source: *mut c_void,
        host_address: *mut *mut c_char,
    ) -> XPLATRESULT {
        if !host_address.is_null() {
            *host_address = ptr::null_mut();
        }
        XPLAT_FAIL
    }

    /// Stub implementation for stopping token source
    #[no_mangle]
    pub unsafe extern "C" fn rust_imsi_token_source_stop(_token_source: *mut c_void) {
        // No-op for stub
    }

    /// Stub implementation for destroying token source
    #[no_mangle]
    pub unsafe extern "C" fn rust_destroy_imsi_token_source(_token_source: *mut c_void) {
        // No-op for stub
    }

    /// Stub implementation for freeing string
    #[no_mangle]
    pub unsafe extern "C" fn rust_free_string(_str: *mut c_char) {
        // No-op for stub (since we never allocate strings)
    }
}

// Re-export stub constants when native library is not available
#[cfg(all(feature = "msi_auth", not(msi_native_available)))]
pub use stub_implementations::*;

#[cfg(test)]
mod tests {
    use super::*;
    use std::ptr;

    #[test]
    fn test_endpoint_type_values() {
        assert_eq!(ImdsEndpointType::CustomEndpoint as c_int, 0);
        assert_eq!(ImdsEndpointType::ArcEndpoint as c_int, 1);
        assert_eq!(ImdsEndpointType::AzureEndpoint as c_int, 2);
    }

    #[test]
    fn test_null_pointers() {
        // Test that we can create null pointers safely
        let null_ptr: *mut c_void = ptr::null_mut();
        assert!(null_ptr.is_null());
        
        let null_char: *mut c_char = ptr::null_mut();
        assert!(null_char.is_null());
    }
}
