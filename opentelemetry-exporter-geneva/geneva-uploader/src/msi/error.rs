//! Error types for MSI authentication

use std::fmt;
use thiserror::Error;

#[cfg(feature = "msi_auth")]
use crate::msi::ffi::XPLATRESULT;

/// Errors that can occur when working with MSI tokens
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum MsiError {
    /// MSI token source initialization failed
    #[error("Initialization failed")]
    InitializationFailed,
    
    /// General failure occurred
    #[error("General failure")]
    GeneralFailure,
    
    /// Azure MSI token request failed
    #[error("Azure MSI token request failed")]
    AzureMsiFailed,
    
    /// ARC MSI token request failed
    #[error("ARC MSI token request failed")]
    ArcMsiFailed,
    
    /// AntMds MSI token request failed
    #[error("AntMds MSI token request failed")]
    AntMdsMsiFailed,
    
    /// MSI authentication failed
    #[error("MSI authentication failed: {0}")]
    AuthenticationFailed(String),
    
    /// IMDS endpoint is not accessible
    #[error("IMDS endpoint error")]
    ImdsEndpointError,
    
    /// Invalid parameter provided
    #[error("Invalid parameter: {0}")]
    InvalidParameter(String),
    
    /// Null pointer encountered in FFI
    #[error("Null pointer encountered")]
    NullPointer,
    
    /// String conversion between Rust and C failed
    #[error("String conversion failed")]
    StringConversionFailed,
    
    /// Unknown error code from underlying library
    #[error("Unknown error code: {0}")]
    Unknown(i32),
}

#[cfg(feature = "msi_auth")]
impl MsiError {
    /// Convert an XPLATRESULT to a MsiError
    pub fn from_xplat_result(result: XPLATRESULT) -> Self {
        // Error codes based on XPlatErrors.h
        match result {
            0 => return Self::GeneralFailure, // XPLAT_NO_ERROR should not be an error
            code if code < 0 => {
                // Extract facility and error code
                let facility = ((code as u32) >> 8) & 0xFF;
                let error_code = (code as u32) & 0xFF;
                
                match facility {
                    0x1 => { // XPLAT_FACILITY_GENERAL
                        match error_code {
                            0x1 => Self::GeneralFailure,
                            0x3 => Self::InitializationFailed,
                            _ => Self::Unknown(result),
                        }
                    },
                    0x2 => { // XPLAT_FACILITY_MSI_TOKEN
                        match error_code {
                            0x1 => Self::AzureMsiFailed,
                            0x2 => Self::ArcMsiFailed,
                            0x3 => Self::AntMdsMsiFailed,
                            _ => Self::Unknown(result),
                        }
                    },
                    0x3 => { // XPLAT_FACILITY_IMDS
                        match error_code {
                            0x1 => Self::ImdsEndpointError,
                            _ => Self::Unknown(result),
                        }
                    },
                    _ => Self::Unknown(result),
                }
            },
            _ => Self::Unknown(result),
        }
    }
    
    /// Check if an XPLATRESULT indicates success
    pub fn is_success(result: XPLATRESULT) -> bool {
        result >= 0
    }
    
    /// Convert an XPLATRESULT to a Result type
    pub fn check_result(result: XPLATRESULT) -> Result<(), Self> {
        if Self::is_success(result) {
            Ok(())
        } else {
            Err(Self::from_xplat_result(result))
        }
    }
}

/// Result type for MSI operations
pub type MsiResult<T> = Result<T, MsiError>;

#[cfg(test)]
#[cfg(feature = "msi_auth")]
mod tests {
    use super::*;

    #[test]
    fn test_error_conversion() {
        // Test success
        assert!(MsiError::is_success(0));
        assert!(MsiError::check_result(0).is_ok());
        
        // Test simple negative error codes (for mock implementation)
        assert_eq!(MsiError::from_xplat_result(-1), MsiError::Unknown(-1));
        assert_eq!(MsiError::from_xplat_result(-2), MsiError::Unknown(-2));
        
        // Test that positive codes other than 0 are treated as success
        assert!(MsiError::is_success(1));
        assert!(MsiError::check_result(1).is_ok());
    }
    
    #[test]
    fn test_error_display() {
        let error = MsiError::InitializationFailed;
        assert_eq!(error.to_string(), "Initialization failed");
        
        let param_error = MsiError::InvalidParameter("resource".to_string());
        assert_eq!(param_error.to_string(), "Invalid parameter: resource");
    }
}
