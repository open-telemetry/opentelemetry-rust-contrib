// To run the test, execute the following command in the stress directory as sudo:
// sudo -E ~/.cargo/bin/cargo rnu --bin user-events-enabled --release

// TODO : Add stess result here.

mod throughput;
use eventheader_dynamic::{Provider, ProviderOptions};
use lazy_static::lazy_static;

// Global constants for level and keyword
const LEVEL: u8 = 4; // Example level (Informational)
const KEYWORD: u64 = 0x01; // Example keyword

lazy_static! {
    static ref PROVIDER: Provider = {
        // Initialize the Provider with dynamic options
        let mut options = ProviderOptions::new();
        options = *options.group_name("testprovider");
        let mut provider = Provider::new("testprovider", &options);

        // Register events with specific levels and keywords
        provider.register_set(LEVEL.into(), KEYWORD);

        provider
    };
}

fn main() {
    // Execute the throughput test with the test_log function
    throughput::test_throughput(test_user_events_enabled);
}

fn test_user_events_enabled() {
    // Find and check if the event is enabled
    if let Some(event_set) = PROVIDER.find_set(LEVEL.into(), KEYWORD) {
        let _ = event_set.enabled(); // Perform the enabled check
    }
}
