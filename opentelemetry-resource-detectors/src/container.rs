//! Container resource detector
//!
//! Detects the container ID from the cgroup files under `/proc`

use opentelemetry::KeyValue;
use opentelemetry_sdk::resource::{Resource, ResourceDetector};
#[cfg(target_os = "linux")]
use std::fs::read_to_string;

#[cfg(target_os = "linux")]
const CGROUP_PATH: &str = "/proc/self/cgroup";
#[cfg(target_os = "linux")]
const MOUNTINFO_PATH: &str = "/proc/self/mountinfo";

/// Min and max container ID lengths.
#[cfg(any(target_os = "linux", test))]
const MIN_CONTAINER_ID_LENGTH: usize = 32;
#[cfg(any(target_os = "linux", test))]
const MAX_CONTAINER_ID_LENGTH: usize = 64;

/// Detects `container.id` from `/proc/self/cgroup` (cgroup v1), falling back to
/// `/proc/self/mountinfo` (cgroup v2). Returns an empty [`Resource`] when no ID is found.
pub struct ContainerResourceDetector;

impl ResourceDetector for ContainerResourceDetector {
    fn detect(&self) -> Resource {
        Resource::builder_empty()
            .with_attributes(detect_container_id().map(|container_id| {
                KeyValue::new(
                    opentelemetry_semantic_conventions::attribute::CONTAINER_ID,
                    container_id,
                )
            }))
            .build()
    }
}

#[cfg(target_os = "linux")]
fn detect_container_id() -> Option<String> {
    if let Ok(content) = read_to_string(CGROUP_PATH) {
        if let Some(id) = content.lines().find_map(extract_container_id_from_cgroup) {
            return Some(id.to_string());
        }
    }

    if let Ok(content) = read_to_string(MOUNTINFO_PATH) {
        if let Some(id) = extract_container_id_from_mountinfo(&content) {
            return Some(id.to_string());
        }
    }

    None
}

#[cfg(not(target_os = "linux"))]
fn detect_container_id() -> Option<String> {
    None
}

/// Checks if `candidate` is a hex string between 32 and 64 chars.
#[cfg(any(target_os = "linux", test))]
fn is_valid_container_id(candidate: &str) -> bool {
    (MIN_CONTAINER_ID_LENGTH..=MAX_CONTAINER_ID_LENGTH).contains(&candidate.len())
        && candidate.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Extracts a container ID from a `/proc/self/cgroup` line. The ID is the final path
/// segment, with runtime prefixes and dot-separated suffixes removed.
#[cfg(any(target_os = "linux", test))]
fn extract_container_id_from_cgroup(line: &str) -> Option<&str> {
    let last_segment = line[line.rfind('/')? + 1..].trim();

    let candidate = match last_segment.rfind([':', '-']) {
        Some(index) => &last_segment[index + 1..],
        None => last_segment,
    };

    let candidate = candidate.split('.').next().unwrap_or(candidate);

    is_valid_container_id(candidate).then_some(candidate)
}

/// Extracts a container ID from `/proc/self/mountinfo`
#[cfg(any(target_os = "linux", test))]
fn extract_container_id_from_mountinfo(content: &str) -> Option<&str> {
    content
        .lines()
        .find_map(extract_container_id_from_mountinfo_line)
}

/// Extracts a container ID from a single `/proc/self/mountinfo` line. The ID is the
/// segment after `containers` or `overlay-containers` on the `/etc/hostname` mount point line.
#[cfg(any(target_os = "linux", test))]
fn extract_container_id_from_mountinfo_line(line: &str) -> Option<&str> {
    // Root and mount point precede the " - " separator and are indexed 3 and 4 when split by whitespace.
    let mut fields = line.split(" - ").next()?.split_whitespace();
    let root = fields.nth(3)?;
    let mount_point = fields.next()?;
    if mount_point != "/etc/hostname" {
        return None;
    }

    let mut prev = "";
    for segment in root.split('/') {
        if matches!(prev, "containers" | "overlay-containers") && is_valid_container_id(segment) {
            return Some(segment);
        }
        prev = segment;
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_id_from_cgroup_v1_lines() {
        let cases = [
            // Prefix only.
            (
                "13:name=systemd:/podruntime/docker/kubepods/docker-dc579f8a8319c8cf7d38e1adf263bc08d23",
                Some("dc579f8a8319c8cf7d38e1adf263bc08d23"),
            ),
            // Prefix and suffix.
            (
                "11:devices:/kubepods.slice/crio-dc679f8a8319c8cf7d38e1adf263bc08d23.scope",
                Some("dc679f8a8319c8cf7d38e1adf263bc08d23"),
            ),
            // No prefix, no suffix.
            (
                "1:name=systemd:/pod/d86d75589bf6cc254f3e2cc29debdf85dde404998aa128997a819ff991827356",
                Some("d86d75589bf6cc254f3e2cc29debdf85dde404998aa128997a819ff991827356"),
            ),
            // ID follows the last colon.
            (
                "0::/kubepods/burstable/podABC/cri-containerd:e857a4bf04e0a6b0d3b1b9f3d6e2c1a0f9e8d7c6b5a4938271605f4e3d2c1b0a",
                Some("e857a4bf04e0a6b0d3b1b9f3d6e2c1a0f9e8d7c6b5a4938271605f4e3d2c1b0a"),
            ),
        ];

        for (line, expected) in cases {
            assert_eq!(
                extract_container_id_from_cgroup(line),
                expected,
                "line: {line}"
            );
        }
    }

    #[test]
    fn ignores_non_container_cgroup_lines() {
        // A bare root path, a line without any path, and a non-hex segment.
        assert_eq!(extract_container_id_from_cgroup("0::/"), None);
        assert_eq!(extract_container_id_from_cgroup("2:cpu:nopath"), None);
        assert_eq!(
            extract_container_id_from_cgroup("3:cpuset:/system.slice/sshd.service"),
            None
        );
    }

    #[test]
    fn rejects_out_of_range_hex_segments() {
        // Too short.
        assert_eq!(
            extract_container_id_from_cgroup("5:pids:/user.slice/user-1000.slice/session-2.scope"),
            None
        );
        // Too long.
        let too_long = "a".repeat(MAX_CONTAINER_ID_LENGTH + 1);
        assert_eq!(
            extract_container_id_from_cgroup(&format!("0::/docker/{too_long}")),
            None
        );
    }

    #[test]
    fn extracts_id_from_mountinfo() {
        let content = "\
1573 1471 0:286 / / rw,relatime master:533 - overlay overlay rw
1579 1573 0:290 / /dev rw,nosuid - tmpfs tmpfs rw
1580 1579 0:291 / /dev/pts rw,nosuid,noexec - devpts devpts rw
2304 1573 254:1 /docker/containers/1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef/hostname /etc/hostname rw,relatime - ext4 /dev/vda1 rw
";
        assert_eq!(
            extract_container_id_from_mountinfo(content),
            Some("1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef")
        );
    }

    #[test]
    fn extracts_id_from_mountinfo_overlay_containers() {
        let content = "\
2304 1573 254:1 /containers/storage/overlay-containers/abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890/userdata/hostname /etc/hostname rw - ext4 /dev/vda1 rw
";
        assert_eq!(
            extract_container_id_from_mountinfo(content),
            Some("abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890")
        );
    }

    #[test]
    fn returns_none_when_mountinfo_has_no_container_id() {
        let content = "\
1573 1471 0:286 / / rw,relatime - overlay overlay rw
1579 1573 0:290 / /dev rw,nosuid - tmpfs tmpfs rw
";
        assert_eq!(extract_container_id_from_mountinfo(content), None);
    }

    #[test]
    fn extracts_id_from_later_mountinfo_line() {
        let content = "\
1234 1111 0:50 /etc/hostname /etc/hostname rw,relatime - tmpfs tmpfs rw
2304 1573 254:1 /docker/containers/1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef/hostname /etc/hostname rw - ext4 /dev/vda1 rw
";
        assert_eq!(
            extract_container_id_from_mountinfo(content),
            Some("1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef")
        );
    }

    #[test]
    fn extracts_shorter_hex_id_from_mountinfo() {
        // Valid hex ID shorter than 64 chars.
        let content = "\
2304 1573 254:1 /docker/containers/abcdef1234567890abcdef1234567890/hostname /etc/hostname rw - ext4 /dev/vda1 rw
";
        assert_eq!(
            extract_container_id_from_mountinfo(content),
            Some("abcdef1234567890abcdef1234567890")
        );
    }

    #[test]
    fn ignores_non_hex_mountinfo_segment() {
        let content = "\
2304 1573 254:1 /docker/containers/zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz/hostname /etc/hostname rw - ext4 /dev/vda1 rw
";
        assert_eq!(extract_container_id_from_mountinfo(content), None);
    }

    #[test]
    fn detect_does_not_panic() {
        // On the host running the tests there may or may not be a container ID;
        // either way detection must not panic.
        let _ = ContainerResourceDetector.detect();
    }
}
