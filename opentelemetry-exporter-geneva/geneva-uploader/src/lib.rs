mod common;
mod config_service;
mod ingestion_service;
mod payload_encoder;

pub mod client;

#[cfg(test)]
mod bench;

#[allow(unused_imports)]
pub(crate) use config_service::client::{
    GenevaConfigClient, GenevaConfigClientConfig, GenevaConfigClientError, IngestionGatewayInfo,
};

#[allow(unused_imports)]
pub(crate) use ingestion_service::uploader::{
    GenevaUploader, GenevaUploaderConfig, GenevaUploaderError, Result,
};

pub use client::EncodedBatch;
pub use client::{GenevaClient, GenevaClientConfig};
pub use config_service::client::AuthMethod;
