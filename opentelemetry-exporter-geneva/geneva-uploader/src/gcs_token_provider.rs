use secrecy::SecretString;
use std::error::Error;
use std::fmt::Debug;
use std::future::Future;
use std::pin::Pin;

/// Error returned by an external GCS token provider.
pub type GcsTokenProviderError = Box<dyn Error + Send + Sync + 'static>;

/// Future returned by [`GcsTokenProvider::get_token`].
pub type GcsTokenFuture<'a> =
    Pin<Box<dyn Future<Output = Result<SecretString, GcsTokenProviderError>> + Send + 'a>>;

/// Supplies Entra access tokens used to authenticate Geneva Config Service requests.
///
/// Implementations own identity selection, token acquisition, caching, refresh,
/// retry policy, and execution policy. The returned token must be valid for
/// `resource`, sufficiently fresh for a GCS request, and must not include an
/// authorization scheme prefix.
///
/// Calls may occur concurrently. Implementations must not perform blocking work
/// while their returned future is polled on the caller's async executor.
pub trait GcsTokenProvider: Send + Sync + Debug {
    /// Returns a current Entra access token for `resource`.
    fn get_token<'a>(&'a self, resource: &'a str) -> GcsTokenFuture<'a>;
}
