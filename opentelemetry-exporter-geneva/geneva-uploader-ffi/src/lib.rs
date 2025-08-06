//! FFI bindings for geneva-uploader to be used from Go via CGO
//! 
//! This crate provides C-compatible functions that can be called from Go
//! to use the Geneva uploader functionality.

#![allow(unsafe_op_in_unsafe_fn)]
#![allow(unsafe_attr_outside_unsafe)]

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_uint};
use std::ptr;
use std::sync::Arc;
use std::cell::RefCell;
use tokio::runtime::Runtime;
use once_cell::sync::Lazy;

use geneva_uploader::client::{GenevaClient, GenevaClientConfig};
use geneva_uploader::AuthMethod;
use prost::Message;
use std::path::PathBuf;

/// Global shared Tokio runtime for efficiency and Geneva client compatibility
/// Benefits:
/// - Geneva client is designed for concurrent operations (buffer_unordered)
/// - Thread-agnostic: FFI callers can use from any thread
/// - Optimal resource usage: single runtime shared across all clients
/// - High performance: multi-threaded runtime supports Geneva's concurrent uploads
static RUNTIME: Lazy<Runtime> = Lazy::new(|| {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4))
        .thread_name("geneva-ffi-worker")
        .enable_all() // TODO: Consider enabling only time + net for Geneva's needs
        .build()
        .expect("Failed to create Tokio runtime for Geneva FFI")
});

// Thread-local error storage - eliminates mutex contention and provides better errno semantics
// Benefits:
// - Zero synchronization overhead - each thread has isolated storage
// - Better C library semantics - matches errno behavior (per-thread)
// - Natural isolation - thread A errors don't overwrite thread B errors
// - Automatic cleanup on thread destruction
thread_local! {
    static THREAD_LOCAL_ERROR: RefCell<Option<CString>> = RefCell::new(None);
}

/// Opaque handle for GenevaClient
pub struct GenevaClientHandle {
    client: Arc<GenevaClient>,
}

/// Configuration structure for Geneva client (C-compatible)
#[repr(C)]
pub struct GenevaConfig {
    pub endpoint: *const c_char,
    pub environment: *const c_char,
    pub account: *const c_char,
    pub namespace_name: *const c_char,
    pub region: *const c_char,
    pub config_major_version: c_uint,
    pub auth_method: c_int, // 0 = ManagedIdentity, 1 = Certificate
    pub tenant: *const c_char,
    pub role_name: *const c_char,
    pub role_instance: *const c_char,
    pub max_concurrent_uploads: c_int, // -1 for default
    // Certificate auth fields (only used when auth_method == 1)
    pub cert_path: *const c_char,      // Path to certificate file
    pub cert_password: *const c_char,  // Certificate password
}

/// Error codes returned by FFI functions
#[repr(C)]
pub enum GenevaError {
    Success = 0,
    InvalidConfig = 1,
    InitializationFailed = 2,
    UploadFailed = 3,
    InvalidData = 4,
    InternalError = 5,
    AsyncOperationPending = 6,
}

/// Callback function type for async upload completion
/// Parameters: error_code, user_data
pub type UploadCallback = unsafe extern "C" fn(GenevaError, *mut std::ffi::c_void);

/// Wrapper for Send-safe pointer passing in FFI callbacks
/// This is safe because FFI callbacks are designed to handle pointers across thread boundaries
#[derive(Clone, Copy)]
struct SendPtr(*mut std::ffi::c_void);

unsafe impl Send for SendPtr {}
unsafe impl Sync for SendPtr {}

impl SendPtr {
    fn new(ptr: *mut std::ffi::c_void) -> Self {
        Self(ptr)
    }
    
    fn as_ptr(self) -> *mut std::ffi::c_void {
        self.0
    }
}

/// Wrapper for Send-safe callback function passing in FFI
/// This is safe because FFI callbacks are designed to be called from any thread
#[derive(Clone, Copy)]
struct SendCallback(UploadCallback);

unsafe impl Send for SendCallback {}
unsafe impl Sync for SendCallback {}

impl SendCallback {
    fn new(callback: UploadCallback) -> Self {
        Self(callback)
    }
    
    fn call(self, error_code: GenevaError, user_data: *mut std::ffi::c_void) {
        unsafe { (self.0)(error_code, user_data) }
    }
}

/// Sets the last error message using thread-local storage (lock-free)
fn set_last_error(msg: &str) {
    if let Ok(c_string) = CString::new(msg) {
        THREAD_LOCAL_ERROR.with(|error| {
            *error.borrow_mut() = Some(c_string);
        });
    }
}

/// Validates that all required configuration fields are non-null
unsafe fn validate_required_config_fields(config: &GenevaConfig) -> Result<(), &'static str> {
    if config.endpoint.is_null() {
        return Err("Required field 'endpoint' is null");
    }
    if config.environment.is_null() {
        return Err("Required field 'environment' is null");
    }
    if config.account.is_null() {
        return Err("Required field 'account' is null");
    }
    if config.namespace_name.is_null() {
        return Err("Required field 'namespace_name' is null");
    }
    if config.region.is_null() {
        return Err("Required field 'region' is null");
    }
    if config.tenant.is_null() {
        return Err("Required field 'tenant' is null");
    }
    if config.role_name.is_null() {
        return Err("Required field 'role_name' is null");
    }
    if config.role_instance.is_null() {
        return Err("Required field 'role_instance' is null");
    }
    Ok(())
}

/// Safely converts a C string to Rust String with error context
unsafe fn c_str_to_string(ptr: *const c_char, field_name: &str) -> Result<String, String> {
    if ptr.is_null() {
        return Err(format!("Field '{}' is null", field_name));
    }
    
    match CStr::from_ptr(ptr).to_str() {
        Ok(s) => Ok(s.to_string()),
        Err(_) => Err(format!("Invalid UTF-8 in field '{}'", field_name)),
    }
}

/// Creates a new Geneva client
/// 
/// # Safety
/// - config must be a valid pointer to GenevaConfig
/// - All string fields in config must be valid null-terminated C strings
/// - Returns opaque handle or null on error
#[no_mangle]
pub unsafe extern "C" fn geneva_client_new(config: *const GenevaConfig) -> *mut GenevaClientHandle {
    // Safely dereference the config pointer with null check
    if config.is_null() {
        set_last_error("Configuration pointer is null");
        return ptr::null_mut();
    }
    let config = unsafe { &*config };
    
    // Validate all required fields are non-null
    if let Err(err_msg) = validate_required_config_fields(config) {
        set_last_error(err_msg);
        return ptr::null_mut();
    }

    // Convert C strings to Rust strings with detailed error messages
    let endpoint = match unsafe { c_str_to_string(config.endpoint, "endpoint") } {
        Ok(s) => s,
        Err(err) => {
            set_last_error(&err);
            return ptr::null_mut();
        }
    };
    
    let environment = match unsafe { c_str_to_string(config.environment, "environment") } {
        Ok(s) => s,
        Err(err) => {
            set_last_error(&err);
            return ptr::null_mut();
        }
    };
    
    let account = match unsafe { c_str_to_string(config.account, "account") } {
        Ok(s) => s,
        Err(err) => {
            set_last_error(&err);
            return ptr::null_mut();
        }
    };
    
    let namespace = match unsafe { c_str_to_string(config.namespace_name, "namespace_name") } {
        Ok(s) => s,
        Err(err) => {
            set_last_error(&err);
            return ptr::null_mut();
        }
    };
    
    let region = match unsafe { c_str_to_string(config.region, "region") } {
        Ok(s) => s,
        Err(err) => {
            set_last_error(&err);
            return ptr::null_mut();
        }
    };
    
    let tenant = match unsafe { c_str_to_string(config.tenant, "tenant") } {
        Ok(s) => s,
        Err(err) => {
            set_last_error(&err);
            return ptr::null_mut();
        }
    };
    
    let role_name = match unsafe { c_str_to_string(config.role_name, "role_name") } {
        Ok(s) => s,
        Err(err) => {
            set_last_error(&err);
            return ptr::null_mut();
        }
    };
    
    let role_instance = match unsafe { c_str_to_string(config.role_instance, "role_instance") } {
        Ok(s) => s,
        Err(err) => {
            set_last_error(&err);
            return ptr::null_mut();
        }
    };

    // Convert auth method with validation
    let auth_method = match config.auth_method {
        0 => AuthMethod::ManagedIdentity,
        1 => {
            // For certificate auth, validate cert_path and cert_password
            if config.cert_path.is_null() {
                set_last_error("Certificate path is required for certificate authentication");
                return ptr::null_mut();
            }
            if config.cert_password.is_null() {
                set_last_error("Certificate password is required for certificate authentication");
                return ptr::null_mut();
            }
            
            let cert_path = match unsafe { c_str_to_string(config.cert_path, "cert_path") } {
                Ok(s) => PathBuf::from(s),
                Err(err) => {
                    set_last_error(&err);
                    return ptr::null_mut();
                }
            };
            
            let cert_password = match unsafe { c_str_to_string(config.cert_password, "cert_password") } {
                Ok(s) => s,
                Err(err) => {
                    set_last_error(&err);
                    return ptr::null_mut();
                }
            };
            
            AuthMethod::Certificate {
                path: cert_path,
                password: cert_password,
            }
        },
        _ => {
            set_last_error(&format!("Invalid auth_method: {}. Must be 0 (ManagedIdentity) or 1 (Certificate)", config.auth_method));
            return ptr::null_mut();
        }
    };

    // Validate and set max concurrent uploads
    let max_concurrent_uploads = if config.max_concurrent_uploads < 0 {
        None
    } else if config.max_concurrent_uploads == 0 {
        set_last_error("max_concurrent_uploads cannot be 0. Use -1 for default or positive value");
        return ptr::null_mut();
    } else {
        Some(config.max_concurrent_uploads as usize)
    };

    // Create Geneva client config
    let geneva_config = GenevaClientConfig {
        endpoint,
        environment,
        account,
        namespace,
        region,
        config_major_version: config.config_major_version,
        auth_method,
        tenant,
        role_name,
        role_instance,
        max_concurrent_uploads,
    };

    // Create Geneva client using the shared runtime
    let client = match RUNTIME.block_on(GenevaClient::new(geneva_config)) {
        Ok(client) => Arc::new(client),
        Err(e) => {
            set_last_error(&format!("Failed to create Geneva client: {}", e));
            return ptr::null_mut();
        }
    };

    let handle = GenevaClientHandle { client };
    Box::into_raw(Box::new(handle))
}

/// Uploads logs to Geneva synchronously (blocks until complete)
/// 
/// # Safety
/// - handle must be a valid pointer returned by geneva_client_new
/// - data must be a valid pointer to protobuf-encoded ResourceLogs data
/// - data_len must be the correct length of the data
/// 
/// # Note
/// This function blocks the calling thread. For high-performance scenarios,
/// consider using geneva_upload_logs_async instead.
#[no_mangle]
pub unsafe extern "C" fn geneva_upload_logs_sync(
    handle: *mut GenevaClientHandle,
    data: *const u8,
    data_len: usize,
) -> GenevaError {
    if handle.is_null() {
        set_last_error("Geneva client handle is null");
        return GenevaError::InvalidData;
    }
    
    if data.is_null() {
        set_last_error("Data pointer is null");
        return GenevaError::InvalidData;
    }
    
    if data_len == 0 {
        set_last_error("Data length is zero");
        return GenevaError::InvalidData;
    }

    let handle = unsafe { &*handle };
    let data_slice = unsafe { std::slice::from_raw_parts(data, data_len) };

    // Decode protobuf data
    let resource_logs = match Message::decode(data_slice) {
        Ok(logs) => logs,
        Err(e) => {
            set_last_error(&format!("Failed to decode protobuf data: {}", e));
            return GenevaError::InvalidData;
        }
    };

    // Upload logs using the shared runtime (blocking)
    match RUNTIME.block_on(handle.client.upload_logs(&[resource_logs])) {
        Ok(_) => GenevaError::Success,
        Err(e) => {
            set_last_error(&format!("Failed to upload logs to Geneva: {}", e));
            GenevaError::UploadFailed
        }
    }
}

/// Uploads logs to Geneva asynchronously with callback notification (main function)
/// 
/// # Safety
/// - handle must be a valid pointer returned by geneva_client_new
/// - data must be a valid pointer to protobuf-encoded ResourceLogs data
/// - data_len must be the correct length of the data
/// - callback will be called when the operation completes
/// - user_data will be passed to the callback
/// 
/// # Returns
/// - GenevaError::AsyncOperationPending if the upload was queued successfully
/// - Other error codes for immediate validation failures
#[no_mangle]
pub unsafe extern "C" fn geneva_upload_logs(
    handle: *mut GenevaClientHandle,
    data: *const u8,
    data_len: usize,
    callback: UploadCallback,
    user_data: *mut std::ffi::c_void,
) -> GenevaError {
    if handle.is_null() {
        set_last_error("Geneva client handle is null");
        return GenevaError::InvalidData;
    }
    
    if data.is_null() {
        set_last_error("Data pointer is null");
        return GenevaError::InvalidData;
    }
    
    if data_len == 0 {
        set_last_error("Data length is zero");
        return GenevaError::InvalidData;
    }

    let handle = unsafe { &*handle };
    let data_slice = unsafe { std::slice::from_raw_parts(data, data_len) };

    // Decode protobuf data
    let resource_logs = match Message::decode(data_slice) {
        Ok(logs) => logs,
        Err(e) => {
            set_last_error(&format!("Failed to decode protobuf data: {}", e));
            return GenevaError::InvalidData;
        }
    };

    // Clone the client for the async task
    let client = handle.client.clone();
    
    // Wrap the callback and user_data pointer for safe transfer across threads
    let callback_wrapper = SendCallback::new(callback);
    let user_data_wrapper = SendPtr::new(user_data);
    
    // Spawn async task on the runtime
    RUNTIME.spawn(async move {
        let result = client.upload_logs(&[resource_logs]).await;
        
        // Determine the error code
        let error_code = match result {
            Ok(_) => GenevaError::Success,
            Err(_) => GenevaError::UploadFailed,
        };
        
        // Spawn callback on dedicated thread to avoid blocking the async runtime
        // and ensure thread safety.
        std::thread::spawn(move || {
            callback_wrapper.call(error_code, user_data_wrapper.as_ptr());
        });
    });

    GenevaError::AsyncOperationPending
}

/// Frees a Geneva client handle
/// 
/// # Safety
/// - handle must be a valid pointer returned by geneva_client_new
/// - handle must not be used after calling this function
#[no_mangle]
pub unsafe extern "C" fn geneva_client_free(handle: *mut GenevaClientHandle) {
    if !handle.is_null() {
        let _ = unsafe { Box::from_raw(handle) };
    }
}

/// Gets the last error message for the current thread (lock-free)
/// 
/// # Safety
/// - Returns a C string that should not be freed by the caller
/// - The returned string is valid until the next call to set_last_error on this thread
/// - Each thread has its own error storage (better errno semantics)
#[no_mangle]
pub unsafe extern "C" fn geneva_get_last_error() -> *const c_char {
    THREAD_LOCAL_ERROR.with(|error| {
        match error.borrow().as_ref() {
            Some(err) => err.as_ptr(),
            None => ptr::null(),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    #[test]
    fn test_geneva_client_new_with_null_config() {
        unsafe {
            let client = geneva_client_new(ptr::null());
            assert!(client.is_null(), "Client should be null for null config");
            
            // Check that error message was set
            let error_ptr = geneva_get_last_error();
            assert!(!error_ptr.is_null(), "Should have error message");
        }
    }

    // Dummy callback for testing
    unsafe extern "C" fn dummy_callback(_error_code: GenevaError, _user_data: *mut std::ffi::c_void) {
        // Do nothing - just for testing
    }

    #[test]
    fn test_geneva_upload_logs_with_null_handle() {
        unsafe {
            let data = vec![1, 2, 3, 4];
            let result = geneva_upload_logs(ptr::null_mut(), data.as_ptr(), data.len(), dummy_callback, ptr::null_mut());
            assert_eq!(result as u32, GenevaError::InvalidData as u32);
        }
    }

    #[test]
    fn test_geneva_upload_logs_with_null_data() {
        unsafe {
            let result = geneva_upload_logs(ptr::null_mut(), ptr::null(), 0, dummy_callback, ptr::null_mut());
            assert_eq!(result as u32, GenevaError::InvalidData as u32);
        }
    }

    #[test]
    fn test_geneva_upload_logs_with_zero_length() {
        unsafe {
            let data = vec![1, 2, 3, 4];
            let result = geneva_upload_logs(ptr::null_mut(), data.as_ptr(), 0, dummy_callback, ptr::null_mut());
            assert_eq!(result as u32, GenevaError::InvalidData as u32);
        }
    }

    #[test]
    fn test_geneva_client_free_with_null() {
        unsafe {
            // Should not crash
            geneva_client_free(ptr::null_mut());
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

    #[test]
    fn test_certificate_auth_missing_cert_path() {
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
                auth_method: 1, // Certificate auth
                tenant: tenant.as_ptr(),
                role_name: role_name.as_ptr(),
                role_instance: role_instance.as_ptr(),
                max_concurrent_uploads: -1,
                cert_path: ptr::null(), // Missing cert path
                cert_password: ptr::null(),
            };

            let client = geneva_client_new(&config);
            assert!(client.is_null(), "Client should be null for missing cert path");
        }
    }

    #[test]
    fn test_certificate_auth_missing_cert_password() {
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

            let config = GenevaConfig {
                endpoint: endpoint.as_ptr(),
                environment: environment.as_ptr(),
                account: account.as_ptr(),
                namespace_name: namespace.as_ptr(),
                region: region.as_ptr(),
                config_major_version: 1,
                auth_method: 1, // Certificate auth
                tenant: tenant.as_ptr(),
                role_name: role_name.as_ptr(),
                role_instance: role_instance.as_ptr(),
                max_concurrent_uploads: -1,
                cert_path: cert_path.as_ptr(),
                cert_password: ptr::null(), // Missing cert password
            };

            let client = geneva_client_new(&config);
            assert!(client.is_null(), "Client should be null for missing cert password");
        }
    }

    #[test]
    fn test_geneva_upload_logs_sync_with_null_handle() {
        unsafe {
            let data = vec![1, 2, 3, 4];
            let result = geneva_upload_logs_sync(ptr::null_mut(), data.as_ptr(), data.len());
            assert_eq!(result as u32, GenevaError::InvalidData as u32);
        }
    }

    #[test]
    fn test_geneva_upload_logs_sync_with_null_data() {
        unsafe {
            let result = geneva_upload_logs_sync(ptr::null_mut(), ptr::null(), 0);
            assert_eq!(result as u32, GenevaError::InvalidData as u32);
        }
    }

    #[test]
    fn test_max_concurrent_uploads_zero() {
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
                auth_method: 0,
                tenant: tenant.as_ptr(),
                role_name: role_name.as_ptr(),
                role_instance: role_instance.as_ptr(),
                max_concurrent_uploads: 0, // Invalid - should be positive or -1
                cert_path: ptr::null(),
                cert_password: ptr::null(),
            };

            let client = geneva_client_new(&config);
            assert!(client.is_null(), "Client should be null for max_concurrent_uploads = 0");
        }
    }

    #[test]
    fn test_callback_function_signature() {
        // Test that the callback function signature is correct by compilation
        unsafe extern "C" fn test_callback(_error_code: GenevaError, _user_data: *mut std::ffi::c_void) {
            // This test just validates the callback signature compiles correctly
            // Real callback testing would require a valid client and network connection
        }

        unsafe {
            let data = vec![1, 2, 3, 4];
            
            // Test with null handle - should fail immediately with validation error
            let result = geneva_upload_logs(
                ptr::null_mut(), 
                data.as_ptr(), 
                data.len(), 
                test_callback, 
                ptr::null_mut()
            );
            
            // Should fail immediately with invalid data (null handle)
            assert_eq!(result as u32, GenevaError::InvalidData as u32);
        }
    }
}
