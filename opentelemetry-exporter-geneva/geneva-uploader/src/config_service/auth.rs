//! Authentication, config, and shared types for Geneva Config clients.

use crate::config_service::error;
use native_tls::{Identity, Protocol};
use std::fs;
use std::path::PathBuf;

/// Authentication methods for the Geneva Config Client.
///
/// The client supports two authentication methods:
/// - Certificate-based authentication using PKCS#12 (.p12) files
/// - Managed Identity (Azure) - planned for future implementation
///
/// # Certificate Format
/// Certificates should be in PKCS#12 (.p12) format for client TLS authentication.
///
/// ## Converting from PEM to PKCS#12
///
/// If you have PEM format cert and key, you can convert them using OpenSSL:
///
/// ### Linux/macOS:
/// ```bash
/// openssl pkcs12 -export \
///   -in cert.pem \
///   -inkey key.pem \
///   -out client.p12 \
///   -name "alias"
/// ```
///
/// ### Windows (PowerShell):
/// ```powershell
/// openssl pkcs12 -export -in cert.pem -inkey key.pem -out client.p12 -name "alias"
/// ```
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub(crate) enum AuthMethod {
    /// Certificate-based authentication
    ///
    /// # Arguments
    /// * `path` - Path to the PKCS#12 (.p12) certificate file
    /// * `password` - Password to decrypt the PKCS#12 file
    Certificate { path: PathBuf, password: String },
    /// Azure Managed Identity authentication
    ///
    /// Note(TODO): This is not yet implemented.
    ManagedIdentity,
}

#[cfg(feature = "self_signed_certs")]
pub(crate) fn configure_tls_connector(
    mut builder: native_tls::TlsConnectorBuilder,
    identity: native_tls::Identity,
) -> native_tls::TlsConnectorBuilder {
    eprintln!("WARNING: Self-signed certificates will be accepted. This should only be used in development!");
    builder
        .identity(identity)
        .min_protocol_version(Some(Protocol::Tlsv12))
        .max_protocol_version(Some(Protocol::Tlsv12))
        .danger_accept_invalid_certs(true);
    builder
}

#[cfg(not(feature = "self_signed_certs"))]
pub(crate) fn configure_tls_connector(
    mut builder: native_tls::TlsConnectorBuilder,
    identity: native_tls::Identity,
) -> native_tls::TlsConnectorBuilder {
    builder
        .identity(identity)
        .min_protocol_version(Some(Protocol::Tlsv12))
        .max_protocol_version(Some(Protocol::Tlsv12));
    builder
}

/// Helper for loading PKCS#12 identity from disk.
#[allow(dead_code)]
pub(crate) fn load_identity(
    path: &PathBuf,
    password: &str,
) -> error::GenevaConfigClientResult<Identity> {
    let p12_bytes =
        fs::read(path).map_err(|e| error::GenevaConfigClientError::Certificate(e.to_string()))?;
    Identity::from_pkcs12(&p12_bytes, password)
        .map_err(|e| error::GenevaConfigClientError::Certificate(e.to_string()))
}
