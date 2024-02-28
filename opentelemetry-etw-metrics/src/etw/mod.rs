use opentelemetry::{
    global,
    metrics::{MetricsError, Result},
};

use lazy_static::lazy_static;
use tracelogging_dynamic as tld;

use std::{collections::HashSet, pin::Pin, sync::Mutex};

/// Protocol constant
const PROTOCOL_FIELD_VALUE: u32 = 0;
/// Protobuf definition version
const PROTOBUF_VERSION: &[u8; 8] = b"v0.19.00";

lazy_static! {
    static ref REGISTERED_PROVIDERS: Mutex<HashSet<String>> = Mutex::new(HashSet::new());
}

#[derive(Debug)]
pub struct ProviderGuard {
    provider: Pin<Box<tld::Provider>>,
}

impl ProviderGuard {
    /// Register the ETW provider.
    pub fn register(name: &str) -> Result<Self> {
        // Ensure that no other thread has registered a provider with the same name.
        let mut registered_providers = REGISTERED_PROVIDERS.lock().unwrap();
        if registered_providers.contains(name) {
            return Err(MetricsError::Other(
                format!("Failed to register ETW provider named {}: provider with the same name already registered", name)
            ));
        }
        registered_providers.insert(name.to_string());
        // Release the lock so other threads can register providers.
        // Releasing before registering the provider is safe because only one thread can register a provider with the same name.
        drop(registered_providers);

        let provider = Box::pin(tld::Provider::new_with_id(
            name,
            &tld::Provider::options(), // Use default options
            &tld::Guid::try_parse("{EDC24920-E004-40F6-A8E1-0E6E48F39D84}").unwrap(), // This GUID is defined in https://github.com/microsoft/common-schema/blob/main/v4.0/Mappings/OTelMetrics.md
        ));

        // # Safety
        // The following preconditions must be satisfied to safely register the ETW_PROVIDER:
        // - The provider must not have already been registered.
        // - For a given provider, a call to `register` must not occur concurrently calls to either `register` or `unregister`.
        //
        // The first precondition is upheld as `registered_providers` is used to synchronize 'registration' of providers.
        // No thread can register a provider with the same name as an already registered provider.
        //
        // The second precondition is upheld as:
        // - No two `ProviderGuards` can register the same provider so concurrent `register` or `unregister` calls are not possible.
        // - A `ProviderGuard` will have completely `register`ed its provider during initialization so `unregister`ing can only
        // happen after `register`ing is complete.
        match unsafe { provider.as_ref().register() } {
            0 => {
                println!("Successfully registered ETW provider named: {}", name);
            }
            error_code => {
                global::handle_error(MetricsError::Other(format!(
                    "Failed to register ETW provider named {} with error code: {}",
                    name, error_code
                )));
            }
        }

        Ok(ProviderGuard { provider })
    }

    /// Write an event to the ETW provider.
    pub fn write(&self, buffer: &[u8]) -> u32 {
        tld::EventBuilder::new()
            .reset("otlp_metrics", tld::Level::Informational, 1, 0) // otlp_metrics is defined in https://github.com/microsoft/common-schema/blob/main/v4.0/Mappings/OTelMetrics.md
            .id_version(81, 0) // Event id 81 is defined in https://github.com/microsoft/common-schema/blob/main/v4.0/Mappings/OTelMetrics.md
            .raw_add_data_slice(buffer)
            .write(&self.provider, None, None)
    }

    /// Unregister the provider.
    pub fn unregister(&self) {
        match self.provider.unregister() {
            0 => println!(
                "Successfully unregistered ETW provider named {}",
                self.provider.name()
            ),
            error_code => eprintln!(
                "Failed to unregister ETW provider with error code: {}",
                error_code
            ),
        }

        let mut registered_providers = REGISTERED_PROVIDERS.lock().unwrap();
        if !registered_providers.remove(self.provider.name()) {
            eprintln!(
                "Could not remove provider name from set of registered provider names as the set did not contain provider name: {}",
                self.provider.name()
            );
        }
    }
}

impl Drop for ProviderGuard {
    fn drop(&mut self) {
        // `Provider::Unregister` is called when a `Provider` is dropped, so we must ensure to remove the provider name
        // from the set of registered providers.
        self.unregister();
    }
}

#[cfg(test)]
mod tests {
    use super::{ProviderGuard, REGISTERED_PROVIDERS};

    use opentelemetry::metrics::MetricsError;

    #[test]
    fn register() {
        let _provider = ProviderGuard::register("provider_name").unwrap();

        {
            let registered_providers = REGISTERED_PROVIDERS.lock().unwrap();
            assert!(registered_providers.contains("provider_name"));
        }
    }

    #[test]
    fn duplicate_provider_name_registration_fails() {
        let _provider = ProviderGuard::register("duplicate_provider_name").unwrap();

        let result = ProviderGuard::register("duplicate_provider_name");

        assert!(result.is_err());
        assert_eq!(result
            .unwrap_err().to_string(), MetricsError::Other("Failed to register ETW provider named duplicate_provider_name: provider with the same name already registered".to_string()).to_string());
    }

    #[test]
    fn multiple_unregister_calls_succeed() {
        let provider = ProviderGuard::register("multiple_unregister_calls").unwrap();

        {
            let registered_providers = REGISTERED_PROVIDERS.lock().unwrap();
            assert!(registered_providers.contains("multiple_unregister_calls"));
        }

        provider.unregister();
        provider.unregister();

        {
            let registered_providers = REGISTERED_PROVIDERS.lock().unwrap();
            assert!(!registered_providers.contains("multiple_unregister_calls"));
        }
    }
}
