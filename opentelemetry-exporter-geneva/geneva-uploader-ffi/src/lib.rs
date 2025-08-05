//! FFI bindings for geneva-uploader to be used from Go via CGO
//!
//! This crate provides C-compatible functions that can be called from Go
//! to use the Geneva uploader functionality.

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_uint};
use std::ptr;
use std::sync::Arc;
use tokio::runtime::Runtime;

use geneva_uploader::client::{GenevaClient, GenevaClientConfig};
use geneva_uploader::AuthMethod;
use prost::Message;
use std::path::PathBuf;

/// Opaque handle for GenevaClient
pub struct GenevaClientHandle {
    client: Arc<GenevaClient>,
    runtime: Runtime,
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
    pub cert_path: *const c_char,     // Path to certificate file
    pub cert_password: *const c_char, // Certificate password
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
}

/// Creates a new Geneva client
///
/// # Safety
/// - config must be a valid pointer to GenevaConfig
/// - All string fields in config must be valid null-terminated C strings
/// - Returns opaque handle or null on error
#[no_mangle]
pub unsafe extern "C" fn geneva_client_new(config: *const GenevaConfig) -> *mut GenevaClientHandle {
    if config.is_null() {
        return ptr::null_mut();
    }

    let config = &*config;

    // Convert C strings to Rust strings
    let endpoint = match CStr::from_ptr(config.endpoint).to_str() {
        Ok(s) => s.to_string(),
        Err(_) => return ptr::null_mut(),
    };

    let environment = match CStr::from_ptr(config.environment).to_str() {
        Ok(s) => s.to_string(),
        Err(_) => return ptr::null_mut(),
    };

    let account = match CStr::from_ptr(config.account).to_str() {
        Ok(s) => s.to_string(),
        Err(_) => return ptr::null_mut(),
    };

    let namespace = match CStr::from_ptr(config.namespace_name).to_str() {
        Ok(s) => s.to_string(),
        Err(_) => return ptr::null_mut(),
    };

    let region = match CStr::from_ptr(config.region).to_str() {
        Ok(s) => s.to_string(),
        Err(_) => return ptr::null_mut(),
    };

    let tenant = match CStr::from_ptr(config.tenant).to_str() {
        Ok(s) => s.to_string(),
        Err(_) => return ptr::null_mut(),
    };

    let role_name = match CStr::from_ptr(config.role_name).to_str() {
        Ok(s) => s.to_string(),
        Err(_) => return ptr::null_mut(),
    };

    let role_instance = match CStr::from_ptr(config.role_instance).to_str() {
        Ok(s) => s.to_string(),
        Err(_) => return ptr::null_mut(),
    };

    // Convert auth method
    let auth_method = match config.auth_method {
        0 => AuthMethod::ManagedIdentity,
        1 => {
            // For certificate auth, we need cert_path and cert_password
            let cert_path = if config.cert_path.is_null() {
                return ptr::null_mut();
            } else {
                match CStr::from_ptr(config.cert_path).to_str() {
                    Ok(s) => PathBuf::from(s),
                    Err(_) => return ptr::null_mut(),
                }
            };

            let cert_password = if config.cert_password.is_null() {
                return ptr::null_mut();
            } else {
                match CStr::from_ptr(config.cert_password).to_str() {
                    Ok(s) => s.to_string(),
                    Err(_) => return ptr::null_mut(),
                }
            };

            AuthMethod::Certificate {
                path: cert_path,
                password: cert_password,
            }
        }
        _ => return ptr::null_mut(),
    };

    // Set max concurrent uploads
    let max_concurrent_uploads = if config.max_concurrent_uploads < 0 {
        None
    } else {
        Some(config.max_concurrent_uploads as usize)
    };

    // Create Tokio runtime
    let runtime = match Runtime::new() {
        Ok(rt) => rt,
        Err(_) => return ptr::null_mut(),
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

    // Create Geneva client
    let client = match runtime.block_on(GenevaClient::new(geneva_config)) {
        Ok(client) => Arc::new(client),
        Err(_) => return ptr::null_mut(),
    };

    let handle = GenevaClientHandle { client, runtime };
    Box::into_raw(Box::new(handle))
}

/// Uploads logs to Geneva
///
/// # Safety
/// - handle must be a valid pointer returned by geneva_client_new
/// - data must be a valid pointer to protobuf-encoded ResourceLogs data
/// - data_len must be the correct length of the data
#[no_mangle]
pub unsafe extern "C" fn geneva_upload_logs(
    handle: *mut GenevaClientHandle,
    data: *const u8,
    data_len: usize,
) -> GenevaError {
    if handle.is_null() || data.is_null() {
        return GenevaError::InvalidData;
    }

    let handle = &*handle;
    let data_slice = std::slice::from_raw_parts(data, data_len);

    // Decode protobuf data
    let resource_logs = match prost::Message::decode(data_slice) {
        Ok(logs) => logs,
        Err(_) => return GenevaError::InvalidData,
    };

    // Upload logs
    match handle
        .runtime
        .block_on(handle.client.upload_logs(&[resource_logs]))
    {
        Ok(_) => GenevaError::Success,
        Err(_) => GenevaError::UploadFailed,
    }
}

/// Frees a Geneva client handle
///
/// # Safety
/// - handle must be a valid pointer returned by geneva_client_new
/// - handle must not be used after calling this function
#[no_mangle]
pub unsafe extern "C" fn geneva_client_free(handle: *mut GenevaClientHandle) {
    if !handle.is_null() {
        let _ = Box::from_raw(handle);
    }
}

/// Gets the last error message (for debugging)
///
/// # Safety
/// - Returns a C string that should not be freed by the caller
/// - The returned string is valid until the next call to this function
#[no_mangle]
pub unsafe extern "C" fn geneva_get_last_error() -> *const c_char {
    static mut LAST_ERROR: Option<CString> = None;

    match &LAST_ERROR {
        Some(err) => err.as_ptr(),
        None => ptr::null(),
    }
}

/// Sets the last error message (internal use)
unsafe fn set_last_error(msg: &str) {
    static mut LAST_ERROR: Option<CString> = None;
    LAST_ERROR = CString::new(msg).ok();
}
