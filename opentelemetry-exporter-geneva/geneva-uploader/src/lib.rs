mod config_service;
pub mod ingestion_service;

mod uploader;

#[allow(unused_imports)]
pub(crate) use config_service::auth::AuthMethod;

#[allow(unused_imports)]
pub(crate) use config_service::geneva_config_info_client::{
    GenevaConfigClient, IngestionGatewayInfo,
};

#[allow(unused_imports)]
pub(crate) use config_service::geneva_ingestion_info_client::GenevaIngestionClient;

#[allow(unused_imports)]
pub(crate) use config_service::error::GenevaConfigClientError;

#[allow(unused_imports)]
pub(crate) use ingestion_service::uploader::{
    GenevaUploader, GenevaUploaderConfig, GenevaUploaderError, IngestionResponse, Result,
};

pub use uploader::{create_uploader, GenevaUploader as Uploader};
