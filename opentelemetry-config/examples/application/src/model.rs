use serde::Deserialize;
use serde_yaml::Value;

// Applications specific configuration model
#[derive(Deserialize)]
pub struct Application {
    pub version: String,
    pub service: Service,
}

#[derive(Deserialize)]
pub struct Service {
    // Telemetry configuration uses the common Telemetry model
    pub telemetry: Value,
}
