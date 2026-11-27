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
                        arch_to_semconv(ARCH),
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

/// Map a [`std::env::consts::ARCH`] value to the corresponding well-known
/// [`host.arch` value](https://opentelemetry.io/docs/specs/semconv/resource/host/#host-arch-values)
/// from the semantic conventions.
///
/// The semantic conventions require that one of the well-known values MUST be
/// used when it applies; for any other architecture a custom value MAY be
/// used, so unknown values are passed through unchanged.
///
/// Well-known values without a Rust target (e.g. `ia64`) need no mapping, as
/// their raw value already matches the semantic convention.
fn arch_to_semconv(arch: &str) -> &str {
    match arch {
        // 64-bit x86
        "x86_64" => "amd64",
        // 32-bit x86 (raw value already matches the semantic convention)
        "x86" => "x86",
        // 64-bit ARM
        "aarch64" => "arm64",
        // 32-bit ARM
        "arm" => "arm32",
        // 32-bit PowerPC
        "powerpc" => "ppc32",
        // 64-bit PowerPC
        "powerpc64" => "ppc64",
        // IBM z/Architecture (raw value already matches the semantic convention)
        "s390x" => "s390x",
        // No well-known value: pass the raw value through as a custom value
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
    use super::{arch_to_semconv, HostResourceDetector};
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
    fn test_arch_to_semconv_maps_to_well_known_values() {
        // std::env::consts::ARCH values with a well-known host.arch equivalent
        assert_eq!(arch_to_semconv("x86_64"), "amd64");
        assert_eq!(arch_to_semconv("x86"), "x86");
        assert_eq!(arch_to_semconv("aarch64"), "arm64");
        assert_eq!(arch_to_semconv("arm"), "arm32");
        assert_eq!(arch_to_semconv("powerpc"), "ppc32");
        assert_eq!(arch_to_semconv("powerpc64"), "ppc64");
        assert_eq!(arch_to_semconv("s390x"), "s390x");
    }

    #[test]
    fn test_arch_to_semconv_passes_unknown_values_through() {
        // Architectures without a well-known value are passed through as
        // custom values, as allowed by the semantic conventions.
        assert_eq!(arch_to_semconv("riscv64"), "riscv64");
        assert_eq!(arch_to_semconv("loongarch64"), "loongarch64");
        assert_eq!(arch_to_semconv("mips64"), "mips64");
        assert_eq!(arch_to_semconv("wasm32"), "wasm32");
    }

    #[test]
    fn test_resource_host_arch_value() {
        let resource = HostResourceDetector::default().detect();

        let host_arch = resource
            .get(&Key::from_static_str(
                opentelemetry_semantic_conventions::attribute::HOST_ARCH,
            ))
            .expect("host.arch should always be present");

        // host.arch MUST use the well-known values from the semantic
        // conventions, not the raw Rust target arch strings.
        #[cfg(target_arch = "x86_64")]
        assert_eq!(host_arch, Value::from("amd64"));

        #[cfg(target_arch = "aarch64")]
        assert_eq!(host_arch, Value::from("arm64"));

        #[cfg(target_arch = "x86")]
        assert_eq!(host_arch, Value::from("x86"));

        #[cfg(target_arch = "arm")]
        assert_eq!(host_arch, Value::from("arm32"));

        #[cfg(target_arch = "powerpc")]
        assert_eq!(host_arch, Value::from("ppc32"));

        #[cfg(target_arch = "powerpc64")]
        assert_eq!(host_arch, Value::from("ppc64"));

        #[cfg(target_arch = "s390x")]
        assert_eq!(host_arch, Value::from("s390x"));
    }
}
