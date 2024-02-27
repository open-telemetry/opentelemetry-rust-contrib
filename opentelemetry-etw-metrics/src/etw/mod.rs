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

pub struct ProviderGuard {
    provider: Pin<Box<tld::Provider>>,
}

impl ProviderGuard {
    /// Register the ETW provider.
    pub fn register(
        name: &str,
        id: Option<tld::Guid>,
        options: Option<tld::ProviderOptions>,
    ) -> Result<Self> {
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

        let options = options.unwrap_or(tld::Provider::options());

        let provider = match id {
            Some(id) => Box::pin(tld::Provider::new_with_id(name, &options, &id)),
            None => Box::pin(tld::Provider::new(name, &options)),
        };

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
                println!("Successfully registered ETW provider")
            }
            error_code => {
                global::handle_error(MetricsError::Other(format!(
                    "Failed to register ETW provider with error code: {}",
                    error_code
                )));
            }
        }

        Ok(ProviderGuard { provider })
    }

    /// Write an event to the ETW provider.
    pub fn write(&self, buffer: &[u8]) -> u32 {
        tld::EventBuilder::new()
            .reset("otlp_metrics", tld::Level::Informational, 1, 0)
            .add_u32(
                "PROTOCOL_FIELD_VALUE",
                PROTOCOL_FIELD_VALUE,
                tld::OutType::Default,
                0,
            )
            .add_binary(
                "PROTOBUF_VERSION",
                PROTOBUF_VERSION,
                tld::OutType::Default,
                0,
            )
            .add_binary("BUFFER", buffer, tld::OutType::Default, 0)
            .write(&self.provider, None, None)
    }

    /// Unregister the provider.
    /// TODO: Figure out how to make sure that we unregister even if the process is killed or the library is unloaded.
    pub fn unregister(&self) {
        match self.provider.unregister() {
            0 => println!("Successfully unregistered ETW provider"),
            error_code => eprintln!(
                "Failed to unregister ETW provider with error code: {}",
                error_code
            ),
        }

        let mut registered_providers = REGISTERED_PROVIDERS.lock().unwrap();
        registered_providers.remove(self.provider.name());
    }
}

impl Drop for ProviderGuard {
    fn drop(&mut self) {
        // `Provider::Unregister` is called when a `Provider` is dropped, so we must ensure to remove the provider name
        // from the set of registered providers.
        self.unregister();
    }
}
