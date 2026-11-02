mod config_service;
mod gcs_token_provider;
mod ingestion_service;
mod payload_encoder;
mod secret_header;

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
pub use client::{
    AccountRouting, AgentFedCredential, AgentFedCredentialFuture, AgentFedCredentialSource,
};
pub use client::{
    GenevaClient, GenevaClientConfig, LogsConfig, LogsEventNameMapping, LogsEventNameRoutingKey,
    SpanEventNameMapping, SpanEventNameRoutingKey, TracesConfig, UploadError,
};
pub use config_service::client::AuthMethod;
pub use gcs_token_provider::{GcsTokenFuture, GcsTokenProvider, GcsTokenProviderError};
