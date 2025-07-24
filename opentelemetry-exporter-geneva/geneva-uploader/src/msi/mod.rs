//! Azure Managed Service Identity (MSI) authentication module
//! 
//! This module provides MSI authentication functionality integrated directly into the Geneva uploader.
//! It contains the essential components from the MSI library needed for Geneva authentication.

#[cfg(feature = "msi_auth")]
pub mod error;
#[cfg(feature = "msi_auth")]
pub mod ffi;
#[cfg(feature = "msi_auth")]
pub mod token_source;
#[cfg(feature = "msi_auth")]
pub mod types;

#[cfg(feature = "msi_auth")]
pub use error::{MsiError, MsiResult};
#[cfg(feature = "msi_auth")]
pub use token_source::get_msi_access_token;
#[cfg(feature = "msi_auth")]
pub use types::ManagedIdentity;

/// Azure Monitor service endpoints for Geneva authentication
#[cfg(feature = "msi_auth")]
pub mod resources {
    /// Azure Monitor endpoint for public Azure cloud (used for Geneva authentication)
    pub const AZURE_MONITOR_PUBLIC: &str = "https://monitor.core.windows.net/";
    // Add more endpoints as needed
}
