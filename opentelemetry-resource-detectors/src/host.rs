//! HOST resource detector
//!
//! Detect the unique host ID.
use opentelemetry::KeyValue;
use opentelemetry_sdk::resource::ResourceDetector;
use opentelemetry_sdk::Resource;
use std::env::consts::ARCH;
#[cfg(target_os = "linux")]
use std::fs::read_to_string;
#[cfg(target_os = "linux")]
use std::path::Path;
#[cfg(target_os = "macos")]
use std::process::Command;
use std::time::Duration;

/// Detect host information.
///
/// This resource detector returns the following information:
///
/// - [`host.id from non-containerized systems`]: https://opentelemetry.io/docs/specs/semconv/resource/host/#collecting-hostid-from-non-containerized-systems
/// - Host architecture (host.arch).
pub struct HostResourceDetector {
    host_id_detect: fn() -> Option<String>,
}

impl ResourceDetector for HostResourceDetector {
    fn detect(&self, _timeout: Duration) -> Resource {
        Resource::new(
            [
                // Get host.id
                (self.host_id_detect)().map(|host_id| {
                    KeyValue::new(
                        opentelemetry_semantic_conventions::resource::HOST_ID,
                        host_id,
                    )
                }),
                // Get host.arch
                Some(KeyValue::new(
                    opentelemetry_semantic_conventions::resource::HOST_ARCH,
                    ARCH,
                )),
            ]
            .into_iter()
            .flatten(),
        )
    }
}

#[cfg(target_os = "linux")]
fn host_id_detect() -> Option<String> {
    let machine_id_path = Path::new("/etc/machine-id");
    let dbus_machine_id_path = Path::new("/var/lib/dbus/machine-id");
    read_to_string(machine_id_path)
        .or_else(|_| read_to_string(dbus_machine_id_path))
        .ok()
}

#[cfg(target_os = "macos")]
fn host_id_detect() -> Option<String> {
    let output = Command::new("ioreg")
        .arg("-rd1")
        .arg("-c")
        .arg("IOPlatformExpertDevice")
        .output()
        .ok()?
        .stdout;

    let output = String::from_utf8(output).ok()?;
    let line = output
        .lines()
        .find(|line| line.contains("IOPlatformUUID"))?;

    Some(line.split_once('=')?.1.trim().trim_matches('"').to_owned())
}

// TODO: Implement non-linux platforms
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn host_id_detect() -> Option<String> {
    None
}

impl Default for HostResourceDetector {
    fn default() -> Self {
        Self { host_id_detect }
    }
}

#[cfg(test)]
mod tests {
    use super::HostResourceDetector;
    use opentelemetry::{Key, Value};
    use opentelemetry_sdk::resource::ResourceDetector;
    use std::time::Duration;

    #[cfg(target_os = "linux")]
    #[test]
    fn test_host_resource_detector_linux() {
        let resource = HostResourceDetector::default().detect(Duration::from_secs(0));
        assert_eq!(resource.len(), 2);
        assert!(resource
            .get(Key::from_static_str(
                opentelemetry_semantic_conventions::resource::HOST_ID
            ))
            .is_some());
        assert!(resource
            .get(Key::from_static_str(
                opentelemetry_semantic_conventions::resource::HOST_ARCH
            ))
            .is_some())
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_host_resource_detector_macos() {
        let resource = HostResourceDetector::default().detect(Duration::from_secs(0));
        dbg!(&resource);
        assert_eq!(resource.len(), 2);
        assert!(resource
            .get(Key::from_static_str(
                opentelemetry_semantic_conventions::resource::HOST_ID
            ))
            .is_some());
        assert!(resource
            .get(Key::from_static_str(
                opentelemetry_semantic_conventions::resource::HOST_ARCH
            ))
            .is_some())
    }

    #[test]
    fn test_resource_host_arch_value() {
        let resource = HostResourceDetector::default().detect(Duration::from_secs(0));

        assert!(resource
            .get(Key::from_static_str(
                opentelemetry_semantic_conventions::resource::HOST_ARCH
            ))
            .is_some());

        #[cfg(target_arch = "x86_64")]
        assert_eq!(
            resource.get(Key::from_static_str(
                opentelemetry_semantic_conventions::resource::HOST_ARCH
            )),
            Some(Value::from("x86_64"))
        );

        #[cfg(target_arch = "aarch64")]
        assert_eq!(
            resource.get(Key::from_static_str(
                opentelemetry_semantic_conventions::resource::HOST_ARCH
            )),
            Some(Value::from("aarch64"))
        )
    }
}
