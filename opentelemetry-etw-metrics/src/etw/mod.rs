use opentelemetry::{
    global,
    metrics::{MetricsError, Result},
};

use std::sync::Once;

tracelogging::define_provider!(ETW_PROVIDER, "OpenTelemetry-ETW-Metrics");

static ETW_PROVIDER_REGISTRANT: Once = Once::new();

/// Protocol constant
const PROTOCOL_FIELD_VALUE: u32 = 0;
/// Protobuf definition version
const PROTOBUF_VERSION: &[u8; 8] = b"v0.19.00";

/// Safely register the ETW provider.
pub fn register() {
    // # Safety
    // The following preconditions must be satisfied to safely register the ETW_PROVIDER:
    // - The ETW_PROVIDER must not have already been registered.
    // - Another thread cannot call register or unregister at the same time.
    // The first precondition is upheld as `std::sync::Once` guarantees that the closure will only be called once.
    // The second precondition is upheld as calls to `unregister` will not occur unless the ETW_PROVIDER has been registered (checked using the `is_completed` method of `std::sync::Once`)
    // which guarantees that a call to `unregister` will not occur as `register` is occurring. There is a chancer that `unregister`
    // will do nothing if `register` is ongoing but this is not unsound.
    ETW_PROVIDER_REGISTRANT.call_once(|| match unsafe { ETW_PROVIDER.register() } {
        0 => {
            println!("Successfully registered ETW provider")
        }
        error_code => {
            global::handle_error(MetricsError::Other(format!(
                "Failed to register ETW provider with error code: {}",
                error_code
            )));
        }
    });
}

/// Write an event to the ETW provider.
pub fn write(buffer: &[u8]) -> u32 {
    tracelogging::write_event!(
        ETW_PROVIDER,
        "otlp_metrics",
        level(tracelogging::Level::Informational),
        u32("PROTOCOL_FIELD_VALUE", &PROTOCOL_FIELD_VALUE),
        binary("PROTOBUF_VERSION", PROTOBUF_VERSION),
        binary("BUFFER", &buffer)
    )
}

/// Unregister the already registered ETW provider or do nothing.
/// TODO: Figure out how to make sure that we unregister even if the process is killed or the library is unloaded.
pub fn unregister() {
    if ETW_PROVIDER_REGISTRANT.is_completed() {
        match ETW_PROVIDER.unregister() {
            0 => println!("Successfully unregistered ETW provider"),
            error_code => eprintln!(
                "Failed to unregister ETW provider with error code: {}",
                error_code
            ),
        }
    } else {
        println!("ETW provider is not registered so there is nothing to unregister.");
    }
}
