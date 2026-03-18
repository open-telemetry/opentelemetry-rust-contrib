//! C-compatible FFI bindings for geneva-uploader

// Allow #[repr(C)] and other FFI attributes without wrapping in unsafe blocks (standard FFI practice)
#![allow(unsafe_attr_outside_unsafe)]

use std::ffi::CStr;
use std::marker::PhantomData;
use std::os::raw::{c_char, c_uint};
use std::ptr;
use std::sync::OnceLock;
use tokio::runtime::Runtime;

use geneva_uploader::client::{EncodedBatch, GenevaClient, GenevaClientConfig};
use geneva_uploader::AuthMethod;
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
static RUNTIME: OnceLock<Runtime> = OnceLock::new(); // TODO - Consider using LazyLock once msrv is 1.80.

fn runtime() -> &'static Runtime {
    RUNTIME.get_or_init(|| {
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
    })
}

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

    let handle_ref = unsafe { handle.as_ref().unwrap() };

    if handle_ref.magic() != GENEVA_HANDLE_MAGIC {
        return GenevaError::InvalidHandle;
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
    client: GenevaClient,
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

/// Configuration for Workload Identity auth (valid only when auth_method == 2)
#[repr(C)]
#[derive(Copy, Clone)]
pub struct GenevaWorkloadIdentityAuthConfig {
    pub resource: *const c_char, // Azure AD resource URI (e.g., "https://monitor.azure.com")
}

/// Configuration for User-assigned Managed Identity by client ID (valid only when auth_method == 3)
#[repr(C)]
#[derive(Copy, Clone)]
pub struct GenevaUserManagedIdentityAuthConfig {
    pub client_id: *const c_char, // Azure AD client ID
}

/// Configuration for User-assigned Managed Identity by object ID (valid only when auth_method == 4)
#[repr(C)]
#[derive(Copy, Clone)]
pub struct GenevaUserManagedIdentityByObjectIdAuthConfig {
    pub object_id: *const c_char, // Azure AD object ID
}

/// Configuration for User-assigned Managed Identity by resource ID (valid only when auth_method == 5)
#[repr(C)]
#[derive(Copy, Clone)]
pub struct GenevaUserManagedIdentityByResourceIdAuthConfig {
    pub resource_id: *const c_char, // Azure resource ID
}

#[repr(C)]
pub union GenevaAuthConfig {
    pub cert: GenevaCertAuthConfig, // Valid when auth_method == 1
    pub workload_identity: GenevaWorkloadIdentityAuthConfig, // Valid when auth_method == 2
    pub user_msi: GenevaUserManagedIdentityAuthConfig, // Valid when auth_method == 3
    pub user_msi_objid: GenevaUserManagedIdentityByObjectIdAuthConfig, // Valid when auth_method == 4
    pub user_msi_resid: GenevaUserManagedIdentityByResourceIdAuthConfig, // Valid when auth_method == 5
}

/// Configuration structure for Geneva client (C-compatible, tagged union)
///
/// # Auth Methods
/// - 0 = SystemManagedIdentity (auto-detected VM/AKS system-assigned identity)
/// - 1 = Certificate (mTLS with PKCS#12 certificate)
/// - 2 = WorkloadIdentity (explicit Azure Workload Identity for AKS)
/// - 3 = UserManagedIdentity (by client ID)
/// - 4 = UserManagedIdentityByObjectId (by object ID)
/// - 5 = UserManagedIdentityByResourceId (by resource ID)
///
/// # Resource Configuration
/// Different auth methods require different resource configuration:
/// - **Auth methods 0, 3, 4, 5 (MSI variants)**: Use the `msi_resource` field to specify the Azure AD resource URI
/// - **Auth method 2 (WorkloadIdentity)**: Use `auth.workload_identity.resource` field
/// - **Auth method 1 (Certificate)**: No resource needed
///
/// The `msi_resource` field specifies the Azure AD resource URI for token acquisition
/// (e.g., <https://monitor.azure.com>). For user-assigned identities (3, 4, 5), the
/// auth union specifies WHICH identity to use, while `msi_resource` specifies WHAT
/// Azure resource to request tokens FOR. These are orthogonal concerns.
#[repr(C)]
pub struct GenevaConfig {
    pub endpoint: *const c_char,
    pub environment: *const c_char,
    pub account: *const c_char,
    pub namespace_name: *const c_char,
    pub region: *const c_char,
    pub config_major_version: c_uint,
    pub auth_method: c_uint,
    pub tenant: *const c_char,
    pub role_name: *const c_char,
    pub role_instance: *const c_char,
    pub auth: GenevaAuthConfig, // Active member selected by auth_method
    pub msi_resource: *const c_char, // Azure AD resource URI for MSI auth (auth methods 0, 3, 4, 5). Not used for auth methods 1, 2. Nullable.
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
    InvalidHandle = 104,

    // Granular config/auth errors (used)
    InvalidAuthMethod = 110,
    InvalidCertConfig = 111,
    InvalidWorkloadIdentityConfig = 112,
    InvalidUserMsiConfig = 113,
    InvalidUserMsiByObjectIdConfig = 114,
    InvalidUserMsiByResourceIdConfig = 115,

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

/// Writes error message to caller-provided buffer if available
///
/// This function has zero allocation cost when err_msg_out is NULL or err_msg_len is 0.
/// Only allocates (via Display::to_string) when caller requests error details.
unsafe fn write_error_if_provided(
    err_msg_out: *mut c_char,
    err_msg_len: usize,
    error: &impl std::fmt::Display,
) {
    if !err_msg_out.is_null() && err_msg_len > 0 {
        let error_string = error.to_string();
        let bytes_to_copy = error_string.len().min(err_msg_len - 1);
        if bytes_to_copy > 0 {
            unsafe {
                std::ptr::copy_nonoverlapping(
                    error_string.as_ptr() as *const c_char,
                    err_msg_out,
                    bytes_to_copy,
                );
            }
        }
        // Always null-terminate if we have space
        unsafe {
            *err_msg_out.add(bytes_to_copy) = 0;
        }
    }
}

/// Creates a new Geneva client with explicit result semantics (no TLS needed).
///
/// On success: returns GenevaError::Success and writes a non-null handle into *out_handle.
/// On failure: returns an error code and writes a diagnostic message into err_msg_out if provided.
///
/// # Safety
/// - config must be a valid pointer to a GenevaConfig struct
/// - out_handle must be a valid pointer to receive the client handle
/// - err_msg_out: optional buffer to receive error message (can be NULL)
/// - err_msg_len: size of err_msg_out buffer
/// - caller must eventually call geneva_client_free on the returned handle
#[no_mangle]
pub unsafe extern "C" fn geneva_client_new(
    config: *const GenevaConfig,
    out_handle: *mut *mut GenevaClientHandle,
    err_msg_out: *mut c_char,
    err_msg_len: usize,
) -> GenevaError {
    // Validate pointers
    if config.is_null() || out_handle.is_null() {
        return GenevaError::NullPointer;
    }
    unsafe { *out_handle = ptr::null_mut() };

    let config = unsafe { config.as_ref().unwrap() };

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
        Err(e) => {
            unsafe { write_error_if_provided(err_msg_out, err_msg_len, &e) };
            return GenevaError::InvalidConfig;
        }
    };
    let environment = match unsafe { c_str_to_string(config.environment, "environment") } {
        Ok(s) => s,
        Err(e) => {
            unsafe { write_error_if_provided(err_msg_out, err_msg_len, &e) };
            return GenevaError::InvalidConfig;
        }
    };
    let account = match unsafe { c_str_to_string(config.account, "account") } {
        Ok(s) => s,
        Err(e) => {
            unsafe { write_error_if_provided(err_msg_out, err_msg_len, &e) };
            return GenevaError::InvalidConfig;
        }
    };
    let namespace = match unsafe { c_str_to_string(config.namespace_name, "namespace_name") } {
        Ok(s) => s,
        Err(e) => {
            unsafe { write_error_if_provided(err_msg_out, err_msg_len, &e) };
            return GenevaError::InvalidConfig;
        }
    };
    let region = match unsafe { c_str_to_string(config.region, "region") } {
        Ok(s) => s,
        Err(e) => {
            unsafe { write_error_if_provided(err_msg_out, err_msg_len, &e) };
            return GenevaError::InvalidConfig;
        }
    };
    let tenant = match unsafe { c_str_to_string(config.tenant, "tenant") } {
        Ok(s) => s,
        Err(e) => {
            unsafe { write_error_if_provided(err_msg_out, err_msg_len, &e) };
            return GenevaError::InvalidConfig;
        }
    };
    let role_name = match unsafe { c_str_to_string(config.role_name, "role_name") } {
        Ok(s) => s,
        Err(e) => {
            unsafe { write_error_if_provided(err_msg_out, err_msg_len, &e) };
            return GenevaError::InvalidConfig;
        }
    };
    let role_instance = match unsafe { c_str_to_string(config.role_instance, "role_instance") } {
        Ok(s) => s,
        Err(e) => {
            unsafe { write_error_if_provided(err_msg_out, err_msg_len, &e) };
            return GenevaError::InvalidConfig;
        }
    };

    // Auth method conversion
    let auth_method = match config.auth_method {
        0 => {
            // System-assigned Managed Identity
            AuthMethod::SystemManagedIdentity
        }

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
                Err(e) => {
                    unsafe { write_error_if_provided(err_msg_out, err_msg_len, &e) };
                    return GenevaError::InvalidConfig;
                }
            };
            let cert_password =
                match unsafe { c_str_to_string(cert.cert_password, "cert_password") } {
                    Ok(s) => s,
                    Err(e) => {
                        unsafe { write_error_if_provided(err_msg_out, err_msg_len, &e) };
                        return GenevaError::InvalidConfig;
                    }
                };
            AuthMethod::Certificate {
                path: cert_path,
                password: cert_password,
            }
        }

        2 => {
            // Workload Identity authentication
            let workload_identity = unsafe { config.auth.workload_identity };
            if workload_identity.resource.is_null() {
                return GenevaError::InvalidWorkloadIdentityConfig;
            }
            let resource = match unsafe { c_str_to_string(workload_identity.resource, "resource") }
            {
                Ok(s) => s,
                Err(e) => {
                    unsafe { write_error_if_provided(err_msg_out, err_msg_len, &e) };
                    return GenevaError::InvalidConfig;
                }
            };
            AuthMethod::WorkloadIdentity { resource }
        }

        3 => {
            // User-assigned Managed Identity by client ID
            let user_msi = unsafe { config.auth.user_msi };
            if user_msi.client_id.is_null() {
                return GenevaError::InvalidUserMsiConfig;
            }
            let client_id = match unsafe { c_str_to_string(user_msi.client_id, "client_id") } {
                Ok(s) => s,
                Err(e) => {
                    unsafe { write_error_if_provided(err_msg_out, err_msg_len, &e) };
                    return GenevaError::InvalidConfig;
                }
            };
            AuthMethod::UserManagedIdentity { client_id }
        }

        4 => {
            // User-assigned Managed Identity by object ID
            let user_msi_objid = unsafe { config.auth.user_msi_objid };
            if user_msi_objid.object_id.is_null() {
                return GenevaError::InvalidUserMsiByObjectIdConfig;
            }
            let object_id = match unsafe { c_str_to_string(user_msi_objid.object_id, "object_id") }
            {
                Ok(s) => s,
                Err(e) => {
                    unsafe { write_error_if_provided(err_msg_out, err_msg_len, &e) };
                    return GenevaError::InvalidConfig;
                }
            };
            AuthMethod::UserManagedIdentityByObjectId { object_id }
        }

        5 => {
            // User-assigned Managed Identity by resource ID
            let user_msi_resid = unsafe { config.auth.user_msi_resid };
            if user_msi_resid.resource_id.is_null() {
                return GenevaError::InvalidUserMsiByResourceIdConfig;
            }
            let resource_id =
                match unsafe { c_str_to_string(user_msi_resid.resource_id, "resource_id") } {
                    Ok(s) => s,
                    Err(e) => {
                        unsafe { write_error_if_provided(err_msg_out, err_msg_len, &e) };
                        return GenevaError::InvalidConfig;
                    }
                };
            AuthMethod::UserManagedIdentityByResourceId { resource_id }
        }

        _ => {
            return GenevaError::InvalidAuthMethod;
        }
    };

    // Parse optional MSI resource
    let msi_resource = if !config.msi_resource.is_null() {
        match unsafe { c_str_to_string(config.msi_resource, "msi_resource") } {
            Ok(s) => Some(s),
            Err(e) => {
                unsafe { write_error_if_provided(err_msg_out, err_msg_len, &e) };
                return GenevaError::InvalidConfig;
            }
        }
    } else {
        None
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
        msi_resource,
    };

    // Create client
    let client = match GenevaClient::new(geneva_config) {
        Ok(client) => client,
        Err(e) => {
            unsafe { write_error_if_provided(err_msg_out, err_msg_len, &e) };
            return GenevaError::InitializationFailed;
        }
    };

    let handle = GenevaClientHandle {
        magic: GENEVA_HANDLE_MAGIC,
        client,
    };
    unsafe { *out_handle = Box::into_raw(Box::new(handle)) };
    GenevaError::Success
}

/// Returns the number of batches in the encoded batches handle
///
/// # Safety
/// - batches must be a valid pointer returned by geneva_encode_and_compress_log_records, or null
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
/// - batches must be a valid pointer returned by geneva_encode_and_compress_log_records
/// - index must be less than the value returned by geneva_batches_len
/// - err_msg_out: optional buffer to receive error message (can be NULL)
/// - err_msg_len: size of err_msg_out buffer
#[no_mangle]
pub unsafe extern "C" fn geneva_upload_batch_sync(
    handle: *mut GenevaClientHandle,
    batches: *const EncodedBatchesHandle,
    index: usize,
    err_msg_out: *mut c_char,
    err_msg_len: usize,
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

    let batch = &batches_ref.batches[index];
    let client = &handle_ref.client;
    let res = runtime().block_on(async move { client.upload_batch(batch).await });
    match res {
        Ok(_) => GenevaError::Success,
        Err(e) => {
            unsafe { write_error_if_provided(err_msg_out, err_msg_len, &e) };
            GenevaError::UploadFailed
        }
    }
}

/// Frees encoded batches handle
///
/// # Safety
/// - batches must be a valid pointer returned by geneva_encode_and_compress_log_records, or null
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

// ============================================================
// Zero-copy log record FFI (no intermediate OTLP conversion)
// ============================================================

// Layout assertions — catch accidental ABI breakage if either the Rust struct
// or the C header is modified independently.  Values are 64-bit specific; the
// assertions are guarded so they only fire on 64-bit targets.
#[cfg(target_pointer_width = "64")]
const _: () = {
    use std::mem::{offset_of, size_of};
    assert!(size_of::<GenevaAttrValueC>() == 16);
    assert!(size_of::<GenevaLogRecordC>() == 112);
    assert!(offset_of!(GenevaLogRecordC, attr_count) == 104);
};

use otap_df_pdata_views::views::{
    common::{AnyValueView, AttributeView, InstrumentationScopeView, ValueType},
    logs::{LogRecordView, LogsDataView, ResourceLogsView, ScopeLogsView},
    resource::ResourceView,
};

/// Attribute value type tag for [`GenevaAttrValueC`].
///
/// Maps to Geneva Bond scalar types. Use the constants below to populate the
/// `tag` field before setting the active union member in `data`.
#[repr(u8)]
#[derive(Copy, Clone, PartialEq, Eq)]
pub enum GenevaAttrType {
    String = 0,
    Int64 = 1,
    Double = 2,
    Bool = 3,
}

/// Attribute value data union.
///
/// The active member is selected by [`GenevaAttrValueC::tag`].
/// Initialise only the member that corresponds to the chosen tag;
/// the other members are ignored.
#[repr(C)]
#[derive(Copy, Clone)]
pub union GenevaAttrData {
    /// Valid when `tag == GenevaAttrType::String`. Null-terminated UTF-8 string.
    pub str_val: *const c_char,
    /// Valid when `tag == GenevaAttrType::Int64`.
    pub int64_val: i64,
    /// Valid when `tag == GenevaAttrType::Double`.
    pub double_val: f64,
    /// Valid when `tag == GenevaAttrType::Bool`. 0 = false, anything else = true.
    pub bool_val: u8,
}

/// Tagged attribute value for [`GenevaLogRecordC`].
///
/// Set `tag` to the desired [`GenevaAttrType`] and populate the matching
/// field inside `data`.
#[repr(C)]
#[derive(Copy, Clone)]
pub struct GenevaAttrValueC {
    /// Discriminant — one of the [`GenevaAttrType`] constants.
    pub tag: u8,
    pub data: GenevaAttrData,
}

/// A single log record in C-compatible layout for zero-copy ingestion.
///
/// # Memory ownership
///
/// **Rust never takes ownership of any C memory.**  All pointer fields are
/// borrowed for the duration of the call to
/// [`geneva_encode_and_compress_log_records`] only.  After the call returns the
/// caller may free or reuse every buffer immediately.
///
/// # What is zero-copy
///
/// Every field accessed through a pointer (`event_name`, `body`, `severity_text`,
/// attribute keys and string values) is read directly from C memory — no
/// intermediate heap copy is made of the *input* data.  Fixed-size fields
/// (`trace_id`, `span_id`, `flags`, numeric timestamps/severity) are copied by
/// value as part of normal struct access (a few bytes each, on the stack).
///
/// # What does allocate
///
/// The *output* (Bond-encoded + LZ4-compressed bytes) necessarily allocates:
/// - One `Vec<u8>` per record for the Bond row encoding.
/// - One `String` per unique `event_name` value (amortised via `Arc`).
/// - One `Vec<u8>` per distinct event-name batch for the LZ4 output.
///
/// These allocations are owned by the returned [`EncodedBatchesHandle`] and
/// freed when that handle is passed to [`geneva_batches_free`].
///
/// # String fields
/// String fields use null-terminated C strings (`*const c_char`).
/// Pass `NULL` for any optional field that is absent.
///
/// # Attribute arrays
/// `attr_keys` and `attr_values` are parallel arrays of length `attr_count`.
/// Pass `NULL` for both (and set `attr_count = 0`) when there are no attributes.
#[repr(C)]
pub struct GenevaLogRecordC {
    /// Event name (null-terminated). `NULL` or empty string → default `"Log"`.
    pub event_name: *const c_char,

    /// Primary timestamp (nanoseconds since Unix epoch). `0` = absent.
    pub time_unix_nano: u64,

    /// Observation timestamp (nanoseconds since Unix epoch). `0` = absent.
    pub observed_time_unix_nano: u64,

    /// OTLP severity number (`0` = unspecified/unknown).
    pub severity_number: i32,

    /// Severity text (null-terminated). `NULL` = absent.
    pub severity_text: *const c_char,

    /// Log body as a null-terminated UTF-8 string. `NULL` = absent.
    pub body: *const c_char,

    /// 16-byte trace ID. Only read when `trace_id_present != 0`.
    pub trace_id: [u8; 16],
    /// Non-zero if `trace_id` carries a valid trace ID.
    pub trace_id_present: u8,

    /// 8-byte span ID. Only read when `span_id_present != 0`.
    pub span_id: [u8; 8],
    /// Non-zero if `span_id` carries a valid span ID.
    pub span_id_present: u8,

    /// Trace flags. Only used when `flags_present != 0`.
    pub flags: u32,
    /// Non-zero if `flags` is meaningful.
    pub flags_present: u8,

    /// Parallel array of null-terminated attribute keys (`attr_count` elements).
    /// `NULL` when `attr_count == 0`.
    pub attr_keys: *const *const c_char,

    /// Parallel array of attribute values (`attr_count` elements, one per key).
    /// `NULL` when `attr_count == 0`.
    pub attr_values: *const GenevaAttrValueC,

    /// Number of entries in `attr_keys` / `attr_values`.
    pub attr_count: usize,
}

// ---------------------------------------------------------------------------
// Stub view types (no resource/scope metadata required from C callers)
// ---------------------------------------------------------------------------

struct NoView;

impl ResourceView for NoView {
    type Attribute<'a>
        = NoAttrView
    where
        Self: 'a;
    type AttributesIter<'a>
        = std::iter::Empty<NoAttrView>
    where
        Self: 'a;
    fn attributes(&self) -> Self::AttributesIter<'_> {
        std::iter::empty()
    }
    fn dropped_attributes_count(&self) -> u32 {
        0
    }
}

impl InstrumentationScopeView for NoView {
    type Attribute<'a>
        = NoAttrView
    where
        Self: 'a;
    type AttributeIter<'a>
        = std::iter::Empty<NoAttrView>
    where
        Self: 'a;
    fn name(&self) -> Option<&[u8]> {
        None
    }
    fn version(&self) -> Option<&[u8]> {
        None
    }
    fn attributes(&self) -> Self::AttributeIter<'_> {
        std::iter::empty()
    }
    fn dropped_attributes_count(&self) -> u32 {
        0
    }
}

struct NoAttrView;

impl AttributeView for NoAttrView {
    type Val<'v>
        = NoAnyValue
    where
        Self: 'v;
    fn key(&self) -> &[u8] {
        b""
    }
    fn value(&self) -> Option<Self::Val<'_>> {
        None
    }
}

struct NoAnyValue;

impl<'a> AnyValueView<'a> for NoAnyValue {
    type KeyValue = NoAttrView;
    type ArrayIter<'arr>
        = std::iter::Empty<Self>
    where
        Self: 'arr;
    type KeyValueIter<'kv>
        = std::iter::Empty<NoAttrView>
    where
        Self: 'kv;
    fn value_type(&self) -> ValueType {
        ValueType::String
    }
    fn as_string(&self) -> Option<&[u8]> {
        None
    }
    fn as_bool(&self) -> Option<bool> {
        None
    }
    fn as_int64(&self) -> Option<i64> {
        None
    }
    fn as_double(&self) -> Option<f64> {
        None
    }
    fn as_bytes(&self) -> Option<&[u8]> {
        None
    }
    fn as_array(&self) -> Option<Self::ArrayIter<'_>> {
        None
    }
    fn as_kvlist(&self) -> Option<Self::KeyValueIter<'_>> {
        None
    }
}

// ---------------------------------------------------------------------------
// Body AnyValueView (string-only; C callers pass body as *const c_char)
// ---------------------------------------------------------------------------

/// Borrows the body C string for the lifetime of the enclosing log record view.
struct GenevaBodyRef<'a> {
    ptr: *const c_char,
    _marker: PhantomData<&'a c_char>,
}

impl<'a> AnyValueView<'a> for GenevaBodyRef<'a> {
    type KeyValue = NoAttrView;
    type ArrayIter<'arr>
        = std::iter::Empty<Self>
    where
        Self: 'arr;
    type KeyValueIter<'kv>
        = std::iter::Empty<NoAttrView>
    where
        Self: 'kv;

    fn value_type(&self) -> ValueType {
        ValueType::String
    }

    fn as_string(&self) -> Option<&[u8]> {
        if self.ptr.is_null() {
            return None;
        }
        // Safety: ptr is non-null and C caller guarantees it is valid for 'a.
        Some(unsafe { CStr::from_ptr(self.ptr) }.to_bytes())
    }

    fn as_bool(&self) -> Option<bool> {
        None
    }
    fn as_int64(&self) -> Option<i64> {
        None
    }
    fn as_double(&self) -> Option<f64> {
        None
    }
    fn as_bytes(&self) -> Option<&[u8]> {
        None
    }
    fn as_array(&self) -> Option<Self::ArrayIter<'_>> {
        None
    }
    fn as_kvlist(&self) -> Option<Self::KeyValueIter<'_>> {
        None
    }
}

// ---------------------------------------------------------------------------
// Attribute AnyValueView (dispatches on GenevaAttrType tag)
// ---------------------------------------------------------------------------

/// Borrows one attribute value entry from the C arrays.
struct GenevaAttrAnyValue<'a> {
    val: *const GenevaAttrValueC,
    _marker: PhantomData<&'a GenevaAttrValueC>,
}

impl<'a> AnyValueView<'a> for GenevaAttrAnyValue<'a> {
    type KeyValue = NoAttrView;
    type ArrayIter<'arr>
        = std::iter::Empty<Self>
    where
        Self: 'arr;
    type KeyValueIter<'kv>
        = std::iter::Empty<NoAttrView>
    where
        Self: 'kv;

    fn value_type(&self) -> ValueType {
        // Safety: val is non-null (checked at GenevaAttrRef::value).
        let tag = unsafe { (*self.val).tag };
        match tag {
            t if t == GenevaAttrType::String as u8 => ValueType::String,
            t if t == GenevaAttrType::Int64 as u8 => ValueType::Int64,
            t if t == GenevaAttrType::Double as u8 => ValueType::Double,
            t if t == GenevaAttrType::Bool as u8 => ValueType::Bool,
            _ => ValueType::String, // fallback; as_string returns None
        }
    }

    fn as_string(&self) -> Option<&[u8]> {
        // Safety: val non-null; tag/data consistent by C caller contract.
        let entry = unsafe { &*self.val };
        if entry.tag == GenevaAttrType::String as u8 {
            let ptr = unsafe { entry.data.str_val };
            if ptr.is_null() {
                return None;
            }
            Some(unsafe { CStr::from_ptr(ptr) }.to_bytes())
        } else {
            None
        }
    }

    fn as_int64(&self) -> Option<i64> {
        let entry = unsafe { &*self.val };
        if entry.tag == GenevaAttrType::Int64 as u8 {
            Some(unsafe { entry.data.int64_val })
        } else {
            None
        }
    }

    fn as_double(&self) -> Option<f64> {
        let entry = unsafe { &*self.val };
        if entry.tag == GenevaAttrType::Double as u8 {
            Some(unsafe { entry.data.double_val })
        } else {
            None
        }
    }

    fn as_bool(&self) -> Option<bool> {
        let entry = unsafe { &*self.val };
        if entry.tag == GenevaAttrType::Bool as u8 {
            Some(unsafe { entry.data.bool_val } != 0)
        } else {
            None
        }
    }

    fn as_bytes(&self) -> Option<&[u8]> {
        None
    }
    fn as_array(&self) -> Option<Self::ArrayIter<'_>> {
        None
    }
    fn as_kvlist(&self) -> Option<Self::KeyValueIter<'_>> {
        None
    }
}

// ---------------------------------------------------------------------------
// AttributeView for one (key, value) pair from the C arrays
// ---------------------------------------------------------------------------

struct GenevaAttrRef<'a> {
    key: *const c_char,
    val: *const GenevaAttrValueC,
    _marker: PhantomData<&'a GenevaLogRecordC>,
}

impl<'a> AttributeView for GenevaAttrRef<'a> {
    type Val<'v>
        = GenevaAttrAnyValue<'v>
    where
        Self: 'v;

    fn key(&self) -> &[u8] {
        if self.key.is_null() {
            return b"";
        }
        unsafe { CStr::from_ptr(self.key) }.to_bytes()
    }

    fn value(&self) -> Option<Self::Val<'_>> {
        if self.val.is_null() {
            return None;
        }
        Some(GenevaAttrAnyValue {
            val: self.val,
            _marker: PhantomData,
        })
    }
}

// ---------------------------------------------------------------------------
// Attribute iterator over the parallel C arrays
// ---------------------------------------------------------------------------

struct GenevaAttrIter<'a> {
    keys: *const *const c_char,
    values: *const GenevaAttrValueC,
    len: usize,
    pos: usize,
    _marker: PhantomData<&'a GenevaLogRecordC>,
}

impl<'a> Iterator for GenevaAttrIter<'a> {
    type Item = GenevaAttrRef<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        // Guard against a C caller setting attr_count > 0 with NULL array pointers.
        if self.pos >= self.len || self.keys.is_null() || self.values.is_null() {
            return None;
        }
        // Safety: pos < len, and both pointers are non-null (checked above).
        let key = unsafe { *self.keys.add(self.pos) };
        let val = unsafe { self.values.add(self.pos) };
        self.pos += 1;
        Some(GenevaAttrRef {
            key,
            val,
            _marker: PhantomData,
        })
    }
}

// ---------------------------------------------------------------------------
// LogRecordView impl for GenevaLogRecordC (via reference wrapper)
// ---------------------------------------------------------------------------

struct GenevaLogRecordRef<'a>(&'a GenevaLogRecordC);

impl<'a> LogRecordView for GenevaLogRecordRef<'a> {
    type Attribute<'att>
        = GenevaAttrRef<'att>
    where
        Self: 'att;
    type AttributeIter<'att>
        = GenevaAttrIter<'att>
    where
        Self: 'att;
    type Body<'bod>
        = GenevaBodyRef<'bod>
    where
        Self: 'bod;

    fn time_unix_nano(&self) -> Option<u64> {
        if self.0.time_unix_nano != 0 {
            Some(self.0.time_unix_nano)
        } else {
            None
        }
    }

    fn observed_time_unix_nano(&self) -> Option<u64> {
        if self.0.observed_time_unix_nano != 0 {
            Some(self.0.observed_time_unix_nano)
        } else {
            None
        }
    }

    fn severity_number(&self) -> Option<i32> {
        Some(self.0.severity_number)
    }

    fn severity_text(&self) -> Option<&[u8]> {
        if self.0.severity_text.is_null() {
            return None;
        }
        let bytes = unsafe { CStr::from_ptr(self.0.severity_text) }.to_bytes();
        if bytes.is_empty() {
            None
        } else {
            Some(bytes)
        }
    }

    fn body(&self) -> Option<Self::Body<'_>> {
        if self.0.body.is_null() {
            return None;
        }
        Some(GenevaBodyRef {
            ptr: self.0.body,
            _marker: PhantomData,
        })
    }

    fn attributes(&self) -> Self::AttributeIter<'_> {
        GenevaAttrIter {
            keys: self.0.attr_keys,
            values: self.0.attr_values,
            len: self.0.attr_count,
            pos: 0,
            _marker: PhantomData,
        }
    }

    fn dropped_attributes_count(&self) -> u32 {
        0
    }

    fn flags(&self) -> Option<u32> {
        if self.0.flags_present != 0 {
            Some(self.0.flags)
        } else {
            None
        }
    }

    fn trace_id(&self) -> Option<&[u8; 16]> {
        if self.0.trace_id_present != 0 {
            Some(&self.0.trace_id)
        } else {
            None
        }
    }

    fn span_id(&self) -> Option<&[u8; 8]> {
        if self.0.span_id_present != 0 {
            Some(&self.0.span_id)
        } else {
            None
        }
    }

    fn event_name(&self) -> Option<&[u8]> {
        if self.0.event_name.is_null() {
            return None;
        }
        let bytes = unsafe { CStr::from_ptr(self.0.event_name) }.to_bytes();
        if bytes.is_empty() {
            None
        } else {
            Some(bytes)
        }
    }
}

// ---------------------------------------------------------------------------
// View hierarchy: flat slice → single resource → single scope → records
// ---------------------------------------------------------------------------

struct FlatScopeLogs<'a>(&'a [GenevaLogRecordC]);

impl<'a> ScopeLogsView for FlatScopeLogs<'a> {
    type Scope<'s>
        = NoView
    where
        Self: 's;
    type LogRecord<'r>
        = GenevaLogRecordRef<'r>
    where
        Self: 'r;
    type LogRecordsIter<'r>
        = std::iter::Map<
        std::slice::Iter<'r, GenevaLogRecordC>,
        fn(&'r GenevaLogRecordC) -> GenevaLogRecordRef<'r>,
    >
    where
        Self: 'r;

    fn scope(&self) -> Option<Self::Scope<'_>> {
        None
    }

    fn log_records(&self) -> Self::LogRecordsIter<'_> {
        self.0.iter().map(GenevaLogRecordRef)
    }

    fn schema_url(&self) -> Option<&[u8]> {
        None
    }
}

struct FlatResourceLogs<'a>(&'a [GenevaLogRecordC]);

impl<'a> ResourceLogsView for FlatResourceLogs<'a> {
    type Resource<'r>
        = NoView
    where
        Self: 'r;
    type ScopeLogs<'s>
        = FlatScopeLogs<'s>
    where
        Self: 's;
    type ScopesIter<'s>
        = std::iter::Once<FlatScopeLogs<'s>>
    where
        Self: 's;

    fn resource(&self) -> Option<Self::Resource<'_>> {
        None
    }

    fn scopes(&self) -> Self::ScopesIter<'_> {
        std::iter::once(FlatScopeLogs(self.0))
    }

    fn schema_url(&self) -> Option<&[u8]> {
        None
    }
}

struct FlatLogsView<'a>(&'a [GenevaLogRecordC]);

impl<'a> LogsDataView for FlatLogsView<'a> {
    type ResourceLogs<'r>
        = FlatResourceLogs<'r>
    where
        Self: 'r;
    type ResourcesIter<'r>
        = std::iter::Once<FlatResourceLogs<'r>>
    where
        Self: 'r;

    fn resources(&self) -> Self::ResourcesIter<'_> {
        std::iter::once(FlatResourceLogs(self.0))
    }
}

// ---------------------------------------------------------------------------
// FFI function: encode a flat C array of log records (zero-copy, no OTLP)
// ---------------------------------------------------------------------------

/// Encode a flat C array of log records into LZ4-compressed Geneva batches.
///
/// This is the zero-copy path for C/C++ callers: records are read directly
/// from `records` without any intermediate OTLP serialisation.
///
/// # Limitations
/// Each record is treated as a standalone log entry.  Resource attributes
/// (service name, host, etc.) and instrumentation scope metadata are **not**
/// supported by this path and are silently ignored.
///
/// # Parameters
/// - `handle`: valid client handle returned by [`geneva_client_new`].
/// - `records`: pointer to an array of `record_count` [`GenevaLogRecordC`] structs.
///   All pointers inside each struct must remain valid for the duration of this call.
/// - `record_count`: number of elements in `records`.
/// - `out_batches`: receives a non-null [`EncodedBatchesHandle`] on success.
///   Must be freed with [`geneva_batches_free`] when no longer needed.
/// - `err_msg_out`: optional caller-supplied buffer for a diagnostic message.
/// - `err_msg_len`: byte capacity of `err_msg_out` (including NUL terminator).
///
/// # Return value
/// [`GenevaError::Success`] on success; an error code otherwise.
///
/// # Safety
/// - `handle` must be a valid pointer returned by `geneva_client_new`.
/// - `records` must point to at least `record_count` initialised `GenevaLogRecordC` values.
/// - Every pointer field inside each `GenevaLogRecordC` must be valid for the duration of this call.
/// - `out_batches` must be non-null.
/// - `err_msg_out` may be `NULL`; if non-null it must point to a buffer of at least `err_msg_len` bytes.
#[no_mangle]
pub unsafe extern "C" fn geneva_encode_and_compress_log_records(
    handle: *mut GenevaClientHandle,
    records: *const GenevaLogRecordC,
    record_count: usize,
    out_batches: *mut *mut EncodedBatchesHandle,
    err_msg_out: *mut c_char,
    err_msg_len: usize,
) -> GenevaError {
    if out_batches.is_null() {
        return GenevaError::NullPointer;
    }
    unsafe { *out_batches = ptr::null_mut() };

    if handle.is_null() {
        return GenevaError::NullPointer;
    }
    if records.is_null() {
        return GenevaError::NullPointer;
    }
    if record_count == 0 {
        return GenevaError::EmptyInput;
    }

    let validation_result = unsafe { validate_handle(handle) };
    if validation_result != GenevaError::Success {
        return validation_result;
    }

    let handle_ref = unsafe { handle.as_ref().unwrap() };

    // Safety: records is non-null and record_count elements are caller-guaranteed valid.
    let slice = unsafe { std::slice::from_raw_parts(records, record_count) };
    let view = FlatLogsView(slice);

    match handle_ref.client.encode_and_compress_logs(&view) {
        Ok(batches) => {
            let h = EncodedBatchesHandle {
                magic: GENEVA_HANDLE_MAGIC,
                batches,
            };
            unsafe { *out_batches = Box::into_raw(Box::new(h)) };
            GenevaError::Success
        }
        Err(e) => {
            unsafe { write_error_if_provided(err_msg_out, err_msg_len, &e) };
            GenevaError::InternalError
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    // Build a minimal unsigned JWT with the Endpoint claim and an exp. Matches what extract_endpoint_from_token expects.
    #[allow(dead_code)]
    fn generate_mock_jwt_and_expiry(endpoint: &str, ttl_secs: i64) -> (String, String) {
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
        use chrono::{Duration, Utc};

        let header = r#"{"alg":"none","typ":"JWT"}"#;
        let exp = Utc::now() + Duration::seconds(ttl_secs);
        let payload = format!(r#"{{"Endpoint":"{endpoint}","exp":{}}}"#, exp.timestamp());

        let header_b64 = URL_SAFE_NO_PAD.encode(header.as_bytes());
        let payload_b64 = URL_SAFE_NO_PAD.encode(payload.as_bytes());
        let token = format!("{}.{}.{sig}", header_b64, payload_b64, sig = "dummy");

        (token, exp.to_rfc3339())
    }

    #[test]
    fn test_geneva_client_new_with_null_config() {
        unsafe {
            let mut out: *mut GenevaClientHandle = std::ptr::null_mut();
            let rc = geneva_client_new(std::ptr::null(), &mut out, ptr::null_mut(), 0);
            assert_eq!(rc as u32, GenevaError::NullPointer as u32);
            assert!(out.is_null());
        }
    }

    #[test]
    fn test_upload_batch_sync_with_nulls() {
        unsafe {
            let result =
                geneva_upload_batch_sync(ptr::null_mut(), ptr::null(), 0, ptr::null_mut(), 0);
            assert_eq!(result as u32, GenevaError::NullPointer as u32);
        }
    }

    #[test]
    fn test_encode_log_records_with_nulls() {
        unsafe {
            // null handle
            let mut out: *mut EncodedBatchesHandle = std::ptr::null_mut();
            let rc = geneva_encode_and_compress_log_records(
                ptr::null_mut(),
                ptr::null(),
                0,
                &mut out,
                ptr::null_mut(),
                0,
            );
            assert_eq!(rc as u32, GenevaError::NullPointer as u32);
            assert!(out.is_null());

            // null records pointer
            let rc2 = geneva_encode_and_compress_log_records(
                ptr::null_mut(), // null handle → NullPointer before records check
                ptr::null(),
                1,
                &mut out,
                ptr::null_mut(),
                0,
            );
            assert_eq!(rc2 as u32, GenevaError::NullPointer as u32);

            // zero count
            let dummy_record = GenevaLogRecordC {
                event_name: ptr::null(),
                time_unix_nano: 0,
                observed_time_unix_nano: 0,
                severity_number: 0,
                severity_text: ptr::null(),
                body: ptr::null(),
                trace_id: [0u8; 16],
                trace_id_present: 0,
                span_id: [0u8; 8],
                span_id_present: 0,
                flags: 0,
                flags_present: 0,
                attr_keys: ptr::null(),
                attr_values: ptr::null(),
                attr_count: 0,
            };
            let rc3 = geneva_encode_and_compress_log_records(
                ptr::null_mut(), // null handle checked first
                &dummy_record as *const _,
                0, // zero count → EmptyInput (but null handle checked first → NullPointer)
                &mut out,
                ptr::null_mut(),
                0,
            );
            assert_eq!(rc3 as u32, GenevaError::NullPointer as u32);
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
                auth_method: 0, // SystemManagedIdentity - union not used
                tenant: tenant.as_ptr(),
                role_name: role_name.as_ptr(),
                role_instance: role_instance.as_ptr(),
                // SAFETY: GenevaAuthConfig only contains raw pointers (*const c_char).
                // Zero-initializing raw pointers creates null pointers, which is valid.
                // The union is never accessed for SystemManagedIdentity (auth_method 0).
                auth: std::mem::zeroed(),
                msi_resource: ptr::null(),
            };

            let mut out: *mut GenevaClientHandle = std::ptr::null_mut();
            let rc = geneva_client_new(&config, &mut out, ptr::null_mut(), 0);
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
                auth_method: 99, // Invalid auth method - union not used
                tenant: tenant.as_ptr(),
                role_name: role_name.as_ptr(),
                role_instance: role_instance.as_ptr(),
                auth: std::mem::zeroed(), // Union not accessed for invalid auth method
                msi_resource: ptr::null(),
            };

            let mut out: *mut GenevaClientHandle = std::ptr::null_mut();
            let rc = geneva_client_new(&config, &mut out, ptr::null_mut(), 0);
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
                msi_resource: ptr::null(),
            };

            let mut out: *mut GenevaClientHandle = std::ptr::null_mut();
            let rc = geneva_client_new(&config, &mut out, ptr::null_mut(), 0);
            assert_eq!(rc as u32, GenevaError::InvalidCertConfig as u32);
            assert!(out.is_null());
        }
    }

    #[test]
    fn test_workload_identity_auth_missing_resource() {
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
                auth_method: 2, // Workload Identity
                tenant: tenant.as_ptr(),
                role_name: role_name.as_ptr(),
                role_instance: role_instance.as_ptr(),
                auth: GenevaAuthConfig {
                    workload_identity: GenevaWorkloadIdentityAuthConfig {
                        resource: ptr::null(),
                    },
                },
                msi_resource: ptr::null(),
            };

            let mut out: *mut GenevaClientHandle = std::ptr::null_mut();
            let rc = geneva_client_new(&config, &mut out, ptr::null_mut(), 0);
            assert_eq!(rc as u32, GenevaError::InvalidWorkloadIdentityConfig as u32);
            assert!(out.is_null());
        }
    }

    #[test]
    fn test_user_msi_auth_missing_client_id() {
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
                auth_method: 3, // User Managed Identity by client ID
                tenant: tenant.as_ptr(),
                role_name: role_name.as_ptr(),
                role_instance: role_instance.as_ptr(),
                auth: GenevaAuthConfig {
                    user_msi: GenevaUserManagedIdentityAuthConfig {
                        client_id: ptr::null(),
                    },
                },
                msi_resource: ptr::null(),
            };

            let mut out: *mut GenevaClientHandle = std::ptr::null_mut();
            let rc = geneva_client_new(&config, &mut out, ptr::null_mut(), 0);
            assert_eq!(rc as u32, GenevaError::InvalidUserMsiConfig as u32);
            assert!(out.is_null());
        }
    }

    #[test]
    fn test_user_msi_auth_by_object_id_missing() {
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
                auth_method: 4, // User Managed Identity by object ID
                tenant: tenant.as_ptr(),
                role_name: role_name.as_ptr(),
                role_instance: role_instance.as_ptr(),
                auth: GenevaAuthConfig {
                    user_msi_objid: GenevaUserManagedIdentityByObjectIdAuthConfig {
                        object_id: ptr::null(),
                    },
                },
                msi_resource: ptr::null(),
            };

            let mut out: *mut GenevaClientHandle = std::ptr::null_mut();
            let rc = geneva_client_new(&config, &mut out, ptr::null_mut(), 0);
            assert_eq!(
                rc as u32,
                GenevaError::InvalidUserMsiByObjectIdConfig as u32
            );
            assert!(out.is_null());
        }
    }

    #[test]
    fn test_user_msi_auth_by_resource_id_missing() {
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
                auth_method: 5, // User Managed Identity by resource ID
                tenant: tenant.as_ptr(),
                role_name: role_name.as_ptr(),
                role_instance: role_instance.as_ptr(),
                auth: GenevaAuthConfig {
                    user_msi_resid: GenevaUserManagedIdentityByResourceIdAuthConfig {
                        resource_id: ptr::null(),
                    },
                },
                msi_resource: ptr::null(),
            };

            let mut out: *mut GenevaClientHandle = std::ptr::null_mut();
            let rc = geneva_client_new(&config, &mut out, ptr::null_mut(), 0);
            assert_eq!(
                rc as u32,
                GenevaError::InvalidUserMsiByResourceIdConfig as u32
            );
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
                msi_resource: ptr::null(),
            };

            let mut out: *mut GenevaClientHandle = std::ptr::null_mut();
            let rc = geneva_client_new(&config, &mut out, ptr::null_mut(), 0);
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
