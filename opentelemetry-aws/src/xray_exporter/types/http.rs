use serde::Serialize;
use std::{borrow::Cow, marker::PhantomData};

use super::{
    error::Result,
    segment_document::builder_type::{DocumentBuilderType, Segment, Subsegment},
    utils::{verify_string_length, MaybeSkip},
};
use crate::{field_setter, flag_setter};

/// HTTP request and response information.
///
/// Records details about an HTTP request that your application served (in a segment)
/// or that your application made to a downstream HTTP API (in a subsegment).
#[derive(Debug, Serialize)]
pub(super) struct HttpData<'a> {
    /// Information about the HTTP request
    #[serde(skip_serializing_if = "MaybeSkip::skip")]
    request: HttpRequestData<'a>,

    /// Information about the HTTP response
    #[serde(skip_serializing_if = "MaybeSkip::skip")]
    response: HttpResponseData,
}

impl MaybeSkip for HttpData<'_> {
    /// Returns true if this HTTP data is empty (both request and response are empty)
    fn skip(&self) -> bool {
        self.request.skip() && self.response.skip()
    }
}

/// Builder for constructing HTTP request and response metadata.
#[derive(Debug, Default)]
pub(crate) struct HttpDataBuilder<'a, DBT: DocumentBuilderType> {
    pub request: HttpRequestDataBuilder<'a, DBT>,
    pub response: HttpResponseDataBuilder,
}

impl<'a, DBT: DocumentBuilderType> HttpDataBuilder<'a, DBT> {
    /// Builds the `HttpData` instance.
    pub(super) fn build(self) -> HttpData<'a> {
        HttpData {
            request: self.request.build(),
            response: self.response.build(),
        }
    }
}

/// Information about an HTTP request.
#[derive(Debug, Serialize)]
struct HttpRequestData<'a> {
    /// The HTTP method (e.g., GET, POST). Limited to 250 characters.
    #[serde(skip_serializing_if = "MaybeSkip::skip")]
    method: Option<Cow<'a, str>>,

    /// The full URL of the request. Limited to 250 characters.
    #[serde(skip_serializing_if = "MaybeSkip::skip")]
    url: Option<Cow<'a, str>>,

    /// The user agent string from the requester's client. Limited to 250 characters.
    #[serde(skip_serializing_if = "MaybeSkip::skip")]
    user_agent: Option<Cow<'a, str>>,

    /// The IP address of the requester. Limited to 250 characters.
    #[serde(skip_serializing_if = "MaybeSkip::skip")]
    client_ip: Option<Cow<'a, str>>,

    /// Boolean indicating that the client_ip was read from an X-Forwarded-For header
    /// and is not reliable as it could have been forged. (segments only)
    #[serde(skip_serializing_if = "MaybeSkip::skip")]
    x_forwarded_for: bool,

    /// Boolean indicating that the downstream call is to another traced service.
    /// If true, X-Ray considers the trace broken until the downstream service
    /// uploads a segment with a matching parent_id. (subsegments only)
    #[serde(skip_serializing_if = "MaybeSkip::skip")]
    traced: bool,
}

impl MaybeSkip for HttpRequestData<'_> {
    /// Returns true if this HTTP request data is empty (all fields are None/default)
    fn skip(&self) -> bool {
        self.method.skip()
            && self.url.skip()
            && self.user_agent.skip()
            && self.client_ip.skip()
            && !self.x_forwarded_for
            && !self.traced
    }
}

/// Builder for constructing HTTP request metadata.
#[derive(Debug, Default)]
pub(crate) struct HttpRequestDataBuilder<'a, DBT: DocumentBuilderType> {
    method: Option<Cow<'a, str>>,
    url: Option<Cow<'a, str>>,
    user_agent: Option<Cow<'a, str>>,
    client_ip: Option<Cow<'a, str>>,
    x_forwarded_for: bool,
    traced: bool,
    _phatom_data: PhantomData<DBT>,
}

impl HttpRequestDataBuilder<'_, Segment> {
    flag_setter!(x_forwarded_for);
}

impl HttpRequestDataBuilder<'_, Subsegment> {
    flag_setter!(traced);
}

impl<'a, DBT: DocumentBuilderType> HttpRequestDataBuilder<'a, DBT> {
    /// Sets the HTTP method.
    ///
    /// # Arguments
    ///
    /// * `method` - The HTTP method (e.g., GET, POST)
    ///
    /// # Errors
    ///
    /// Returns `ConstraintError::StringTooLong(250)` if the method is longer than 250 characters.
    pub fn method(&mut self, method: Cow<'a, str>) -> Result<&mut Self> {
        verify_string_length(method.as_ref(), 250)?;
        self.method = Some(method);
        Ok(self)
    }

    /// Sets the URL.
    ///
    /// # Arguments
    ///
    /// * `url` - The full URL of the request
    ///
    /// # Errors
    ///
    /// Returns `ConstraintError::StringTooLong(250)` if the URL is longer than 250 characters.
    pub fn url(&mut self, url: Cow<'a, str>) -> Result<&mut Self> {
        verify_string_length(url.as_ref(), 250)?;
        self.url = Some(url);
        Ok(self)
    }

    /// Sets the user agent.
    ///
    /// # Arguments
    ///
    /// * `user_agent` - The user agent string from the requester's client
    ///
    /// # Errors
    ///
    /// Returns `ConstraintError::StringTooLong(250)` if the user agent is longer than 250 characters.
    pub fn user_agent(&mut self, user_agent: Cow<'a, str>) -> Result<&mut Self> {
        verify_string_length(user_agent.as_ref(), 250)?;
        self.user_agent = Some(user_agent);
        Ok(self)
    }

    /// Sets the client IP.
    ///
    /// # Arguments
    ///
    /// * `client_ip` - The IP address of the requester
    ///
    /// # Errors
    ///
    /// Returns `ConstraintError::StringTooLong(250)` if the client IP is longer than 250 characters.
    pub fn client_ip(&mut self, client_ip: Cow<'a, str>) -> Result<&mut Self> {
        verify_string_length(client_ip.as_ref(), 250)?;
        self.client_ip = Some(client_ip);
        Ok(self)
    }

    /// Builds the `HttpRequestData` instance.
    fn build(self) -> HttpRequestData<'a> {
        HttpRequestData {
            method: self.method,
            url: self.url,
            user_agent: self.user_agent,
            client_ip: self.client_ip,
            x_forwarded_for: self.x_forwarded_for,
            traced: self.traced,
        }
    }
}

/// Information about an HTTP response.
#[derive(Debug, Serialize, Clone, Copy)]
pub(super) struct HttpResponseData {
    /// The HTTP status code of the response
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<u16>,

    /// The length of the response body in bytes
    #[serde(skip_serializing_if = "Option::is_none")]
    content_length: Option<i64>,
}

impl MaybeSkip for HttpResponseData {
    /// Returns true if this HTTP response data is empty (all fields are None)
    fn skip(&self) -> bool {
        self.status.is_none() && self.content_length.is_none()
    }
}

/// Builder for constructing HTTP response metadata.
#[derive(Debug, Default)]
pub(crate) struct HttpResponseDataBuilder {
    status: Option<u16>,
    content_length: Option<i64>,
}

impl HttpResponseDataBuilder {
    field_setter!(status:u16);
    field_setter!(content_length:i64);

    /// Builds the `HttpResponseData` instance.
    pub(super) fn build(self) -> HttpResponseData {
        HttpResponseData {
            status: self.status,
            content_length: self.content_length,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xray_exporter::types::error::ConstraintError;
    use std::borrow::Cow;

    // Helper function to create a string of specified length
    fn create_string(len: usize) -> String {
        "a".repeat(len)
    }

    // Tests for HttpRequestDataBuilder validation methods

    #[test]
    fn http_request_builder_validation_valid() {
        let mut builder = HttpRequestDataBuilder::<Segment>::default();

        // Valid method (exactly 250 chars)
        let method_250 = create_string(250);
        assert!(builder.method(Cow::Owned(method_250)).is_ok());

        // Valid url (less than 250 chars)
        let url = "https://example.com/api/v1/resource";
        assert!(builder.url(Cow::Borrowed(url)).is_ok());

        // Valid user_agent (typical length)
        let user_agent = "Mozilla/5.0 (Windows NT 10.0; Win64; x64)";
        assert!(builder.user_agent(Cow::Borrowed(user_agent)).is_ok());

        // Valid client_ip (IPv4)
        let client_ip = "192.168.1.1";
        assert!(builder.client_ip(Cow::Borrowed(client_ip)).is_ok());

        // Valid client_ip (IPv6)
        let client_ip_v6 = "2001:0db8:85a3:0000:0000:8a2e:0370:7334";
        assert!(builder.client_ip(Cow::Borrowed(client_ip_v6)).is_ok());
    }

    #[test]
    fn http_request_builder_validation_invalid() {
        let mut builder = HttpRequestDataBuilder::<Segment>::default();

        // Invalid method (251 chars - exceeds limit)
        let method_251 = create_string(251);
        let result = builder.method(Cow::Owned(method_251));
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), ConstraintError::StringTooLong(250));

        // Invalid url (over 250 chars)
        let url_long = create_string(300);
        let result = builder.url(Cow::Owned(url_long));
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), ConstraintError::StringTooLong(250));

        // Invalid user_agent (over 250 chars)
        let user_agent_long = create_string(500);
        let result = builder.user_agent(Cow::Owned(user_agent_long));
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), ConstraintError::StringTooLong(250));

        // Invalid client_ip (over 250 chars)
        let client_ip_long = create_string(260);
        let result = builder.client_ip(Cow::Owned(client_ip_long));
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), ConstraintError::StringTooLong(250));
    }
}
