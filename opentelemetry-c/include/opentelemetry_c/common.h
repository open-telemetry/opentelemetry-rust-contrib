/*
 * opentelemetry_c/common.h
 *
 * Common types shared across the OpenTelemetry C API: status codes, booleans,
 * string views, and typed key/value attributes, plus version and error queries.
 *
 * This header is part of the `opentelemetry-c` crate, a Rust-backed C binding for
 * OpenTelemetry. See README.md for status, ABI, and ownership rules.
 *
 * Thread-safety (summary; see sdk.h and trace.h for the full per-handle contract):
 *   - SDK, tracer-provider, and tracer handles may be used concurrently from multiple
 *     threads (every operation other than *_destroy takes a shared view internally).
 *   - A single span handle must NOT be used concurrently from multiple threads; use one
 *     span per thread or synchronize externally. Distinct spans are independent.
 *   - A builder handle is NOT thread-safe; confine it to a single thread.
 *   - No *_destroy may race with any other call on the same handle.
 *   - There are no callbacks: the library never calls back into C code.
 * Version and error queries are thread-safe; the last-error message is thread-local.
 *
 * Strings are passed as length-delimited UTF-8 views (`otel_string_view_t`) and are
 * copied by the library before it returns; the caller retains ownership of the
 * underlying bytes.
 */
#ifndef OPENTELEMETRY_C_COMMON_H
#define OPENTELEMETRY_C_COMMON_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/*
 * Status code returned by fallible functions. OTEL_STATUS_OK (0) means success.
 * Any non-zero value indicates failure; call otel_last_error_message() for detail.
 *
 * New codes may be appended in future minor releases. Treat any unrecognized
 * non-zero value as a generic failure.
 */
typedef enum otel_status_t {
    OTEL_STATUS_OK = 0,               /* Success. */
    OTEL_STATUS_INVALID_ARGUMENT = 1, /* NULL/invalid pointer or handle. */
    OTEL_STATUS_INVALID_UTF8 = 2,     /* A UTF-8 string argument was malformed. */
    OTEL_STATUS_INVALID_CONFIG = 3,   /* SDK builder configuration was invalid. */
    OTEL_STATUS_ALREADY_SHUTDOWN = 4, /* The SDK/provider was already shut down. */
    OTEL_STATUS_TIMEOUT = 5,          /* Operation did not finish within the timeout. */
    OTEL_STATUS_EXPORT_FAILED = 6,    /* A runtime export failed (non-fatal). */
    OTEL_STATUS_INTERNAL_ERROR = 7    /* Unexpected internal error (incl. caught panic). */
} otel_status_t;

/*
 * Boolean type. Crosses the ABI as a fixed-width uint32_t (not a C enum) so that any bit
 * pattern a caller passes is a well-defined value on the Rust side: 0 = false, any
 * non-zero value = true. Use the OTEL_FALSE / OTEL_TRUE constants below.
 */
typedef uint32_t otel_bool_t;
enum {
    OTEL_FALSE = 0,
    OTEL_TRUE = 1
};

/*
 * A borrowed, length-delimited UTF-8 string.
 *
 * The bytes need NOT be NUL-terminated. `ptr` may be NULL only when `len == 0`
 * (representing an empty/absent string). The referenced bytes must remain valid for
 * the duration of the call they are passed to; the library copies whatever it needs
 * to retain before returning.
 *
 * Construct one from a C string with otel_cstr() (see below) or by hand.
 */
typedef struct otel_string_view_t {
    const char* ptr; /* First UTF-8 byte, or NULL when len == 0. */
    size_t len;      /* Number of bytes. */
} otel_string_view_t;

/*
 * Discriminant selecting the active member of otel_attribute_value_t.
 *
 * Crosses the ABI as a fixed-width uint32_t (not a C enum): the Rust side validates it
 * before touching the union, so an out-of-range value is rejected (with
 * OTEL_STATUS_INVALID_ARGUMENT) rather than causing a type-confused read. Use the
 * OTEL_ATTRIBUTE_TYPE_* constants below.
 */
typedef uint32_t otel_attribute_type_t;
enum {
    OTEL_ATTRIBUTE_TYPE_STRING = 0,
    OTEL_ATTRIBUTE_TYPE_BOOL = 1,
    OTEL_ATTRIBUTE_TYPE_INT64 = 2,
    OTEL_ATTRIBUTE_TYPE_DOUBLE = 3
};

/* Tagged-union payload for an attribute value. Set the member matching the tag. */
typedef union otel_attribute_value_t {
    otel_string_view_t string_value; /* OTEL_ATTRIBUTE_TYPE_STRING */
    otel_bool_t bool_value;          /* OTEL_ATTRIBUTE_TYPE_BOOL   */
    int64_t int64_value;             /* OTEL_ATTRIBUTE_TYPE_INT64  */
    double double_value;             /* OTEL_ATTRIBUTE_TYPE_DOUBLE */
} otel_attribute_value_t;

/* A single typed attribute: a non-empty key plus a tagged value. */
typedef struct otel_key_value_t {
    otel_string_view_t key;          /* UTF-8 key; must not be empty. */
    otel_attribute_type_t value_type;/* Selects the active member of `value`.
                                        Values outside the OTEL_ATTRIBUTE_TYPE_* range
                                        are rejected with OTEL_STATUS_INVALID_ARGUMENT. */
    otel_attribute_value_t value;    /* The value payload. */
} otel_key_value_t;

/*
 * ABI layout guards (64-bit, C11+). These match compile-time assertions on the Rust
 * side; a failure means the header and library disagree about struct layout.
 */
#if defined(__STDC_VERSION__) && (__STDC_VERSION__ >= 201112L) && \
    defined(UINTPTR_MAX) && (UINTPTR_MAX == 0xFFFFFFFFFFFFFFFFu)
_Static_assert(sizeof(otel_string_view_t) == 16, "otel_string_view_t ABI mismatch");
_Static_assert(sizeof(otel_attribute_value_t) == 16, "otel_attribute_value_t ABI mismatch");
_Static_assert(sizeof(otel_key_value_t) == 40, "otel_key_value_t ABI mismatch");
#endif

/* ---- Version -------------------------------------------------------------- */

/* Major/minor/patch components of the library version. */
uint32_t otel_version_major(void);
uint32_t otel_version_minor(void);
uint32_t otel_version_patch(void);

/*
 * Full semantic version string (e.g. "0.1.0"). The returned view points at static
 * storage valid for the lifetime of the process; do not free it.
 */
otel_string_view_t otel_version_string(void);

/* ---- Errors --------------------------------------------------------------- */

/*
 * Retrieve the calling thread's last error message.
 *
 * Valid until the next OpenTelemetry C call on the same thread. If no error has been
 * recorded the returned view has a NULL `ptr` and zero `len`. The pointer is
 * NUL-terminated (so it may also be used as a C string), but `len` excludes the NUL.
 */
otel_string_view_t otel_last_error_message(void);

/* ---- Helpers -------------------------------------------------------------- */

#if defined(__cplusplus) || (defined(__STDC_VERSION__) && __STDC_VERSION__ >= 199901L)
#include <string.h>
/* Build a string view from a NUL-terminated C string. `s` may be NULL (=> empty). */
static inline otel_string_view_t otel_cstr(const char* s) {
    otel_string_view_t view;
    view.ptr = s;
    view.len = (s != NULL) ? strlen(s) : (size_t)0;
    return view;
}

/* An empty (absent) string view. */
static inline otel_string_view_t otel_string_view_empty(void) {
    otel_string_view_t view;
    view.ptr = NULL;
    view.len = 0;
    return view;
}
#endif /* inline helpers */

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* OPENTELEMETRY_C_COMMON_H */
