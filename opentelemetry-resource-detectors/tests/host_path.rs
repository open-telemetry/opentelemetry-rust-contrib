//! Run detection in a child process so changing PATH cannot affect other tests.
#![cfg(unix)]

use opentelemetry::Key;
use opentelemetry_resource_detectors::HostResourceDetector;
use opentelemetry_sdk::resource::ResourceDetector;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

#[test]
fn host_detection_ignores_path() {
    const CHILD: &str = "OTEL_HOST_PATH_TEST_CHILD";
    const MARKER: &str = "OTEL_HOST_PATH_TEST_MARKER";
    const EXPECTED_NAME: &str = "OTEL_HOST_PATH_TEST_NAME";
    const EXPECTED_ID: &str = "OTEL_HOST_PATH_TEST_ID";

    if std::env::var_os(CHILD).is_some() {
        let resource = HostResourceDetector::default().detect();
        for (attribute, expected) in [("host.name", EXPECTED_NAME), ("host.id", EXPECTED_ID)] {
            assert_eq!(
                resource
                    .get(&Key::from_static_str(attribute))
                    .map(|v| v.to_string()),
                std::env::var(expected).ok(),
                "{attribute} changed with an untrusted PATH"
            );
        }
        assert!(!std::path::Path::new(&std::env::var_os(MARKER).unwrap()).exists());
        return;
    }

    let baseline = HostResourceDetector::default().detect();
    assert!(baseline.get(&Key::from_static_str("host.name")).is_some());
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("executed");
    for program in ["hostname", "ioreg"] {
        let path = directory.path().join(program);
        std::fs::write(
            &path,
            "#!/bin/sh\nprintf executed > \"$OTEL_HOST_PATH_TEST_MARKER\"\nprintf 'attacker-controlled-host-name\\n'\n",
        )
        .unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    let mut child = Command::new(std::env::current_exe().unwrap());
    child
        .args(["--exact", "host_detection_ignores_path", "--nocapture"])
        .env(CHILD, "1")
        .env(MARKER, &marker)
        .env("PATH", directory.path());
    for (attribute, expected) in [("host.name", EXPECTED_NAME), ("host.id", EXPECTED_ID)] {
        child.env_remove(expected);
        if let Some(value) = baseline.get(&Key::from_static_str(attribute)) {
            child.env(expected, value.to_string());
        }
    }
    let output = child.output().unwrap();
    assert!(
        output.status.success(),
        "child failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!marker.exists(), "executed a program from PATH");
}
