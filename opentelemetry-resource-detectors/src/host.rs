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

/// Detect host information.
///
/// This resource detector returns the following information:
///
/// - [`host.id from non-containerized systems`](https://opentelemetry.io/docs/specs/semconv/resource/host/#collecting-hostid-from-non-containerized-systems)
/// - Host architecture (host.arch).
/// - Host name (host.name).
pub struct HostResourceDetector {
    host_id_detect: fn() -> Option<String>,
    host_name_detect: fn() -> Option<String>,
}

impl ResourceDetector for HostResourceDetector {
    fn detect(&self) -> Resource {
        Resource::builder_empty()
            .with_attributes(
                [
                    // Get host.id
                    (self.host_id_detect)().map(|host_id| {
                        KeyValue::new(
                            opentelemetry_semantic_conventions::attribute::HOST_ID,
                            host_id,
                        )
                    }),
                    // Get host.arch
                    Some(KeyValue::new(
                        opentelemetry_semantic_conventions::attribute::HOST_ARCH,
                        host_arch(ARCH),
                    )),
                    // Get host.name
                    (self.host_name_detect)().map(|host_name| {
                        KeyValue::new(
                            opentelemetry_semantic_conventions::attribute::HOST_NAME,
                            host_name,
                        )
                    }),
                ]
                .into_iter()
                .flatten(),
            )
            .build()
    }
}

/// Map a Rust `target_arch` name to the `host.arch` value defined by the
/// semantic conventions. Architectures without a defined value are reported
/// as Rust names them.
fn host_arch(arch: &'static str) -> &'static str {
    match arch {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        "arm" => "arm32",
        "x86" => "x86",
        "powerpc" => "ppc32",
        "powerpc64" => "ppc64",
        "s390x" => "s390x",
        other => other,
    }
}

#[cfg(target_os = "linux")]
fn host_id_detect() -> Option<String> {
    let machine_id_path = Path::new("/etc/machine-id");
    let dbus_machine_id_path = Path::new("/var/lib/dbus/machine-id");
    read_to_string(machine_id_path)
        .or_else(|_| read_to_string(dbus_machine_id_path))
        .map(|id| id.trim().to_string())
        .ok()
}

#[cfg(target_os = "macos")]
fn host_id_detect() -> Option<String> {
    let output = Command::new("/usr/sbin/ioreg")
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

#[cfg(any(unix, windows, target_os = "redox"))]
fn host_name_detect() -> Option<String> {
    let name = hostname::get().ok()?.into_string().ok()?;
    let name = name.trim().to_owned();
    (!name.is_empty()).then_some(name)
}

#[cfg(not(any(unix, windows, target_os = "redox")))]
fn host_name_detect() -> Option<String> {
    None
}

impl Default for HostResourceDetector {
    fn default() -> Self {
        Self {
            host_id_detect,
            host_name_detect,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{host_arch, HostResourceDetector};
    use opentelemetry::{Key, Value};
    use opentelemetry_sdk::resource::ResourceDetector;

    #[cfg(target_os = "linux")]
    #[test]
    fn test_host_resource_detector_linux() {
        let resource = HostResourceDetector::default().detect();
        // host.id + host.arch are always present; host.name is best-effort
        assert!(matches!(resource.len(), 2 | 3));
        assert!(resource
            .get(&Key::from_static_str(
                opentelemetry_semantic_conventions::attribute::HOST_ID
            ))
            .is_some());
        assert!(resource
            .get(&Key::from_static_str(
                opentelemetry_semantic_conventions::attribute::HOST_ARCH
            ))
            .is_some())
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_host_resource_detector_macos() {
        let resource = HostResourceDetector::default().detect();
        // host.id + host.arch are always present; host.name is best-effort
        assert!(matches!(resource.len(), 2 | 3));
        assert!(resource
            .get(&Key::from_static_str(
                opentelemetry_semantic_conventions::attribute::HOST_ID
            ))
            .is_some());
        assert!(resource
            .get(&Key::from_static_str(
                opentelemetry_semantic_conventions::attribute::HOST_ARCH
            ))
            .is_some())
    }

    #[test]
    fn test_host_name_detected() {
        let detector = HostResourceDetector {
            host_id_detect: || None,
            host_name_detect: || Some("opentelemetry-test".to_string()),
        };
        let resource = detector.detect();
        assert_eq!(
            resource.get(&Key::from_static_str(
                opentelemetry_semantic_conventions::attribute::HOST_NAME
            )),
            Some(Value::from("opentelemetry-test"))
        )
    }

    #[test]
    fn test_host_name_absent_when_none() {
        let detector = HostResourceDetector {
            host_id_detect: || None,
            host_name_detect: || None,
        };
        assert!(detector
            .detect()
            .get(&Key::from_static_str(
                opentelemetry_semantic_conventions::attribute::HOST_NAME
            ))
            .is_none())
    }

    #[test]
    fn test_resource_host_arch_value() {
        let resource = HostResourceDetector::default().detect();

        assert!(resource
            .get(&Key::from_static_str(
                opentelemetry_semantic_conventions::attribute::HOST_ARCH
            ))
            .is_some());

        #[cfg(target_arch = "x86_64")]
        assert_eq!(
            resource.get(&Key::from_static_str(
                opentelemetry_semantic_conventions::attribute::HOST_ARCH
            )),
            Some(Value::from("amd64"))
        );

        #[cfg(target_arch = "aarch64")]
        assert_eq!(
            resource.get(&Key::from_static_str(
                opentelemetry_semantic_conventions::attribute::HOST_ARCH
            )),
            Some(Value::from("arm64"))
        )
    }

    #[test]
    fn test_host_arch_uses_semconv_values() {
        assert_eq!(host_arch("x86_64"), "amd64");
        assert_eq!(host_arch("aarch64"), "arm64");
        assert_eq!(host_arch("arm"), "arm32");
        assert_eq!(host_arch("x86"), "x86");
        assert_eq!(host_arch("powerpc"), "ppc32");
        assert_eq!(host_arch("powerpc64"), "ppc64");
        assert_eq!(host_arch("s390x"), "s390x");
        // No semconv value defined: keep the Rust name.
        assert_eq!(host_arch("riscv64"), "riscv64");
    }
}
