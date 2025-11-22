//! # Metrics Configuration module
//!
//! This module defines the configuration structures for Metrics telemetry
//! used in OpenTelemetry SDKs.

pub mod reader;

use serde::Deserialize;

use crate::model::metrics::reader::Reader;

/// Configuration for Metrics
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub(crate) struct Metrics {
    /// Readers configuration for Metrics telemetry
    pub(crate) readers: Vec<Reader>,
}
