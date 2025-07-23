//! Low-level FFI bindings to the XPlatLib MSI token functionality

use std::os::raw::{c_char, c_int, c_long, c_void};

// Include the C++ wrapper header for MSI authentication
#[cfg(feature = "msi_auth")]
extern "C" {
    #[link_name = "src/msi/native/wrapper.h"]
    fn __include_wrapper_header();
}

// Platform-specific types
#[cfg(target_os = "windows")]
pub type XPLATRESULT = c_long;

#[cfg(not(target_os = "windows"))]
pub type XPLATRESULT = c_int;

/// IMDS Endpoint Types
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImdsEndpointType {
    /// Custom IMDS endpoint
    CustomEndpoint = 0,
    /// Azure Arc IMDS endpoint
    ArcEndpoint = 1,
    /// Standard Azure IMDS endpoint
    AzureEndpoint = 2,
    /// AntMds endpoint for specific environments
    AntMdsEndpoint = 3,
}

#[cfg(feature = "msi_auth")]
extern "C" {
    /// Error constants (these will be linked from the C++ library)
    pub static XPLAT_NO_ERROR: XPLATRESULT;
    pub static XPLAT_FAIL: XPLATRESULT;
    pub static XPLAT_INITIALIZED: XPLATRESULT;
    pub static XPLAT_INITIALIZATION_FAILED: XPLATRESULT;
    pub static XPLAT_AZURE_MSI_FAILED: XPLATRESULT;
    pub static XPLAT_ARC_MSI_FAILED: XPLATRESULT;
    pub static XPLAT_ANTMDS_MSI_FAILED: XPLATRESULT;
    pub static XPLAT_IMDS_ENDPOINT_ERROR: XPLATRESULT;

    /// Simple wrapper for getting MSI access token
    pub fn rust_get_msi_access_token(
        resource: *const c_char,
        managed_id_identifier: *const c_char,
        managed_id_value: *const c_char,
        is_ant_mds: bool,
        token: *mut *mut c_char,
    ) -> XPLATRESULT;

    /// Create MSI Token Source
    pub fn rust_create_imsi_token_source() -> *mut c_void;

    /// Initialize MSI Token Source
    pub fn rust_imsi_token_source_initialize(
        token_source: *mut c_void,
        resource: *const c_char,
        managed_id_identifier: *const c_char,
        managed_id_value: *const c_char,
        fallback_to_default: bool,
        is_ant_mds: bool,
    ) -> XPLATRESULT;

    /// Get Access Token
    pub fn rust_imsi_token_source_get_access_token(
        token_source: *mut c_void,
        force_refresh: bool,
        access_token: *mut *mut c_char,
    ) -> XPLATRESULT;

    /// Get Expires On Seconds
    pub fn rust_imsi_token_source_get_expires_on_seconds(
        token_source: *mut c_void,
        expires_on_seconds: *mut c_long,
    ) -> XPLATRESULT;

    /// Set IMDS Host Address
    pub fn rust_imsi_token_source_set_imds_host_address(
        token_source: *mut c_void,
        host_address: *const c_char,
        endpoint_type: c_int,
    ) -> XPLATRESULT;

    /// Get IMDS Host Address
    pub fn rust_imsi_token_source_get_imds_host_address(
        token_source: *mut c_void,
        host_address: *mut *mut c_char,
    ) -> XPLATRESULT;

    /// Stop Token Source
    pub fn rust_imsi_token_source_stop(token_source: *mut c_void);

    /// Destroy Token Source
    pub fn rust_destroy_imsi_token_source(token_source: *mut c_void);

    /// Free string allocated by the library
    pub fn rust_free_string(str: *mut c_char);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ptr;

    #[test]
    fn test_endpoint_type_values() {
        assert_eq!(ImdsEndpointType::CustomEndpoint as c_int, 0);
        assert_eq!(ImdsEndpointType::ArcEndpoint as c_int, 1);
        assert_eq!(ImdsEndpointType::AzureEndpoint as c_int, 2);
        assert_eq!(ImdsEndpointType::AntMdsEndpoint as c_int, 3);
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
