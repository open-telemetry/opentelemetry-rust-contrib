use std::borrow::Cow;

use super::utils::MaybeSkip;
use crate::field_setter;
use serde::Serialize;

/// Service version information.
#[derive(Debug, Serialize)]
pub(super) struct ServiceData<'a> {
    /// The version of the service that handled the request
    #[serde(skip_serializing_if = "MaybeSkip::skip")]
    version: Option<Cow<'a, str>>,
}

impl MaybeSkip for ServiceData<'_> {
    /// Returns true if this service data is empty (version field is None)
    fn skip(&self) -> bool {
        self.version.skip()
    }
}

/// Builder for constructing service version metadata.
#[derive(Debug, Default)]
pub(crate) struct ServiceDataBuilder<'a> {
    version: Option<Cow<'a, str>>,
}

impl<'a> ServiceDataBuilder<'a> {
    field_setter!(version);

    /// Builds the `ServiceData` instance.
    pub(super) fn build(self) -> ServiceData<'a> {
        ServiceData {
            version: self.version,
        }
    }
}
