use opentelemetry_config::{
    model::Telemetry,
};

use serde::Deserialize;

// Applications specific configuration model
#[derive(Deserialize)]
pub struct Application {
    pub version: String,
    pub service: Service,
}

#[derive(Deserialize)]
pub struct Service {
    // Telemetry configuration uses the common Telemetry model
    pub telemetry: Telemetry,
}