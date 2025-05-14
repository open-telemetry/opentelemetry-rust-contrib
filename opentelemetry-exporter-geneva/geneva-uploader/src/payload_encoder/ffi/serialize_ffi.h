#pragma once
#include <cstddef>
#include <cstdint>

#ifdef __cplusplus
extern "C" {
#endif

struct SchemaResult {
    void* schema_ptr;     // Opaque pointer to SchemaDef
    void* schema_bytes;       // Marshaled bytes (for Rust)
    size_t schema_bytes_len;
};

// Marshals a schema buffer describing the fields into a schema blob and a schema pointer.
// schema_buf: pointer to binary schema description
// schema_len: length of schema_buf
// out_len: output parameter to receive size of returned buffer
// Returns: malloc-allocated pointer to SchemaResult (must be freed via free_schema_buf_ffi)
SchemaResult* marshal_schema_ffi(const void* schema_buf, size_t schema_len, size_t* out_len);

// Marshals a data row using a schema pointer and a binary row buffer.
// schema_ptr: pointer to the SchemaDef (from SchemaResult->schema_ptr)
// row_buf: pointer to binary row data (field values in schema order)
// row_len: length of row_buf
// out_len: output parameter to receive size of returned buffer
// Returns: malloc-allocated pointer to serialized row blob (must be freed via free_row_buf_ffi)
void* marshal_row_ffi(void* schema_ptr,
                           const void* row_buf, size_t row_len,
                           size_t* out_len);

// Frees a buffer allocated by the above functions.
void free_row_buf_ffi(void* ptr);

// Frees a SchemaResult structure and its contents.
void free_schema_buf_ffi(SchemaResult* result);


#ifdef __cplusplus
}
#endif