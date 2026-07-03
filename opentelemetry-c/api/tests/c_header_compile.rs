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

/// A unique temp `.c` path per invocation, so parallel test threads/processes never clobber
/// or delete each other's source file. std-only: process id + `SystemTime` nanos + a
/// monotonic per-process counter.
fn unique_temp_c(label: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "otel_c_{label}_hdr_check_{}_{}_{}.c",
        std::process::id(),
        nanos,
        seq
    ))
}

fn syntax_check(cc: &str, include: &PathBuf, src: &str) {
    let tmp = unique_temp_c("api");
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

#[test]
fn api_convenience_helpers_compile() {
    let cc = match find_cc() {
        Some(cc) => cc,
        None => {
            eprintln!("skipping: no C compiler found");
            return;
        }
    };
    // Exercise every optional header-only helper: the typed key/value constructors
    // (common.h) and the span-status shorthands (trace.h), including building an attribute
    // array for otel_span_add_event(). `-fsyntax-only` does not link, so a NULL span is fine.
    syntax_check(
        &cc,
        &include_dir(),
        r#"#include <opentelemetry_c/api.h>
int main(void) {
    otel_key_value_t attrs[] = {
        otel_kv_string(otel_cstr("str"), otel_cstr("v")),
        otel_kv_bool(otel_cstr("flag"), OTEL_TRUE),
        otel_kv_int64(otel_cstr("count"), 42),
        otel_kv_double(otel_cstr("ratio"), 1.5)
    };
    otel_span_t* span = (void*)0;
    (void)otel_span_add_event(span, otel_cstr("event"), attrs, sizeof(attrs) / sizeof(attrs[0]));
    (void)otel_span_set_attribute(span, otel_kv_int64(otel_cstr("x"), 1));
    (void)otel_span_set_ok(span);
    (void)otel_span_set_error(span, otel_cstr("boom"));
    return 0;
}
"#,
    );
}
