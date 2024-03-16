//! OS resource detector
//!
//! Detect the runtime operating system type.
use opentelemetry::KeyValue;
use opentelemetry_sdk::resource::ResourceDetector;
use opentelemetry_sdk::Resource;
use std::env::consts::OS;
use std::time::Duration;

/// Detect runtime operating system information.
///
/// This detector uses Rust's [`OS constant`] to detect the operating system type and
/// maps the result to the supported value defined in [`OpenTelemetry spec`].
///
/// [`OS constant`]: https://doc.rust-lang.org/std/env/consts/constant.OS.html
/// [`OpenTelemetry spec`]: https://github.com/open-telemetry/opentelemetry-specification/blob/main/specification/resource/semantic_conventions/os.md
pub struct OsResourceDetector;

impl ResourceDetector for OsResourceDetector {
    fn detect(&self, _timeout: Duration) -> Resource {
        Resource::new(vec![KeyValue::new(
            opentelemetry_semantic_conventions::resource::OS_TYPE,
            OS,
        )])
    }
}

#[cfg(target_os = "linux")]
#[cfg(test)]
mod tests {
    use super::OsResourceDetector;
    use opentelemetry::{Key, Value};
    use opentelemetry_sdk::resource::ResourceDetector;
    use std::time::Duration;

    #[test]
    fn test_os_resource_detector() {
        let resource = OsResourceDetector.detect(Duration::from_secs(0));
        assert_eq!(resource.len(), 1);
        assert_eq!(
            resource.get(Key::from_static_str(
                opentelemetry_semantic_conventions::resource::OS_TYPE
            )),
            Some(Value::from("linux"))
        )
    }
}
