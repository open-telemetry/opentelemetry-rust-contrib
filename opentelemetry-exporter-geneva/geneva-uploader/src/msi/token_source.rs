//! High-level safe wrapper around the MSI token source functionality

#[cfg(feature = "msi_auth")]
use std::os::raw::{c_char, c_long};
#[cfg(feature = "msi_auth")]
use std::ptr;

#[cfg(feature = "msi_auth")]
use crate::msi::error::{MsiError, MsiResult};
#[cfg(feature = "msi_auth")]
use crate::msi::ffi;
#[cfg(feature = "msi_auth")]
use crate::msi::types::{string_utils, ManagedIdentity};

/// Convenience function to get an MSI access token with simple parameters
#[cfg(all(feature = "msi_auth", msi_native_available))]
pub fn get_msi_access_token(
    resource: &str,
    managed_identity: Option<&ManagedIdentity>,
    is_ant_mds: bool,
) -> MsiResult<String> {
    let id_type = managed_identity.map(|id| id.identifier_type()).unwrap_or("");
    let id_value = managed_identity.map(|id| id.identifier_value()).unwrap_or("");

    let (_resource_ptr, _resource_cstring) = string_utils::string_to_c_ptr(resource)?;
    let (_id_type_ptr, _id_type_cstring) = string_utils::optional_string_to_c_ptr(
        if id_type.is_empty() { None } else { Some(id_type) }
    )?;
    let (_id_value_ptr, _id_value_cstring) = string_utils::optional_string_to_c_ptr(
        if id_value.is_empty() { None } else { Some(id_value) }
    )?;

    let mut token_ptr: *mut c_char = ptr::null_mut();

    let result = unsafe {
        ffi::rust_get_msi_access_token(
            _resource_ptr,
            _id_type_ptr,
            _id_value_ptr,
            is_ant_mds,
            &mut token_ptr,
        )
    };

    MsiError::check_result(result)?;

    if token_ptr.is_null() {
        return Err(MsiError::NullPointer);
    }

    let token = unsafe {
        let result = string_utils::c_string_to_rust_string(token_ptr);
        ffi::rust_free_string(token_ptr);
        result
    }?;

    Ok(token)
}

/// Stub implementation when MSI feature is enabled but native library is not available
#[cfg(all(feature = "msi_auth", not(msi_native_available)))]
pub fn get_msi_access_token(
    _resource: &str,
    _managed_identity: Option<&ManagedIdentity>,
    _is_ant_mds: bool,
) -> MsiResult<String> {
    Err(MsiError::AuthenticationFailed(
        "MSI native library is not available. Set MSINATIVE_LIB_PATH and ensure dependencies are installed.".to_string()
    ))
}

/// Stub implementation when MSI authentication is not enabled
#[cfg(not(feature = "msi_auth"))]
pub fn get_msi_access_token(
    _resource: &str,
    _managed_identity: Option<&crate::config_service::client::MsiIdentityType>,
    _is_ant_mds: bool,
) -> Result<String, String> {
    Err("MSI authentication support is not enabled. Enable the 'msi_auth' feature to use MSI authentication.".into())
}

#[cfg(test)]
#[cfg(feature = "msi_auth")]
mod tests {
    use super::*;
    use crate::msi::types::ManagedIdentity;

    #[test]
    fn test_managed_identity_parameters() {
        // Test parameter generation for different identity types
        let client_id = ManagedIdentity::ClientId("test-client-id".to_string());
        assert_eq!(client_id.identifier_type(), "client_id");
        assert_eq!(client_id.identifier_value(), "test-client-id");

        let object_id = ManagedIdentity::ObjectId("test-object-id".to_string());
        assert_eq!(object_id.identifier_type(), "object_id");
        assert_eq!(object_id.identifier_value(), "test-object-id");

        let resource_id = ManagedIdentity::ResourceId("/subscriptions/test".to_string());
        assert_eq!(resource_id.identifier_type(), "mi_res_id");
        assert_eq!(resource_id.identifier_value(), "/subscriptions/test");
    }

    #[test]
    fn test_parameter_validation() {
        // Test that empty/None parameters are handled correctly
        let id_type = None::<&ManagedIdentity>.map(|id| id.identifier_type()).unwrap_or("");
        let id_value = None::<&ManagedIdentity>.map(|id| id.identifier_value()).unwrap_or("");
        
        assert_eq!(id_type, "");
        assert_eq!(id_value, "");
    }

    // Note: We can't test the actual get_msi_access_token function without the C++ library
    // being available in the test environment, but we can test the parameter handling logic
}
