//! Instrumentation scope helpers: the shared instrumentation name and accessors
//! for the global tracer and meter.

use std::sync::Arc;

use opentelemetry::global::{self, BoxedTracer};
use opentelemetry::metrics::Meter;

pub(crate) const INSTRUMENTATION_NAME: &str = "opentelemetry-instrumentation-tower";

/// Returns a boxed tracer bound to the global tracer provider.
pub(crate) fn tracer() -> Arc<BoxedTracer> {
    Arc::new(global::tracer(INSTRUMENTATION_NAME))
}

/// Returns a meter bound to the global meter provider.
pub(crate) fn meter() -> Meter {
    global::meter(INSTRUMENTATION_NAME)
}
