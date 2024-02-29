use opentelemetry::{global, metrics::MetricsError};

use tracelogging as tlg;

use std::sync::Once;

tlg::define_provider!(
    PROVIDER,
    "NativeMetricsExtension_Provider",
    id("EDC24920-E004-40F6-A8E1-0E6E48F39D84") // GUID is defined in https://github.com/microsoft/common-schema/blob/main/v4.0/Mappings/OTelMetrics.md
);

static ETW_PROVIDER_REGISTRANT: Once = Once::new();

/// Register the ETW provider.
pub fn register() {
    // # Safety
    //
    // The following preconditions must be satisfied to safely register PROVIDER:
    // - The PROVIDER must not have already been registered.
    // - Another thread cannot call register or unregister at the same time.
    // The first precondition is upheld as `std::sync::Once` guarantees that the closure will only be called once.
    // The second precondition is upheld as calls to `unregister` will not occur unless the PROVIDER has been registered (checked using the `is_completed` method of `std::sync::Once`)
    // which guarantees that a call to `unregister` will not occur as `register` is occurring. There is a chance that `unregister`
    // will do nothing if `register` is ongoing but this is not unsound.
    ETW_PROVIDER_REGISTRANT.call_once(|| match unsafe { PROVIDER.register() } {
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
        PROVIDER,
        "otlp_metrics", // Name is defined in https://github.com/microsoft/common-schema/blob/main/v4.0/Mappings/OTelMetrics.md
        id_version(81, 0), // Event id 81 is defined in https://github.com/microsoft/common-schema/blob/main/v4.0/Mappings/OTelMetrics.md
        level(tracelogging::Level::Informational),
        raw_data(&buffer)
    )
}

/// Unregister the provider.
pub fn unregister() {
    if ETW_PROVIDER_REGISTRANT.is_completed() {
        match PROVIDER.unregister() {
            0 => println!("Successfully unregistered ETW provider"),
            error_code => global::handle_error(MetricsError::Other(format!(
                "Failed to unregister ETW provider with error code: {}",
                error_code
            ))),
        }
    } else {
        println!("ETW provider is not registered so there is nothing to unregister.");
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn register() {
        super::register();
    }

    #[test]
    fn multiple_register_calls_succeed() {
        super::register();
        super::register();
    }

    #[test]
    fn multiple_unregister_calls_succeed() {
        super::register();

        super::unregister();
        super::unregister();
    }
}
