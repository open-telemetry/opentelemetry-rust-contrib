//! FFI bindings for geneva-uploader to be used from Go via CGO
//! 
//! This crate provides C-compatible functions that can be called from Go
//! to use the Geneva uploader functionality.

#![allow(unsafe_op_in_unsafe_fn)]
#![allow(unsafe_attr_outside_unsafe)]

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_uint};
use std::ptr;
use std::sync::{Arc, Mutex};
use tokio::runtime::Runtime;
use once_cell::sync::Lazy;

use geneva_uploader::client::{GenevaClient, GenevaClientConfig};
use geneva_uploader::AuthMethod;
use prost::Message;
use std::path::PathBuf;

/// Global shared Tokio runtime for efficiency
static RUNTIME: Lazy<Runtime> = Lazy::new(|| {
    Runtime::new().expect("Failed to create Tokio runtime")
});

/// Thread-safe error storage
static LAST_ERROR: Lazy<Mutex<Option<CString>>> = Lazy::new(|| Mutex::new(None));

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
}

/// Sets the last error message in a thread-safe way
fn set_last_error(msg: &str) {
    if let Ok(c_string) = CString::new(msg) {
        if let Ok(mut last_error) = LAST_ERROR.lock() {
            *last_error = Some(c_string);
        }
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
    
    unsafe {
        match CStr::from_ptr(ptr).to_str() {
            Ok(s) => Ok(s.to_string()),
            Err(_) => Err(format!("Invalid UTF-8 in field '{}'", field_name)),
        }
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
    if config.is_null() {
        set_last_error("Configuration pointer is null");
        return ptr::null_mut();
    }

    let config = unsafe { &*config };
    
    // Validate all required fields are non-null
    if let Err(err_msg) = unsafe { validate_required_config_fields(config) } {
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

    // Upload logs using the shared runtime
    match RUNTIME.block_on(handle.client.upload_logs(&[resource_logs])) {
        Ok(_) => GenevaError::Success,
        Err(e) => {
            set_last_error(&format!("Failed to upload logs to Geneva: {}", e));
            GenevaError::UploadFailed
        }
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
        let _ = unsafe { Box::from_raw(handle) };
    }
}

/// Gets the last error message (for debugging)
/// 
/// # Safety
/// - Returns a C string that should not be freed by the caller
/// - The returned string is valid until the next call to this function or set_last_error
#[no_mangle]
pub unsafe extern "C" fn geneva_get_last_error() -> *const c_char {
    match LAST_ERROR.lock() {
        Ok(last_error) => match last_error.as_ref() {
            Some(err) => err.as_ptr(),
            None => ptr::null(),
        },
        Err(_) => ptr::null(), // Mutex poisoned, return null
    }
}
