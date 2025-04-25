//! # Route Formatter
//!
//! Format routes from paths.

/// Interface for formatting routes from paths.
///
/// This crate will render the actix web [match pattern] by default. E.g. for
/// path `/users/123/profile` the route for this span would be
/// `/users/{id}/profile`.
///
/// [match pattern]: actix_web::HttpRequest::match_pattern
///
/// # Custom Formatter Examples
///
/// ```
/// use opentelemetry_instrumentation_actix_web::RouteFormatter;
///
/// // A formatter to ensure all paths are reported as lowercase.
/// #[derive(Debug)]
/// struct MyLowercaseFormatter;
///
/// impl RouteFormatter for MyLowercaseFormatter {
///     fn format(&self, path: &str) -> String {
///         path.to_lowercase()
///     }
/// }
///
/// // now a match with pattern `/USERS/{id}` would be recorded as `/users/{id}`
/// ```
pub trait RouteFormatter: std::fmt::Debug {
    /// Function from path to route.
    /// e.g. /users/123 -> /users/:id
    fn format(&self, path: &str) -> String;
}
