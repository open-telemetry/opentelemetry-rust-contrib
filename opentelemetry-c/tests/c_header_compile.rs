//! Best-effort test that the public C headers and the shipped C example compile with a
//! system C compiler. It runs a syntax-only check (no linking), so it does not depend
//! on the shared library being built first. If no C compiler is available (e.g. a
//! minimal CI image) the test is skipped rather than failed.

use std::path::PathBuf;
use std::process::Command;

fn find_c_compiler() -> Option<String> {
    if let Ok(cc) = std::env::var("CC") {
        if !cc.is_empty() {
            return Some(cc);
        }
    }
    for candidate in ["cc", "clang", "gcc"] {
        // `<cc> --version` succeeds if the compiler exists.
        if Command::new(candidate)
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            return Some(candidate.to_owned());
        }
    }
    None
}

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn c_example_and_headers_compile() {
    let cc = match find_c_compiler() {
        Some(cc) => cc,
        None => {
            eprintln!("skipping: no C compiler found (set CC to enable)");
            return;
        }
    };

    let include_dir = manifest_dir().join("include");
    let example = manifest_dir()
        .join("examples")
        .join("c-basic-traces")
        .join("main.c");
    assert!(
        example.exists(),
        "example source missing: {}",
        example.display()
    );

    let output = Command::new(&cc)
        .arg("-std=c11")
        .arg("-Wall")
        .arg("-Wextra")
        .arg("-Werror")
        .arg("-fsyntax-only")
        .arg("-I")
        .arg(&include_dir)
        .arg(&example)
        .output()
        .expect("failed to invoke C compiler");

    assert!(
        output.status.success(),
        "C example failed to compile with {cc}:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

#[test]
fn umbrella_header_is_self_contained() {
    let cc = match find_c_compiler() {
        Some(cc) => cc,
        None => {
            eprintln!("skipping: no C compiler found (set CC to enable)");
            return;
        }
    };

    let include_dir = manifest_dir().join("include");
    // Compile a tiny translation unit that only includes the umbrella header, to prove
    // it pulls in the full API and is warning-clean on its own.
    let tmp = std::env::temp_dir().join("otel_c_umbrella_check.c");
    std::fs::write(
        &tmp,
        b"#include <opentelemetry_c/api.h>\nint main(void){ return (int)otel_version_minor(); }\n",
    )
    .expect("failed to write temp source");

    let output = Command::new(&cc)
        .arg("-std=c11")
        .arg("-Wall")
        .arg("-Wextra")
        .arg("-Werror")
        .arg("-fsyntax-only")
        .arg("-I")
        .arg(&include_dir)
        .arg(&tmp)
        .output()
        .expect("failed to invoke C compiler");

    let _ = std::fs::remove_file(&tmp);
    assert!(
        output.status.success(),
        "umbrella header failed to compile with {cc}:\nstderr:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );
}
