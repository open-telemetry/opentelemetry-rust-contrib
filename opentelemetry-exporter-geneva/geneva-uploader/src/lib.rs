mod config_service;
pub mod ingestion_service;
pub(crate) mod payload_encoder;

mod uploader;

#[cfg(test)]
mod bench;

#[allow(unused_imports)]
pub(crate) use config_service::client::{
    AuthMethod, GenevaConfigClient, GenevaConfigClientConfig, GenevaConfigClientError,
    IngestionGatewayInfo,
};

#[allow(unused_imports)]
pub(crate) use ingestion_service::uploader::{
    GenevaUploader, GenevaUploaderConfig, GenevaUploaderError, IngestionResponse, Result,
};

pub use uploader::{create_uploader, GenevaUploader as Uploader};
