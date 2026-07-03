//! Best-effort check that the SDK header (and the split example) compile with a system C
//! compiler (syntax-only). `sdk.h` includes the API's `common.h`/`trace.h`, so the API's
//! include directory is also on the search path — mirroring how an application compiles.
//! Self-skips if no compiler is available.

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

fn manifest() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn api_include() -> PathBuf {
    manifest()
        .parent()
        .unwrap()
        .join("opentelemetry-c-api/include")
}

fn sdk_include() -> PathBuf {
    manifest().join("include")
}

fn syntax_check(cc: &str, args: &[&std::ffi::OsStr]) {
    let out = Command::new(cc).args(args).output().expect("invoke cc");
    assert!(
        out.status.success(),
        "compile failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn sdk_header_and_example_compile() {
    let cc = match find_cc() {
        Some(cc) => cc,
        None => {
            eprintln!("skipping: no C compiler found");
            return;
        }
    };
    let api_inc = api_include();
    let sdk_inc = sdk_include();

    // A TU that includes only sdk.h (which pulls in the API's common.h/trace.h).
    let tmp = std::env::temp_dir().join("otel_c_sdk_hdr_check.c");
    std::fs::write(
        &tmp,
        "#include <opentelemetry_c/sdk.h>\nint main(void){ otel_sdk_builder_t* b = otel_sdk_builder_new(); (void)b; return 0; }\n",
    )
    .expect("write temp source");
    syntax_check(
        &cc,
        &[
            "-std=c11".as_ref(),
            "-Wall".as_ref(),
            "-Wextra".as_ref(),
            "-Werror".as_ref(),
            "-fsyntax-only".as_ref(),
            "-I".as_ref(),
            api_inc.as_os_str(),
            "-I".as_ref(),
            sdk_inc.as_os_str(),
            tmp.as_os_str(),
        ],
    );
    let _ = std::fs::remove_file(&tmp);

    // The shipped split example (includes api.h + sdk.h).
    let example = manifest().join("examples/c-basic-traces/main.c");
    assert!(example.exists(), "example missing: {}", example.display());
    syntax_check(
        &cc,
        &[
            "-std=c11".as_ref(),
            "-Wall".as_ref(),
            "-Wextra".as_ref(),
            "-Werror".as_ref(),
            "-fsyntax-only".as_ref(),
            "-I".as_ref(),
            api_inc.as_os_str(),
            "-I".as_ref(),
            sdk_inc.as_os_str(),
            example.as_os_str(),
        ],
    );
}
