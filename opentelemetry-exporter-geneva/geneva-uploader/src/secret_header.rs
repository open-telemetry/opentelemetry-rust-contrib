use bytes::Bytes;
use reqwest::header::{HeaderValue, InvalidHeaderValue};
use secrecy::zeroize::Zeroizing;
use secrecy::{ExposeSecret, SecretString};

pub(crate) fn header_value_from_zeroizing_buffer(
    value: Zeroizing<Vec<u8>>,
) -> Result<HeaderValue, InvalidHeaderValue> {
    let mut header = HeaderValue::from_maybe_shared(Bytes::from_owner(value))?;
    header.set_sensitive(true);
    Ok(header)
}

pub(crate) fn bearer_authorization_header(
    token: &SecretString,
) -> Result<HeaderValue, InvalidHeaderValue> {
    let token = token.expose_secret().as_bytes();
    let mut value = Zeroizing::new(Vec::with_capacity(b"Bearer ".len() + token.len()));
    value.extend_from_slice(b"Bearer ");
    value.extend_from_slice(token);
    header_value_from_zeroizing_buffer(value)
}
