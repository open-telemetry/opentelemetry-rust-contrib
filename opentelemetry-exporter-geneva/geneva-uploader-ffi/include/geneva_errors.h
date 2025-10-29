#ifndef GENEVA_ERRORS_H
#define GENEVA_ERRORS_H

#ifdef __cplusplus
extern "C" {
#endif

/* Error codes returned by FFI functions (blocking API only).
   NOTE: Values must remain stable for ABI compatibility. */
typedef enum {
    /* Base codes (stable) */
    GENEVA_SUCCESS = 0,
    GENEVA_INVALID_CONFIG = 1,
    GENEVA_INITIALIZATION_FAILED = 2,
    GENEVA_UPLOAD_FAILED = 3,
    GENEVA_INVALID_DATA = 4,
    GENEVA_INTERNAL_ERROR = 5,

    /* Granular argument/data errors (only those currently used) */
    GENEVA_ERR_NULL_POINTER = 100,
    GENEVA_ERR_EMPTY_INPUT = 101,
    GENEVA_ERR_DECODE_FAILED = 102,
    GENEVA_ERR_INDEX_OUT_OF_RANGE = 103,
    GENEVA_ERR_INVALID_HANDLE = 104,

    /* Granular config/auth errors (only those currently used) */
    GENEVA_ERR_INVALID_AUTH_METHOD = 110,
    GENEVA_ERR_INVALID_CERT_CONFIG = 111,
    GENEVA_ERR_INVALID_WORKLOAD_IDENTITY_CONFIG = 112,
    GENEVA_ERR_INVALID_USER_MSI_CONFIG = 113,
    GENEVA_ERR_INVALID_USER_MSI_BY_OBJECT_ID_CONFIG = 114,
    GENEVA_ERR_INVALID_USER_MSI_BY_RESOURCE_ID_CONFIG = 115,

    /* Missing required config fields (granular INVALID_CONFIG) */
    GENEVA_ERR_MISSING_ENDPOINT = 130,
    GENEVA_ERR_MISSING_ENVIRONMENT = 131,
    GENEVA_ERR_MISSING_ACCOUNT = 132,
    GENEVA_ERR_MISSING_NAMESPACE = 133,
    GENEVA_ERR_MISSING_REGION = 134,
    GENEVA_ERR_MISSING_TENANT = 135,
    GENEVA_ERR_MISSING_ROLE_NAME = 136,
    GENEVA_ERR_MISSING_ROLE_INSTANCE = 137
} GenevaError;

#ifdef __cplusplus
}
#endif

#endif /* GENEVA_ERRORS_H */
