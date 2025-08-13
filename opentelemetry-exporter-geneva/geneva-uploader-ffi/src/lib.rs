//! C-compatible FFI bindings for geneva-uploader

// Allow #[repr(C)] and other FFI attributes without wrapping in unsafe blocks (standard FFI practice)
#![allow(unsafe_attr_outside_unsafe)]

use once_cell::sync::Lazy;
use std::ffi::CStr;
use std::os::raw::{c_char, c_int, c_uint};
use std::ptr;
use std::sync::Arc;
use tokio::runtime::Runtime;

use geneva_uploader::client::{EncodedBatch, GenevaClient, GenevaClientConfig};
use geneva_uploader::AuthMethod;
use opentelemetry_proto::tonic::collector::logs::v1::ExportLogsServiceRequest;
use prost::Message;
use std::path::PathBuf;

/// Magic number for handle validation
const GENEVA_HANDLE_MAGIC: u64 = 0xFEED_BEEF;

/// Shared Tokio runtime for async operations
/// TODO: Consider making runtime configurable via FFI in the future:
/// - Thread count configuration (currently uses available_parallelism())
/// - Runtime type selection (multi_thread vs current_thread)
/// - Per-client runtimes vs shared global runtime
/// - External runtime integration (accept user-provided runtime handle)
/// - Runtime lifecycle management for FFI (shutdown, cleanup)
static RUNTIME: Lazy<Runtime> = Lazy::new(|| {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(
            std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4),
        )
        .thread_name("geneva-ffi-worker")
        .enable_time()
        .enable_io() // Only enable time + I/O for Geneva's needs
        .build()
        .expect("Failed to create Tokio runtime for Geneva FFI")
});

/// Trait for handles that support validation
trait ValidatedHandle {
    fn magic(&self) -> u64;
    fn set_magic(&mut self, magic: u64);
}

/// Generic validation function that works for any ValidatedHandle
unsafe fn validate_handle<T: ValidatedHandle>(handle: *const T) -> GenevaError {
    if handle.is_null() {
        return GenevaError::NullPointer;
    }

    let handle_ref = match unsafe { handle.as_ref() } {
        Some(h) => h,
        None => return GenevaError::NullPointer,
    };

    if handle_ref.magic() != GENEVA_HANDLE_MAGIC {
        return GenevaError::InvalidData;
    }

    GenevaError::Success
}

/// Generic function to clear magic number on free
unsafe fn clear_handle_magic<T: ValidatedHandle>(handle: *mut T) {
    if !handle.is_null() {
        if let Some(h) = unsafe { handle.as_mut() } {
            h.set_magic(0);
        }
    }
}

/// Opaque handle for GenevaClient
pub struct GenevaClientHandle {
    magic: u64, // Magic number for handle validation
    client: Arc<GenevaClient>,
}

impl ValidatedHandle for GenevaClientHandle {
    fn magic(&self) -> u64 {
        self.magic
    }

    fn set_magic(&mut self, magic: u64) {
        self.magic = magic;
    }
}

/// Opaque handle holding encoded batches
pub struct EncodedBatchesHandle {
    magic: u64,
    batches: Vec<EncodedBatch>,
}

impl ValidatedHandle for EncodedBatchesHandle {
    fn magic(&self) -> u64 {
        self.magic
    }

    fn set_magic(&mut self, magic: u64) {
        self.magic = magic;
    }
}

/// Configuration for certificate auth (valid only when auth_method == 1)
#[repr(C)]
#[derive(Copy, Clone)]
pub struct GenevaCertAuthConfig {
    pub cert_path: *const c_char,     // Path to certificate file
    pub cert_password: *const c_char, // Certificate password
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct GenevaMSIAuthConfig {
    pub objid: *const c_char, // Optional: Azure AD object id; reserved for future use
}

#[repr(C)]
pub union GenevaAuthConfig {
    pub msi: GenevaMSIAuthConfig,   // Valid when auth_method == 0
    pub cert: GenevaCertAuthConfig, // Valid when auth_method == 1
}

/// Configuration structure for Geneva client (C-compatible, tagged union)
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
    pub auth: GenevaAuthConfig, // Active member selected by auth_method
}

/// Error codes returned by FFI functions
/// TODO: Use cbindgen to auto-generate geneva_errors.h from this enum to eliminate duplication
#[repr(C)]
#[derive(PartialEq)]
pub enum GenevaError {
    // Base codes (stable)
    Success = 0,
    InvalidConfig = 1,
    InitializationFailed = 2,
    UploadFailed = 3,
    InvalidData = 4,
    InternalError = 5,

    // Granular argument/data errors (used)
    NullPointer = 100,
    EmptyInput = 101,
    DecodeFailed = 102,
    IndexOutOfRange = 103,

    // Granular config/auth errors (used)
    InvalidAuthMethod = 110,
    InvalidCertConfig = 111,

    // Missing required config (granular INVALID_CONFIG)
    MissingEndpoint = 130,
    MissingEnvironment = 131,
    MissingAccount = 132,
    MissingNamespace = 133,
    MissingRegion = 134,
    MissingTenant = 135,
    MissingRoleName = 136,
    MissingRoleInstance = 137,
}

/// Validates that all required configuration fields are non-null
#[allow(dead_code)]
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

/// Safely converts a C string to Rust String
unsafe fn c_str_to_string(ptr: *const c_char, field_name: &str) -> Result<String, String> {
    if ptr.is_null() {
        return Err(format!("Field '{field_name}' is null"));
    }

    match unsafe { CStr::from_ptr(ptr) }.to_str() {
        Ok(s) => Ok(s.to_string()),
        Err(_) => Err(format!("Invalid UTF-8 in field '{field_name}'")),
    }
}

/// Creates a new Geneva client with explicit result semantics (no TLS needed).
///
/// On success: returns GenevaError::Success and writes a non-null handle into *out_handle.
/// On failure: returns an error code and writes a short diagnostic message into err_msg if provided.
///
/// # Safety
/// - config must be a valid pointer to a GenevaConfig struct
/// - out_handle must be a valid pointer to receive the client handle
/// - caller must eventually call geneva_client_free on the returned handle
#[no_mangle]
pub unsafe extern "C" fn geneva_client_new(
    config: *const GenevaConfig,
    out_handle: *mut *mut GenevaClientHandle,
) -> GenevaError {
    // Validate pointers
    if config.is_null() || out_handle.is_null() {
        return GenevaError::NullPointer;
    }
    unsafe { *out_handle = ptr::null_mut() };

    let config = match unsafe { config.as_ref() } {
        Some(c) => c,
        None => return GenevaError::NullPointer,
    };

    // Validate required fields with granular error codes
    if config.endpoint.is_null() {
        return GenevaError::MissingEndpoint;
    }
    if config.environment.is_null() {
        return GenevaError::MissingEnvironment;
    }
    if config.account.is_null() {
        return GenevaError::MissingAccount;
    }
    if config.namespace_name.is_null() {
        return GenevaError::MissingNamespace;
    }
    if config.region.is_null() {
        return GenevaError::MissingRegion;
    }
    if config.tenant.is_null() {
        return GenevaError::MissingTenant;
    }
    if config.role_name.is_null() {
        return GenevaError::MissingRoleName;
    }
    if config.role_instance.is_null() {
        return GenevaError::MissingRoleInstance;
    }

    // Convert C strings to Rust strings
    let endpoint = match unsafe { c_str_to_string(config.endpoint, "endpoint") } {
        Ok(s) => s,
        Err(_e) => {
            return GenevaError::InvalidConfig;
        }
    };
    let environment = match unsafe { c_str_to_string(config.environment, "environment") } {
        Ok(s) => s,
        Err(_e) => {
            return GenevaError::InvalidConfig;
        }
    };
    let account = match unsafe { c_str_to_string(config.account, "account") } {
        Ok(s) => s,
        Err(_e) => {
            return GenevaError::InvalidConfig;
        }
    };
    let namespace = match unsafe { c_str_to_string(config.namespace_name, "namespace_name") } {
        Ok(s) => s,
        Err(_e) => {
            return GenevaError::InvalidConfig;
        }
    };
    let region = match unsafe { c_str_to_string(config.region, "region") } {
        Ok(s) => s,
        Err(_e) => {
            return GenevaError::InvalidConfig;
        }
    };
    let tenant = match unsafe { c_str_to_string(config.tenant, "tenant") } {
        Ok(s) => s,
        Err(_e) => {
            return GenevaError::InvalidConfig;
        }
    };
    let role_name = match unsafe { c_str_to_string(config.role_name, "role_name") } {
        Ok(s) => s,
        Err(_e) => {
            return GenevaError::InvalidConfig;
        }
    };
    let role_instance = match unsafe { c_str_to_string(config.role_instance, "role_instance") } {
        Ok(s) => s,
        Err(_e) => {
            return GenevaError::InvalidConfig;
        }
    };

    // Auth method conversion
    let auth_method = match config.auth_method {
        0 => AuthMethod::ManagedIdentity,
        1 => {
            // Certificate authentication: read fields from tagged union
            let cert = unsafe { config.auth.cert };
            if cert.cert_path.is_null() {
                return GenevaError::InvalidCertConfig;
            }
            if cert.cert_password.is_null() {
                return GenevaError::InvalidCertConfig;
            }
            let cert_path = match unsafe { c_str_to_string(cert.cert_path, "cert_path") } {
                Ok(s) => PathBuf::from(s),
                Err(_e) => {
                    return GenevaError::InvalidConfig;
                }
            };
            let cert_password =
                match unsafe { c_str_to_string(cert.cert_password, "cert_password") } {
                    Ok(s) => s,
                    Err(_e) => {
                        return GenevaError::InvalidConfig;
                    }
                };
            AuthMethod::Certificate {
                path: cert_path,
                password: cert_password,
            }
        }
        _ => {
            return GenevaError::InvalidAuthMethod;
        }
    };

    // Build client config
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
    };

    // Create client
    let client = match GenevaClient::new(geneva_config) {
        Ok(client) => Arc::new(client),
        Err(_e) => {
            return GenevaError::InitializationFailed;
        }
    };

    let handle = GenevaClientHandle {
        client,
        magic: GENEVA_HANDLE_MAGIC,
    };
    unsafe { *out_handle = Box::into_raw(Box::new(handle)) };
    GenevaError::Success
}

/// Encode and compress logs into batches (synchronous)
///
/// # Safety
/// - handle must be a valid pointer returned by geneva_client_new
/// - data must be a valid pointer to protobuf-encoded ExportLogsServiceRequest
/// - data_len must be the correct length of the data
/// - out_batches must be non-null; on success it receives a non-null pointer the caller must free with geneva_batches_free
#[no_mangle]
pub unsafe extern "C" fn geneva_encode_and_compress_logs(
    handle: *mut GenevaClientHandle,
    data: *const u8,
    data_len: usize,
    out_batches: *mut *mut EncodedBatchesHandle,
) -> GenevaError {
    if out_batches.is_null() {
        return GenevaError::NullPointer;
    }
    unsafe { *out_batches = ptr::null_mut() };

    if handle.is_null() {
        return GenevaError::NullPointer;
    }
    if data.is_null() {
        return GenevaError::NullPointer;
    }
    if data_len == 0 {
        return GenevaError::EmptyInput;
    }

    // Validate handle first
    let validation_result = unsafe { validate_handle(handle) };
    if validation_result != GenevaError::Success {
        return validation_result;
    }

    let handle_ref = match unsafe { handle.as_ref() } {
        Some(h) => h,
        None => return GenevaError::NullPointer,
    };
    let data_slice = unsafe { std::slice::from_raw_parts(data, data_len) };

    let logs_data: ExportLogsServiceRequest = match Message::decode(data_slice) {
        Ok(data) => data,
        Err(_e) => {
            return GenevaError::DecodeFailed;
        }
    };

    let resource_logs = logs_data.resource_logs;
    match handle_ref.client.encode_and_compress_logs(&resource_logs) {
        Ok(batches) => {
            let h = EncodedBatchesHandle {
                magic: GENEVA_HANDLE_MAGIC,
                batches,
            };
            unsafe { *out_batches = Box::into_raw(Box::new(h)) };
            GenevaError::Success
        }
        Err(_e) => GenevaError::InternalError,
    }
}

/// Returns the number of batches in the encoded batches handle
///
/// # Safety
/// - batches must be a valid pointer returned by geneva_encode_and_compress_logs, or null
#[no_mangle]
pub unsafe extern "C" fn geneva_batches_len(batches: *const EncodedBatchesHandle) -> usize {
    // Validate batches
    match unsafe { validate_handle(batches) } {
        GenevaError::Success => {
            // Safe to dereference after validation
            let batches_ref = unsafe { batches.as_ref().unwrap() };
            batches_ref.batches.len()
        }
        _ => 0, // Return 0 for invalid handles
    }
}

/// Uploads a specific batch synchronously
///
/// # Safety
/// - handle must be a valid pointer returned by geneva_client_new
/// - batches must be a valid pointer returned by geneva_encode_and_compress_logs
/// - index must be less than the value returned by geneva_batches_len
#[no_mangle]
pub unsafe extern "C" fn geneva_upload_batch_sync(
    handle: *mut GenevaClientHandle,
    batches: *const EncodedBatchesHandle,
    index: usize,
) -> GenevaError {
    // Validate client handle
    match unsafe { validate_handle(handle) } {
        GenevaError::Success => {}
        error => return error,
    }
    // validate batches
    match unsafe { validate_handle(batches) } {
        GenevaError::Success => {}
        error => return error,
    }

    // Now we know both handles are valid, safe to dereference
    let handle_ref = unsafe { handle.as_ref().unwrap() };
    let batches_ref = unsafe { batches.as_ref().unwrap() };

    if index >= batches_ref.batches.len() {
        return GenevaError::IndexOutOfRange;
    }

    let batch = batches_ref.batches[index].clone();
    let client = handle_ref.client.clone();
    let res = RUNTIME.block_on(async move { client.upload_batch(&batch).await });
    match res {
        Ok(_) => GenevaError::Success,
        Err(_e) => GenevaError::UploadFailed,
    }
}

/// Frees encoded batches handle
///
/// # Safety
/// - batches must be a valid pointer returned by geneva_encode_and_compress_logs, or null
/// - batches must not be used after calling this function
#[no_mangle]
pub unsafe extern "C" fn geneva_batches_free(batches: *mut EncodedBatchesHandle) {
    if !batches.is_null() {
        unsafe { clear_handle_magic(batches) };
        let _ = unsafe { Box::from_raw(batches) };
    }
}

// Frees a Geneva client handle
///
/// # Safety
/// - client handle must be a valid pointer returned by geneva_client_new
/// - client handle must not be used after calling this function
#[no_mangle]
pub unsafe extern "C" fn geneva_client_free(handle: *mut GenevaClientHandle) {
    if !handle.is_null() {
        unsafe { clear_handle_magic(handle) };
        let _ = unsafe { Box::from_raw(handle) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry_proto::tonic::collector::logs::v1::ExportLogsServiceRequest;
    use prost::Message;
    use std::ffi::CString;

    #[test]
    fn test_geneva_client_new_with_null_config() {
        unsafe {
            let mut out: *mut GenevaClientHandle = std::ptr::null_mut();
            let rc = geneva_client_new(std::ptr::null(), &mut out);
            assert_eq!(rc as u32, GenevaError::NullPointer as u32);
            assert!(out.is_null());
        }
    }

    #[test]
    fn test_upload_batch_sync_with_nulls() {
        unsafe {
            let result = geneva_upload_batch_sync(ptr::null_mut(), ptr::null(), 0);
            assert_eq!(result as u32, GenevaError::NullPointer as u32);
        }
    }

    #[test]
    fn test_encode_with_nulls() {
        unsafe {
            let mut out: *mut EncodedBatchesHandle = std::ptr::null_mut();
            let rc = geneva_encode_and_compress_logs(ptr::null_mut(), ptr::null(), 0, &mut out);
            assert_eq!(rc as u32, GenevaError::NullPointer as u32);
            assert!(out.is_null());
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
                auth: GenevaAuthConfig {
                    msi: GenevaMSIAuthConfig { objid: ptr::null() },
                },
            };

            let mut out: *mut GenevaClientHandle = std::ptr::null_mut();
            let rc = geneva_client_new(&config, &mut out);
            assert_eq!(rc as u32, GenevaError::MissingEndpoint as u32);
            assert!(out.is_null());
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
                auth: GenevaAuthConfig {
                    msi: GenevaMSIAuthConfig { objid: ptr::null() },
                },
            };

            let mut out: *mut GenevaClientHandle = std::ptr::null_mut();
            let rc = geneva_client_new(&config, &mut out);
            assert_eq!(rc as u32, GenevaError::InvalidAuthMethod as u32);
            assert!(out.is_null());
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
                auth: GenevaAuthConfig {
                    cert: GenevaCertAuthConfig {
                        cert_path: ptr::null(),
                        cert_password: ptr::null(),
                    },
                },
            };

            let mut out: *mut GenevaClientHandle = std::ptr::null_mut();
            let rc = geneva_client_new(&config, &mut out);
            assert_eq!(rc as u32, GenevaError::InvalidCertConfig as u32);
            assert!(out.is_null());
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
                auth: GenevaAuthConfig {
                    cert: GenevaCertAuthConfig {
                        cert_path: cert_path.as_ptr(),
                        cert_password: ptr::null(),
                    },
                },
            };

            let mut out: *mut GenevaClientHandle = std::ptr::null_mut();
            let rc = geneva_client_new(&config, &mut out);
            assert_eq!(rc as u32, GenevaError::InvalidCertConfig as u32);
            assert!(out.is_null());
        }
    }

    #[test]
    fn test_batches_len_with_null() {
        unsafe {
            let n = geneva_batches_len(ptr::null());
            assert_eq!(n, 0, "batches_len should return 0 for null pointer");
        }
    }

    #[test]
    fn test_batches_free_with_null() {
        unsafe {
            geneva_batches_free(ptr::null_mut());
        }
    }
}
