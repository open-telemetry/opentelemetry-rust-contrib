//! UDP client for sending segment documents to the AWS X-Ray daemon.
//!
//! This module provides [`XrayDaemonClient`], which transmits X-Ray segment documents
//! to the X-Ray daemon via UDP. The daemon then forwards the segments to the AWS X-Ray service.
//!
//! # Configuration
//!
//! The client can be configured in two ways:
//!
//! 1. **Explicitly** by providing a socket address to [`XrayDaemonClient::new`]
//! 2. **Via environment variable** using the `AWS_XRAY_DAEMON_ADDRESS` environment variable
//!    when using [`Default::default`]
//!
//! # Examples
//!
//! Using default configuration (localhost:2000 or AWS_XRAY_DAEMON_ADDRESS):
//!
//! ```no_run
//! use opentelemetry_aws::xray_exporter::daemon_client::XrayDaemonClient;
//!
//! // Uses localhost:2000 or AWS_XRAY_DAEMON_ADDRESS if set
//! let client = XrayDaemonClient::default();
//! ```
//!
//! Explicit configuration for custom daemon address:
//!
//! ```no_run
//! use opentelemetry_aws::xray_exporter::daemon_client::XrayDaemonClient;
//! use std::net::{IpAddr, Ipv4Addr, SocketAddr};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! // Using a remote daemon or non-default port
//! let daemon_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100)), 3000);
//! let client = XrayDaemonClient::new(daemon_addr)?;
//! # Ok(())
//! # }
//! ```

use std::{
    io,
    net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket},
    sync::Mutex,
};

use crate::xray_exporter::types::SegmentDocument;

use super::SegmentDocumentExporter;

/// UDP client for transmitting X-Ray segment documents to the X-Ray daemon.
///
/// # Protocol
///
/// Each UDP packet sent to the daemon has the following format:
/// ```text
/// {"format": "json", "version": 1}\n
/// {segment document JSON}
/// ```
///
/// # Examples
///
/// Using default configuration (reads from `AWS_XRAY_DAEMON_ADDRESS` env var or localhost:2000):
///
/// ```no_run
/// use opentelemetry_aws::xray_exporter::{XrayExporter, daemon_client::XrayDaemonClient};
///
/// let client = XrayDaemonClient::default();
/// let exporter = XrayExporter::new(client);
/// ```
///
/// Custom daemon address (non-default port or remote host):
///
/// ```no_run
/// # use opentelemetry_aws::xray_exporter::{XrayExporter, daemon_client::XrayDaemonClient};
/// use std::net::{IpAddr, Ipv4Addr, SocketAddr};
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let daemon_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100)), 3000);
/// let client = XrayDaemonClient::new(daemon_addr)?;
/// let exporter = XrayExporter::new(client);
/// # Ok(())
/// # }
/// ```
#[derive(Debug)]
pub struct XrayDaemonClient {
    socket: UdpSocket,
    inner_buf: Mutex<Vec<u8>>,
}

impl XrayDaemonClient {
    const DAEMON_HEADER: &[u8] = "{\"format\": \"json\", \"version\": 1}\n".as_bytes();
    const DEFAULT_DAEMON_PORT: u16 = 2000;
    const AWS_XRAY_DAEMON_ADDRESS_ENV_VAR: &str = "AWS_XRAY_DAEMON_ADDRESS";

    /// Creates a new X-Ray daemon client that sends to the specified address.
    ///
    /// Creates a UDP socket connects it to the specified daemon address.
    /// The socket is set to non-blocking mode.
    ///
    /// # Errors
    ///
    /// Returns an [`io::Error`] if:
    /// - The UDP socket cannot be created or bound
    /// - The socket cannot be set to non-blocking mode
    /// - The socket cannot connect to the specified address
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use opentelemetry_aws::xray_exporter::daemon_client::XrayDaemonClient;
    /// use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let daemon_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100)), 2000);
    /// let client = XrayDaemonClient::new(daemon_addr)?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// Custom port:
    ///
    /// ```no_run
    /// use opentelemetry_aws::xray_exporter::daemon_client::XrayDaemonClient;
    /// use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let daemon_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 3000);
    /// let client = XrayDaemonClient::new(daemon_addr)?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn new(addr: SocketAddr) -> Result<Self, io::Error> {
        let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
        socket.set_nonblocking(true)?;
        socket.connect(addr)?;

        let mut buf = Vec::with_capacity(65507); // Max UDP packet size
        buf.extend_from_slice(Self::DAEMON_HEADER);

        Ok(Self {
            socket,
            inner_buf: Mutex::new(buf),
        })
    }

    /// Sends a single segment document to the X-Ray daemon via UDP.
    ///
    /// Reuses an internal buffer to avoid allocations on each send.
    fn send_segment_document(
        &self,
        segment_document: SegmentDocument<'_>,
    ) -> Result<(), io::Error> {
        // Get a mut ref on the internal buffer
        let mut buf = self.inner_buf.lock().unwrap();
        // Serialize the segment into the internal buffer, after the
        segment_document.to_writer(&mut *buf);

        // Send
        self.socket.send(&buf)?;

        // Truncate back to the header size so the buffer can be reused
        buf.truncate(Self::DAEMON_HEADER.len());

        Ok(())
    }
}

impl Default for XrayDaemonClient {
    /// Creates a client using the default X-Ray daemon address.
    ///
    /// The daemon address is determined by:
    /// 1. Reading the `AWS_XRAY_DAEMON_ADDRESS` environment variable if set
    /// 2. Falling back to `127.0.0.1:2000` (localhost:2000 UDP)
    ///
    /// # Panics
    ///
    /// Panics if the UDP socket cannot be created. Use [`XrayDaemonClient::new`] for more
    /// control over error handling.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use opentelemetry_aws::xray_exporter::daemon_client::XrayDaemonClient;
    ///
    /// // Uses localhost:2000 or AWS_XRAY_DAEMON_ADDRESS if set
    /// let client = XrayDaemonClient::default();
    /// ```
    fn default() -> Self {
        Self::new(
            std::env::var(Self::AWS_XRAY_DAEMON_ADDRESS_ENV_VAR)
                .ok()
                .and_then(|s| s.parse::<SocketAddr>().ok())
                .unwrap_or_else(|| {
                    #[cfg(feature = "internal-logs")]
                    tracing::warn!(
                        "No valid {} env variable detected, falling back on default",
                        Self::AWS_XRAY_DAEMON_ADDRESS_ENV_VAR
                    );
                    SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), Self::DEFAULT_DAEMON_PORT)
                }),
        )
        .expect("could not bind daemon")
    }
}

impl SegmentDocumentExporter for XrayDaemonClient {
    type Error = io::Error;

    async fn export_segment_documents(
        &self,
        batch: Vec<SegmentDocument<'_>>,
    ) -> Result<(), Self::Error> {
        for segment_document in batch {
            self.send_segment_document(segment_document)?;
        }
        Ok(())
    }
}
