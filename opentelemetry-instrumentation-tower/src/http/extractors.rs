//! Pluggable extractors shared by the HTTP server and client layers.
//!
//! - [`RouteExtractor`] decides how the `http.route` attribute (and span name) is
//!   produced.
//! - [`RequestAttributeExtractor`] / [`ResponseAttributeExtractor`] let you attach
//!   additional attributes to spans and metrics.

use opentelemetry::KeyValue;

#[cfg(feature = "axum")]
use axum::extract::MatchedPath;

/// Trait for extracting the route/target from HTTP requests.
///
/// Implementations return an optional route string. When present, this is used for:
/// - Span names: `"{method} {route}"` (e.g., `"GET /users/:id"`)
/// - The `http.route` metric attribute
///
/// When `None` is returned, span names use method-only (e.g., `"GET"`) and no
/// `http.route` attribute is set.
///
/// # Cardinality Considerations
///
/// The route should be a low-cardinality value (e.g., a route template like `/users/:id`)
/// rather than the actual path (e.g., `/users/123`). High-cardinality routes can overwhelm
/// the OpenTelemetry SDK's built-in cardinality limits and downstream backends.
pub trait RouteExtractor<B>: Clone + Send + Sync + 'static {
    /// Extracts the route from the request, if available.
    ///
    /// Returns `None` to use method-only span names and skip the `http.route` attribute.
    fn extract_route(&self, req: &http::Request<B>) -> Option<String>;
}

/// Route extractor that returns no route (method-only span names).
///
/// This is the safest option as it avoids cardinality explosion from dynamic
/// path segments. Span names will be just the HTTP method (e.g., `"GET"`).
///
/// The `http.route` attribute will not be set when using this extractor.
#[derive(Clone, Default)]
pub struct NoRouteExtractor;

impl<B> RouteExtractor<B> for NoRouteExtractor {
    fn extract_route(&self, _req: &http::Request<B>) -> Option<String> {
        None
    }
}

/// Route extractor that uses Axum's `MatchedPath` for low-cardinality routes.
///
/// This extractor uses the route template (e.g., `/users/:id`) instead of the actual
/// path (e.g., `/users/123`), providing low-cardinality span names and route attributes
/// that are safe for production use.
///
/// # When `MatchedPath` is unavailable
///
/// Returns `None` (falling back to method-only span names) when `MatchedPath` is not
/// present in the request extensions. This can happen when:
///
/// - The OpenTelemetry layer is placed *before* Axum's router in the middleware stack.
///   The layer must be placed *after* the router to access route information.
/// - The request does not match any defined route (404 responses).
/// - Using Axum's `fallback` handler, which does not set `MatchedPath`.
///
/// For correct route extraction, ensure the middleware order is:
///
/// ```ignore
/// let app = Router::new()
///     .route("/users/:id", get(handler))
///     .layer(otel_layer);  // Layer applied after routes
/// ```
///
/// See the [Axum documentation on middleware ordering](https://docs.rs/axum/latest/axum/middleware/index.html#ordering)
/// for more details.
#[cfg(feature = "axum")]
#[derive(Clone, Default)]
pub struct AxumMatchedPathExtractor;

#[cfg(feature = "axum")]
impl<B> RouteExtractor<B> for AxumMatchedPathExtractor {
    fn extract_route(&self, req: &http::Request<B>) -> Option<String> {
        req.extensions()
            .get::<MatchedPath>()
            .map(|matched_path| matched_path.as_str().to_owned())
    }
}

/// Route extractor that uses the URL path (without query parameters).
///
/// # Warning: Cardinality
///
/// Using this extractor can cause **high cardinality** issues if your routes contain
/// dynamic path segments (e.g., `/users/{id}`, `/orders/{order_id}/items/{item_id}`).
/// Each unique path will create a unique span name and route attribute, potentially
/// overwhelming your tracing and metrics backends with millions of unique series.
///
/// **Only use this if**:
/// - Your routes are static (no path parameters)
/// - You understand and accept the cardinality implications
///
/// Consider using a custom [`FnRouteExtractor`] with path normalization instead.
#[derive(Clone, Default)]
pub struct PathExtractor;

impl<B> RouteExtractor<B> for PathExtractor {
    fn extract_route(&self, req: &http::Request<B>) -> Option<String> {
        Some(req.uri().path().to_owned())
    }
}

/// A function-based route extractor.
#[derive(Clone)]
pub struct FnRouteExtractor<F> {
    extractor: F,
}

impl<F> FnRouteExtractor<F> {
    pub fn new(extractor: F) -> Self {
        Self { extractor }
    }
}

impl<F, B> RouteExtractor<B> for FnRouteExtractor<F>
where
    F: Fn(&http::Request<B>) -> Option<String> + Clone + Send + Sync + 'static,
{
    fn extract_route(&self, req: &http::Request<B>) -> Option<String> {
        (self.extractor)(req)
    }
}

#[cfg(feature = "axum")]
pub(crate) type DefaultRouteExtractor = AxumMatchedPathExtractor;

#[cfg(not(feature = "axum"))]
pub(crate) type DefaultRouteExtractor = NoRouteExtractor;

/// Trait for extracting custom attributes from HTTP requests.
pub trait RequestAttributeExtractor<B>: Clone + Send + Sync + 'static {
    fn extract_attributes(&self, req: &http::Request<B>) -> Vec<KeyValue>;
}

/// Trait for extracting custom attributes from HTTP responses.
pub trait ResponseAttributeExtractor<B>: Clone + Send + Sync + 'static {
    fn extract_attributes(&self, res: &http::Response<B>) -> Vec<KeyValue>;
}

/// Default implementation that extracts no attributes.
#[derive(Clone)]
pub struct NoOpExtractor;

impl<B> RequestAttributeExtractor<B> for NoOpExtractor {
    fn extract_attributes(&self, _req: &http::Request<B>) -> Vec<KeyValue> {
        vec![]
    }
}

impl<B> ResponseAttributeExtractor<B> for NoOpExtractor {
    fn extract_attributes(&self, _res: &http::Response<B>) -> Vec<KeyValue> {
        vec![]
    }
}

/// A function-based request attribute extractor.
#[derive(Clone)]
pub struct FnRequestExtractor<F> {
    extractor: F,
}

impl<F> FnRequestExtractor<F> {
    pub fn new(extractor: F) -> Self {
        Self { extractor }
    }
}

impl<F, B> RequestAttributeExtractor<B> for FnRequestExtractor<F>
where
    F: Fn(&http::Request<B>) -> Vec<KeyValue> + Clone + Send + Sync + 'static,
{
    fn extract_attributes(&self, req: &http::Request<B>) -> Vec<KeyValue> {
        (self.extractor)(req)
    }
}

/// A function-based response attribute extractor.
#[derive(Clone)]
pub struct FnResponseExtractor<F> {
    extractor: F,
}

impl<F> FnResponseExtractor<F> {
    pub fn new(extractor: F) -> Self {
        Self { extractor }
    }
}

impl<F, B> ResponseAttributeExtractor<B> for FnResponseExtractor<F>
where
    F: Fn(&http::Response<B>) -> Vec<KeyValue> + Clone + Send + Sync + 'static,
{
    fn extract_attributes(&self, res: &http::Response<B>) -> Vec<KeyValue> {
        (self.extractor)(res)
    }
}
