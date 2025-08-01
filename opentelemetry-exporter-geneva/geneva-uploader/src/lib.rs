mod config_service;
mod ingestion_service;
pub mod payload_encoder;
pub mod retry;

pub mod client;

#[cfg(test)]
mod bench;

#[allow(unused_imports)]
pub(crate) use config_service::client::{
    GenevaConfigClient, GenevaConfigClientConfig, GenevaConfigClientError, IngestionGatewayInfo,
};

#[allow(unused_imports)]
pub(crate) use ingestion_service::uploader::{
    GenevaUploader, GenevaUploaderConfig, GenevaUploaderError, IngestionResponse, Result,
};

pub use client::{GenevaClient, GenevaClientConfig};
pub use config_service::client::AuthMethod;
pub use retry::RetryConfig;
