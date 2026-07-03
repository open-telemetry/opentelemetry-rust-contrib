//! Best-effort check that the API library's public C headers compile with a system C
//! compiler (syntax-only, no linking). Self-skips if no compiler is available.

use std::path::PathBuf;
use std::process::Command;

fn find_cc() -> Option<String> {
    if let Ok(cc) = std::env::var("CC") {
        if !cc.is_empty() {
            return Some(cc);
        }
    }
    for c in ["cc", "clang", "gcc"] {
        if Command::new(c)
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            return Some(c.to_owned());
        }
    }
    None
}

fn include_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("include")
}

fn syntax_check(cc: &str, include: &PathBuf, src: &str) {
    let tmp = std::env::temp_dir().join("otel_c_api_hdr_check.c");
    std::fs::write(&tmp, src).expect("write temp source");
    let out = Command::new(cc)
        .args([
            "-std=c11",
            "-Wall",
            "-Wextra",
            "-Werror",
            "-fsyntax-only",
            "-I",
        ])
        .arg(include)
        .arg(&tmp)
        .output()
        .expect("invoke cc");
    let _ = std::fs::remove_file(&tmp);
    assert!(
        out.status.success(),
        "API header failed to compile:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn api_umbrella_header_compiles() {
    let cc = match find_cc() {
        Some(cc) => cc,
        None => {
            eprintln!("skipping: no C compiler found");
            return;
        }
    };
    syntax_check(
        &cc,
        &include_dir(),
        "#include <opentelemetry_c/api.h>\nint main(void){ return (int)otel_version_minor(); }\n",
    );
    // Individual headers too.
    syntax_check(
        &cc,
        &include_dir(),
        "#include <opentelemetry_c/common.h>\n#include <opentelemetry_c/trace.h>\nint main(void){return 0;}\n",
    );
}
