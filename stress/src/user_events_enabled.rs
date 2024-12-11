mod throughput;
use eventheader_dynamic::{Provider, ProviderOptions};
use lazy_static::lazy_static;

lazy_static! {
    static ref PROVIDER: Provider = {
        // Initialize the Provider with dynamic options
        let mut options = ProviderOptions::new();
        options = *options.group_name("testprovider");
        let mut provider = Provider::new("testprovider", &options);

        // Register events with specific levels and keywords
        let keyword = 0x01; // Example keyword
        let level = 4; // Example level (Informational)
        provider.register_set(level.into(), keyword);

        provider
    };
}

fn main() {
    // Execute the throughput test with the test_log function
    throughput::test_throughput(test_user_events_enabled);
}

fn test_user_events_enabled() {
    let level = 4; // Informational level
    let keyword = 0x01; // Example keyword

    // Find and check if the event is enabled
    if let Some(event_set) = PROVIDER.find_set(level.into(), keyword) {
        println!("echk for enableD");
        let _ = event_set.enabled(); // Perform the enabled check
    }
}
