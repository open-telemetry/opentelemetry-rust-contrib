//! FFI bindings for geneva-uploader to be used from Go via CGO
//!
//! This crate provides C-compatible functions that can be called from Go
//! to use the Geneva uploader functionality.

#![allow(unsafe_op_in_unsafe_fn)]
#![allow(unsafe_attr_outside_unsafe)]

use once_cell::sync::Lazy;
use std::cell::RefCell;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_uint};
use std::ptr;
use std::sync::Arc;
use tokio::runtime::Runtime;

use geneva_uploader::client::{GenevaClient, GenevaClientConfig};
use geneva_uploader::AuthMethod;
use prost::Message;
use std::path::PathBuf;
use opentelemetry_proto::tonic::collector::logs::v1::ExportLogsServiceRequest;

/// Global shared Tokio runtime for efficiency and Geneva client compatibility
/// Benefits:
/// - Geneva client is designed for concurrent operations (buffer_unordered)
/// - Thread-agnostic: FFI callers can use from any thread
/// - Optimal resource usage: single runtime shared across all clients
/// - High performance: multi-threaded runtime supports Geneva's concurrent uploads
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
    AsyncOperationPending = 6,
}

/// Callback function type for async upload completion
/// Parameters: error_code, user_data
pub type UploadCallback = unsafe extern "C" fn(GenevaError, *mut std::ffi::c_void);

/// Wrapper for Send-safe pointer passing in FFI callbacks.
///
/// # Safety
///
/// `SendPtr` wraps a raw pointer intended to be passed as user data in FFI callbacks,
/// typically to C code. The unsafe `Send` and `Sync` implementations are sound only if
/// the following invariants are upheld:
///
/// - The pointer must refer to data that is safe to send across thread boundaries.
///   This usually means the pointer is either to immutable data, or to data that is
///   itself thread-safe (e.g., protected by a mutex, or only accessed by one thread).
/// - The lifetime of the pointed-to data must outlive all uses of the pointer in any
///   thread. The pointer must not be used after the data is freed.
/// - The FFI boundary (C code) must guarantee that the pointer is not accessed
///   concurrently in a way that would violate Rust's aliasing or mutability rules.
/// - This type is only intended for use with FFI callbacks where the underlying C code
///   guarantees these invariants, such as passing opaque handles or user data pointers
///   that are managed externally.
///
/// Failure to uphold these invariants may result in undefined behavior.
#[derive(Clone, Copy)]
struct SendPtr(*mut std::ffi::c_void);

// SAFETY: See the safety comment above. The implementor must ensure that the pointer
// is only used in contexts where it is safe to send across threads, as described.
unsafe impl Send for SendPtr {}
// SAFETY: See the safety comment above. The implementor must ensure that the pointer
// is only used in contexts where it is safe to share between threads, as described.
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
///
/// # Safety
///
/// The `unsafe impl Send for SendCallback` and `unsafe impl Sync for SendCallback` are
/// considered sound under the following specific conditions, which are guaranteed by the
/// FFI design and usage patterns:
///
/// ## Thread Safety Guarantees:
///
/// 1. **C Function Pointer Invariants**: FFI callback function pointers (`extern "C" fn`)
///    are inherently thread-safe because:
///    - They point to immutable code segments in memory
///    - C functions do not capture Rust environment/closures
///    - The function pointer itself is just an address - no mutable state
///
/// 2. **Calling Convention Safety**: The callback uses C calling convention (`extern "C"`),
///    which ensures:
///    - Consistent ABI across thread boundaries
///    - No Rust-specific thread-local state dependencies
///    - Compatible with threading models of C/C++ callers
///
/// 3. **Lifetime and Ownership**:
///    - The callback function pointer must remain valid for the duration of the async operation
///    - The C code is responsible for ensuring the callback function doesn't become invalid
///    - No Rust ownership is transferred - only a function pointer is copied
///
/// 4. **No Shared Mutable State**:
///    - The callback function itself contains no shared mutable state
///    - Any mutable state access must be handled by the C implementation using appropriate
///      synchronization primitives (mutexes, atomics, etc.)
///
/// 5. **Single Invocation Guarantee**:
///    - Each callback is invoked exactly once per async operation
///    - No concurrent access to the same callback instance from multiple threads
///    - The callback is consumed when called, preventing reuse issues
///
/// ## Usage Contract:
///
/// This implementation is only safe when used in the specific context of the Geneva FFI:
/// - Callbacks are passed from C code that guarantees thread-safety of the function pointer
/// - The callback is invoked from a dedicated thread spawned specifically for this purpose
/// - The Geneva FFI ensures proper synchronization between the async runtime and callback thread
///
/// ## Violations that would cause UB:
///
/// - Passing a callback that accesses thread-local storage without proper synchronization
/// - C code invalidating the function pointer while async operation is in progress
/// - Callback function accessing shared mutable state without synchronization
/// - Using this wrapper outside the Geneva FFI context without equivalent guarantees
///
/// The implementor (Geneva FFI) guarantees these invariants are upheld through:
/// - Proper async task lifecycle management
/// - Dedicated callback thread isolation
/// - C API contract enforcement
#[derive(Clone, Copy)]
struct SendCallback(UploadCallback);

// SAFETY: See comprehensive safety documentation above. This is safe because FFI callback
// function pointers are inherently thread-safe (immutable code addresses) and the Geneva FFI
// guarantees proper usage patterns including single invocation, dedicated callback threads,
// and lifetime management.
unsafe impl Send for SendCallback {}

// SAFETY: See comprehensive safety documentation above. Sync is safe for the same reasons as
// Send - the callback is an immutable function pointer with no shared mutable state, and
// the Geneva FFI ensures proper synchronization between async operations and callback invocation.
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

            let cert_password =
                match unsafe { c_str_to_string(config.cert_password, "cert_password") } {
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
        }
        _ => {
            set_last_error(&format!(
                "Invalid auth_method: {}. Must be 0 (ManagedIdentity) or 1 (Certificate)",
                config.auth_method
            ));
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
    println!("[DEBUG] Creating Geneva client with config: endpoint={}, environment={}, account={}, namespace={}, region={}, auth_method={:?}", 
             geneva_config.endpoint, geneva_config.environment, geneva_config.account, 
             geneva_config.namespace, geneva_config.region, geneva_config.auth_method);
    
    let client = match RUNTIME.block_on(GenevaClient::new(geneva_config)) {
        Ok(client) => {
            println!("[DEBUG] Geneva client created successfully");
            Arc::new(client)
        },
        Err(e) => {
            println!("[ERROR] Failed to create Geneva client: {}", e);
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
    println!("[DEBUG] geneva_upload_logs_sync called with handle: {:?}, data_len: {}", handle, data_len);
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

    // Decode protobuf data as LogsData (produced by plog.ProtoMarshaler)
    // TODO: Memory pressure risk - protobuf data is decoded and held in memory during the entire
    // synchronous upload operation. For high-throughput OTLP Collector scenarios, this could
    // accumulate significant memory if multiple threads call this function simultaneously with
    // large log batches or if Geneva uploads are slow. Consider implementing:
    // 1. Reasonable limits on protobuf payload size
    // 2. Preferring async version for high-throughput scenarios
    // 3. Memory usage monitoring in production deployments
    let logs_data: ExportLogsServiceRequest = match Message::decode(data_slice) {
        Ok(data) => data,
        Err(e) => {
            set_last_error(&format!("Failed to decode protobuf LogsData: {}", e));
            return GenevaError::InvalidData;
        }
    };

    // Extract ResourceLogs from the LogsData
    let resource_logs = logs_data.resource_logs;

    // Debug: Print information about the decoded data
    println!("[DEBUG] Decoded {} ResourceLogs from protobuf", resource_logs.len());
    for (i, rl) in resource_logs.iter().enumerate() {
        println!("[DEBUG] ResourceLogs[{}]: {} scope_logs", i, rl.scope_logs.len());
        let total_records: usize = rl.scope_logs.iter().map(|sl| sl.log_records.len()).sum();
        println!("[DEBUG] ResourceLogs[{}]: {} total log records", i, total_records);
    }

    // Upload logs using the shared runtime (blocking)
    println!("[DEBUG] Starting Geneva upload with {} ResourceLogs", resource_logs.len());
    match RUNTIME.block_on(handle.client.upload_logs(&resource_logs)) {
        Ok(_) => {
            println!("[DEBUG] Geneva upload successful!");
            GenevaError::Success
        },
        Err(e) => {
            println!("[ERROR] Geneva upload failed: {}", e);
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
    println!("[DEBUG] geneva_upload_logs called with handle: {:?}, data_len: {}, callback: {:?}, user_data: {:?}", 
             handle, data_len, callback, user_data);
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

    // Decode protobuf data as ExportLogsServiceRequest (produced by plog.ProtoMarshaler)
    let logs_data: ExportLogsServiceRequest = match Message::decode(data_slice) {
        Ok(data) => data,
        Err(e) => {
            set_last_error(&format!("Failed to decode protobuf LogsData: {}", e));
            return GenevaError::InvalidData;
        }
    };

    // Extract ResourceLogs from the LogsData
    let resource_logs = logs_data.resource_logs;

    // Clone the client for the async task
    let client = handle.client.clone();

    // Wrap the callback and user_data pointer for safe transfer across threads
    let callback_wrapper = SendCallback::new(callback);
    let user_data_wrapper = SendPtr::new(user_data);

    // Spawn async task on the runtime
    RUNTIME.spawn(async move {
        let result = client.upload_logs(&resource_logs).await;

        // Determine the error code
        // TODO: Error information loss - detailed error context from upload_logs() is discarded.
        // The Result<(), String> contains valuable debugging information that should be preserved
        // for better observability. Consider either:
        // 1. Storing error details in thread-local storage before callback
        // 2. Extending callback signature to include error details
        // 3. Using more granular error codes for different failure types
        let error_code = match result {
            //print error string for debugging
            Ok(_) => GenevaError::Success,
            Err(e) => {
                println!("Error uploading logs to Geneva: {}", e);
                set_last_error(&format!("Failed to upload logs to Geneva: {}", e));
                GenevaError::UploadFailed
            }
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
    THREAD_LOCAL_ERROR.with(|error| match error.borrow().as_ref() {
        Some(err) => err.as_ptr(),
        None => ptr::null(),
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
    unsafe extern "C" fn dummy_callback(
        _error_code: GenevaError,
        _user_data: *mut std::ffi::c_void,
    ) {
        // Do nothing - just for testing
    }

    #[test]
    fn test_geneva_upload_logs_with_null_handle() {
        unsafe {
            let data = vec![1, 2, 3, 4];
            let result = geneva_upload_logs(
                ptr::null_mut(),
                data.as_ptr(),
                data.len(),
                dummy_callback,
                ptr::null_mut(),
            );
            assert_eq!(result as u32, GenevaError::InvalidData as u32);
        }
    }

    #[test]
    fn test_geneva_upload_logs_with_null_data() {
        unsafe {
            let result = geneva_upload_logs(
                ptr::null_mut(),
                ptr::null(),
                0,
                dummy_callback,
                ptr::null_mut(),
            );
            assert_eq!(result as u32, GenevaError::InvalidData as u32);
        }
    }

    #[test]
    fn test_geneva_upload_logs_with_zero_length() {
        unsafe {
            let data = vec![1, 2, 3, 4];
            let result = geneva_upload_logs(
                ptr::null_mut(),
                data.as_ptr(),
                0,
                dummy_callback,
                ptr::null_mut(),
            );
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
            assert!(
                !error_ptr.is_null(),
                "Should have error message for invalid config"
            );
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
            assert!(
                client.is_null(),
                "Client should be null for invalid auth method"
            );
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
            assert!(
                client.is_null(),
                "Client should be null for missing cert path"
            );
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
            assert!(
                client.is_null(),
                "Client should be null for missing cert password"
            );
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
            assert!(
                client.is_null(),
                "Client should be null for max_concurrent_uploads = 0"
            );
        }
    }

    #[test]
    fn test_callback_function_signature() {
        // Test that the callback function signature is correct by compilation
        unsafe extern "C" fn test_callback(
            _error_code: GenevaError,
            _user_data: *mut std::ffi::c_void,
        ) {
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
                ptr::null_mut(),
            );

            // Should fail immediately with invalid data (null handle)
            assert_eq!(result as u32, GenevaError::InvalidData as u32);
        }
    }
}
