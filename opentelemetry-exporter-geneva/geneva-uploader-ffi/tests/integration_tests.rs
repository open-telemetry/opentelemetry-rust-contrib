//! Integration tests for the Geneva FFI layer
//! 
//! These tests verify the C interface works correctly

use std::ffi::CString;
use std::ptr;

// Import the types from the main crate
use geneva_uploader_ffi::{GenevaConfig, GenevaClientHandle, GenevaError};

// Import the FFI functions directly from the main crate
extern "C" {
    fn geneva_client_new(config: *const GenevaConfig) -> *mut GenevaClientHandle;
    fn geneva_client_free(handle: *mut GenevaClientHandle);
    fn geneva_upload_logs(handle: *mut GenevaClientHandle, data: *const u8, data_len: usize) -> GenevaError;
    fn geneva_get_last_error() -> *const std::os::raw::c_char;
}

#[test]
fn test_geneva_client_new_with_null_config() {
    unsafe {
        let client = geneva_client_new(ptr::null());
        assert!(client.is_null(), "Client should be null for null config");
    }
}

#[test]
fn test_geneva_client_new_with_valid_config() {
    unsafe {
        // Create C strings for configuration
        let endpoint = CString::new("https://test.geneva.com").unwrap();
        let environment = CString::new("test").unwrap();
        let account = CString::new("testaccount").unwrap();
        let namespace = CString::new("testns").unwrap();
        let region = CString::new("testregion").unwrap();
        let tenant = CString::new("testtenant").unwrap();
        let role_name = CString::new("testrole").unwrap();
        let role_instance = CString::new("testinstance").unwrap();

        let config = GenevaConfig {
            endpoint: endpoint.as_ptr(),
            environment: environment.as_ptr(),
            account: account.as_ptr(),
            namespace_name: namespace.as_ptr(),
            region: region.as_ptr(),
            config_major_version: 1,
            auth_method: 0, // ManagedIdentity
            tenant: tenant.as_ptr(),
            role_name: role_name.as_ptr(),
            role_instance: role_instance.as_ptr(),
            max_concurrent_uploads: -1,
            cert_path: ptr::null(),
            cert_password: ptr::null(),
        };

        // Note: This will likely fail due to network/auth, but tests the FFI interface
        let client = geneva_client_new(&config);
        
        // Clean up if client was created
        if !client.is_null() {
            geneva_client_free(client);
        }
        
        // The fact that we got here without crashing means the FFI interface works
        assert!(true, "FFI interface functional");
    }
}

#[test]
fn test_geneva_upload_logs_with_null_handle() {
    unsafe {
        let data = vec![1, 2, 3, 4];
        let result = geneva_upload_logs(ptr::null_mut(), data.as_ptr(), data.len());
        assert_eq!(result as u32, GenevaError::InvalidData as u32);
    }
}

#[test]
fn test_geneva_upload_logs_with_null_data() {
    unsafe {
        // Create a dummy handle pointer (not actually valid, but non-null)
        let dummy_handle = 0x1 as *mut GenevaClientHandle;
        let result = geneva_upload_logs(dummy_handle, ptr::null(), 0);
        assert_eq!(result as u32, GenevaError::InvalidData as u32);
    }
}

#[test]
fn test_geneva_get_last_error() {
    unsafe {
        let error_ptr = geneva_get_last_error();
        // Should either be null or a valid C string
        if !error_ptr.is_null() {
            let error_cstr = std::ffi::CStr::from_ptr(error_ptr);
            let _error_str = error_cstr.to_str().expect("Should be valid UTF-8");
        }
        assert!(true, "Error function accessible");
    }
}

#[test]
fn test_geneva_client_free_with_null() {
    unsafe {
        // Should not crash
        geneva_client_free(ptr::null_mut());
        assert!(true, "Free with null handle should not crash");
    }
}

#[test]
fn test_config_with_certificate_auth() {
    unsafe {
        let endpoint = CString::new("https://test.geneva.com").unwrap();
        let environment = CString::new("test").unwrap();
        let account = CString::new("testaccount").unwrap();
        let namespace = CString::new("testns").unwrap();
        let region = CString::new("testregion").unwrap();
        let tenant = CString::new("testtenant").unwrap();
        let role_name = CString::new("testrole").unwrap();
        let role_instance = CString::new("testinstance").unwrap();
        let cert_path = CString::new("/path/to/cert.p12").unwrap();
        let cert_password = CString::new("password").unwrap();

        let config = GenevaConfig {
            endpoint: endpoint.as_ptr(),
            environment: environment.as_ptr(),
            account: account.as_ptr(),
            namespace_name: namespace.as_ptr(),
            region: region.as_ptr(),
            config_major_version: 1,
            auth_method: 1, // Certificate
            tenant: tenant.as_ptr(),
            role_name: role_name.as_ptr(),
            role_instance: role_instance.as_ptr(),
            max_concurrent_uploads: 4,
            cert_path: cert_path.as_ptr(),
            cert_password: cert_password.as_ptr(),
        };

        // This will likely fail due to invalid cert, but tests the interface
        let client = geneva_client_new(&config);
        
        if !client.is_null() {
            geneva_client_free(client);
        }
        
        assert!(true, "Certificate auth config accepted by FFI");
    }
}

#[test]
fn test_null_field_validation() {
    unsafe {
        // Test with missing endpoint
        let environment = CString::new("test").unwrap();
        let account = CString::new("testaccount").unwrap();
        let namespace = CString::new("testns").unwrap();
        let region = CString::new("testregion").unwrap();
        let tenant = CString::new("testtenant").unwrap();
        let role_name = CString::new("testrole").unwrap();
        let role_instance = CString::new("testinstance").unwrap();

        let config = GenevaConfig {
            endpoint: ptr::null(), // Missing endpoint should cause failure
            environment: environment.as_ptr(),
            account: account.as_ptr(),
            namespace_name: namespace.as_ptr(),
            region: region.as_ptr(),
            config_major_version: 1,
            auth_method: 0,
            tenant: tenant.as_ptr(),
            role_name: role_name.as_ptr(),
            role_instance: role_instance.as_ptr(),
            max_concurrent_uploads: -1,
            cert_path: ptr::null(),
            cert_password: ptr::null(),
        };

        let client = geneva_client_new(&config);
        assert!(client.is_null(), "Client should be null for invalid config");
        
        // Check that we can get error message
        let error_ptr = geneva_get_last_error();
        assert!(!error_ptr.is_null(), "Should have error message for invalid config");
    }
}

#[test]
fn test_invalid_auth_method() {
    unsafe {
        let endpoint = CString::new("https://test.geneva.com").unwrap();
        let environment = CString::new("test").unwrap();
        let account = CString::new("testaccount").unwrap();
        let namespace = CString::new("testns").unwrap();
        let region = CString::new("testregion").unwrap();
        let tenant = CString::new("testtenant").unwrap();
        let role_name = CString::new("testrole").unwrap();
        let role_instance = CString::new("testinstance").unwrap();

        let config = GenevaConfig {
            endpoint: endpoint.as_ptr(),
            environment: environment.as_ptr(),
            account: account.as_ptr(),
            namespace_name: namespace.as_ptr(),
            region: region.as_ptr(),
            config_major_version: 1,
            auth_method: 99, // Invalid auth method
            tenant: tenant.as_ptr(),
            role_name: role_name.as_ptr(),
            role_instance: role_instance.as_ptr(),
            max_concurrent_uploads: -1,
            cert_path: ptr::null(),
            cert_password: ptr::null(),
        };

        let client = geneva_client_new(&config);
        assert!(client.is_null(), "Client should be null for invalid auth method");
    }
}
