use std::os::raw::c_void;

/// An FFI struct returned by C marshalling functions, containing pointers to C-owned
/// schema data. All fields are managed on the C side and must only be freed with
/// [`free_schema_buf_ffi`].
///
/// # Safety
/// - Do **not** use after freeing.
/// - Do **not** mutate fields directly.
/// - Never free twice or from multiple threads.
#[repr(C)]
pub struct SchemaResult {
    pub schema_ptr: *mut c_void,
    pub schema_bytes: *mut c_void,
    pub schema_bytes_len: usize,
}

#[repr(transparent)]
pub struct SchemaResultPtr(pub *mut SchemaResult);

/// # Safety
/// The safety guarantee is the same as before:
/// Only one thread may mutate/free at a time, and ownership rules must be followed.
unsafe impl Send for SchemaResultPtr {}
unsafe impl Sync for SchemaResultPtr {}

unsafe extern "C" {
    pub fn marshal_schema_ffi(
        schema_buf: *const c_void,
        schema_len: usize,
        out_len: *mut usize,
    ) -> *mut SchemaResult;

    pub fn marshal_row_ffi(
        schema_ptr: *mut c_void, // NOTE: This is the schema_ptr from SchemaResult!
        row_buf: *const c_void,
        row_len: usize,
        out_len: *mut usize,
    ) -> *mut c_void;

    pub fn free_row_buf_ffi(ptr: *mut c_void);

    pub fn free_schema_buf_ffi(ptr: *mut SchemaResult);
}
