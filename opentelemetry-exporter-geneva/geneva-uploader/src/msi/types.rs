//! Type definitions for MSI authentication

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::ptr;

#[cfg(feature = "msi_auth")]
use crate::msi::error::{MsiError, MsiResult};

/// Managed Identity configuration
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManagedIdentity {
    /// Use Object ID to identify the managed identity
    ObjectId(String),
    /// Use Client ID to identify the managed identity
    ClientId(String),
    /// Use Resource ID to identify the managed identity
    ResourceId(String),
}

impl ManagedIdentity {
    /// Get the identifier type string for the C++ API
    pub fn identifier_type(&self) -> &'static str {
        match self {
            ManagedIdentity::ObjectId(_) => "object_id",
            ManagedIdentity::ClientId(_) => "client_id",
            ManagedIdentity::ResourceId(_) => "mi_res_id",
        }
    }

    /// Get the identifier value
    pub fn identifier_value(&self) -> &str {
        match self {
            ManagedIdentity::ObjectId(value) => value,
            ManagedIdentity::ClientId(value) => value,
            ManagedIdentity::ResourceId(value) => value,
        }
    }
}

/// Configuration for MSI token requests
#[cfg(feature = "msi_auth")]
#[derive(Debug, Clone)]
pub struct MsiConfig {
    /// The resource for which to request a token (e.g., "https://monitor.core.windows.net/")
    pub resource: String,
    /// Optional managed identity configuration
    pub managed_identity: Option<ManagedIdentity>,
    /// Whether to fallback to default identity if the specified identity fails
    pub fallback_to_default: bool,
    /// Whether this is an AntMds request
    pub is_ant_mds: bool,
}

#[cfg(feature = "msi_auth")]
impl MsiConfig {
    /// Create a new configuration for the specified resource
    pub fn new(resource: impl Into<String>) -> Self {
        Self {
            resource: resource.into(),
            managed_identity: None,
            fallback_to_default: false,
            is_ant_mds: false,
        }
    }

    /// Set the managed identity
    pub fn with_managed_identity(mut self, identity: ManagedIdentity) -> Self {
        self.managed_identity = Some(identity);
        self
    }

    /// Enable fallback to default identity
    pub fn with_fallback_to_default(mut self, fallback: bool) -> Self {
        self.fallback_to_default = fallback;
        self
    }

    /// Set whether this is an AntMds request
    pub fn with_ant_mds(mut self, is_ant_mds: bool) -> Self {
        self.is_ant_mds = is_ant_mds;
        self
    }

    /// Get the identifier type string, or empty string if no managed identity is set
    pub fn identifier_type(&self) -> &str {
        self.managed_identity
            .as_ref()
            .map(|id| id.identifier_type())
            .unwrap_or("")
    }

    /// Get the identifier value, or empty string if no managed identity is set
    pub fn identifier_value(&self) -> &str {
        self.managed_identity
            .as_ref()
            .map(|id| id.identifier_value())
            .unwrap_or("")
    }
}

/// Utility functions for string conversion between Rust and C
#[cfg(feature = "msi_auth")]
pub(crate) mod string_utils {
    use super::*;

    /// Convert a Rust string to a C string pointer
    /// Returns None if the string contains null bytes
    pub fn rust_string_to_c_string(s: &str) -> MsiResult<CString> {
        CString::new(s).map_err(|_| MsiError::StringConversionFailed)
    }

    /// Convert a C string pointer to a Rust String
    /// Returns an error if the pointer is null or the string is not valid UTF-8
    pub unsafe fn c_string_to_rust_string(ptr: *const c_char) -> MsiResult<String> {
        if ptr.is_null() {
            return Err(MsiError::NullPointer);
        }

        let c_str = CStr::from_ptr(ptr);
        c_str
            .to_str()
            .map(|s| s.to_string())
            .map_err(|_| MsiError::StringConversionFailed)
    }

    /// Helper to get a C string pointer from an optional Rust string
    /// Returns a null pointer if the input is None or empty
    pub fn optional_string_to_c_ptr(s: Option<&str>) -> MsiResult<(*const c_char, Option<CString>)> {
        match s {
            Some(s) if !s.is_empty() => {
                let c_string = rust_string_to_c_string(s)?;
                let ptr = c_string.as_ptr();
                Ok((ptr, Some(c_string)))
            }
            _ => Ok((ptr::null(), None)),
        }
    }

    /// Helper to get a C string pointer from a Rust string
    pub fn string_to_c_ptr(s: &str) -> MsiResult<(*const c_char, CString)> {
        let c_string = rust_string_to_c_string(s)?;
        let ptr = c_string.as_ptr();
        Ok((ptr, c_string))
    }
}

/// Token information returned by MSI requests
#[cfg(feature = "msi_auth")]
#[derive(Debug, Clone)]
pub struct TokenInfo {
    /// The access token
    pub access_token: String,
    /// Expiration time in seconds since Unix epoch
    pub expires_on: i64,
}

#[cfg(feature = "msi_auth")]
impl TokenInfo {
    /// Create a new TokenInfo
    pub fn new(access_token: String, expires_on: i64) -> Self {
        Self {
            access_token,
            expires_on,
        }
    }

    /// Check if the token is expired (with a 5-minute buffer)
    pub fn is_expired(&self) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        
        // Add 5-minute buffer to avoid using tokens that are about to expire
        self.expires_on <= (now + 300)
    }

    /// Get the number of seconds until expiration
    pub fn seconds_until_expiration(&self) -> i64 {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        
        self.expires_on - now
    }
}

#[cfg(test)]
#[cfg(feature = "msi_auth")]
mod tests {
    use super::*;

    #[test]
    fn test_managed_identity() {
        let object_id = ManagedIdentity::ObjectId("12345".to_string());
        assert_eq!(object_id.identifier_type(), "object_id");
        assert_eq!(object_id.identifier_value(), "12345");

        let client_id = ManagedIdentity::ClientId("67890".to_string());
        assert_eq!(client_id.identifier_type(), "client_id");
        assert_eq!(client_id.identifier_value(), "67890");

        let resource_id = ManagedIdentity::ResourceId("/subscriptions/...".to_string());
        assert_eq!(resource_id.identifier_type(), "mi_res_id");
        assert_eq!(resource_id.identifier_value(), "/subscriptions/...");
    }

    #[test]
    fn test_msi_config() {
        let config = MsiConfig::new("https://monitor.core.windows.net/");
        assert_eq!(config.resource, "https://monitor.core.windows.net/");
        assert!(config.managed_identity.is_none());
        assert!(!config.fallback_to_default);
        assert!(!config.is_ant_mds);

        let config = config
            .with_managed_identity(ManagedIdentity::ClientId("test".to_string()))
            .with_fallback_to_default(true)
            .with_ant_mds(true);

        assert!(config.managed_identity.is_some());
        assert_eq!(config.identifier_type(), "client_id");
        assert_eq!(config.identifier_value(), "test");
        assert!(config.fallback_to_default);
        assert!(config.is_ant_mds);
    }

    #[test]
    fn test_token_info() {
        let future_time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64 + 3600; // 1 hour from now

        let token = TokenInfo::new("test_token".to_string(), future_time);
        assert!(!token.is_expired());
        assert!(token.seconds_until_expiration() > 0);

        let past_time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64 - 3600; // 1 hour ago

        let expired_token = TokenInfo::new("expired_token".to_string(), past_time);
        assert!(expired_token.is_expired());
        assert!(expired_token.seconds_until_expiration() < 0);
    }

    #[test]
    fn test_string_utils() {
        use string_utils::*;

        // Test valid string conversion
        let c_string = rust_string_to_c_string("test").unwrap();
        assert_eq!(c_string.to_str().unwrap(), "test");

        // Test string with null byte (should fail)
        assert!(rust_string_to_c_string("test\0test").is_err());

        // Test optional string conversion
        let (ptr, _cstring) = optional_string_to_c_ptr(Some("test")).unwrap();
        assert!(!ptr.is_null());

        let (ptr, _cstring) = optional_string_to_c_ptr(None).unwrap();
        assert!(ptr.is_null());

        let (ptr, _cstring) = optional_string_to_c_ptr(Some("")).unwrap();
        assert!(ptr.is_null());
    }
}
