//! OS resource detector
//!
//! Detect the runtime operating system type.
use opentelemetry::KeyValue;
use opentelemetry_sdk::resource::ResourceDetector;
use opentelemetry_sdk::Resource;
use std::env::consts::OS;

/// Detect runtime operating system information.
///
/// This detector uses Rust's [`OS constant`] to detect the operating system type and
/// maps the result to the supported value defined in [`OpenTelemetry spec`].
///
/// [`OS constant`]: https://doc.rust-lang.org/std/env/consts/constant.OS.html
/// [`OpenTelemetry spec`]: https://github.com/open-telemetry/opentelemetry-specification/blob/main/specification/resource/semantic_conventions/os.md
pub struct OsResourceDetector;

impl ResourceDetector for OsResourceDetector {
    fn detect(&self) -> Resource {
        Resource::builder_empty()
            .with_attributes(vec![KeyValue::new(
                opentelemetry_semantic_conventions::attribute::OS_TYPE,
                OS,
            )])
            .build()
    }
}

#[cfg(test)]
mod tests {
    use super::OsResourceDetector;
    use opentelemetry::{Key, Value};
    use opentelemetry_sdk::resource::ResourceDetector;

    #[test]
    fn test_os_resource_detector() {
        let resource = OsResourceDetector.detect();
        assert_eq!(resource.len(), 1);

        #[cfg(target_os = "linux")]
        let expected_os = "linux";

        #[cfg(target_os = "windows")]
        let expected_os = "windows";

        #[cfg(target_os = "macos")]
        let expected_os = "macos";

        assert_eq!(
            resource.get(&Key::from_static_str(
                opentelemetry_semantic_conventions::attribute::OS_TYPE
            )),
            Some(Value::from(expected_os))
        )
    }
}
