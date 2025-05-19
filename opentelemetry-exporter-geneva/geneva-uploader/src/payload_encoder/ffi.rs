use std::os::raw::c_void;

#[repr(C)]
pub struct SchemaResult {
    pub schema_ptr: *mut c_void,
    pub schema_bytes: *mut c_void,
    pub schema_bytes_len: usize,
}

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
