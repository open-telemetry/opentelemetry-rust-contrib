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
    // The API crate is a sibling under `opentelemetry-c/`: sdk -> opentelemetry-c -> api.
    manifest().parent().unwrap().join("api/include")
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

    // A TU that includes only sdk.h (which pulls in the API's common.h/trace.h), and also
    // exercises the optional header-only helpers to confirm they are reachable through the
    // SDK header context. `-fsyntax-only` does not link, so a NULL span is fine.
    let tmp = std::env::temp_dir().join("otel_c_sdk_hdr_check.c");
    std::fs::write(
        &tmp,
        r#"#include <opentelemetry_c/sdk.h>
int main(void) {
    otel_sdk_builder_t* b = otel_sdk_builder_new();
    (void)b;
    otel_span_t* span = (void*)0;
    (void)otel_span_set_attribute(span, otel_kv_double(otel_cstr("d"), 2.5));
    (void)otel_span_set_ok(span);
    return 0;
}
"#,
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

    // A TU that includes every pipeline header and exercises the full builder chain, so each
    // new header compiles standalone and the ownership-transfer signatures line up.
    let pipeline = std::env::temp_dir().join("otel_c_pipeline_hdr_check.c");
    std::fs::write(
        &pipeline,
        r#"#include <opentelemetry_c/sdk.h>
#include <opentelemetry_c/trace_exporter.h>
#include <opentelemetry_c/span_processor.h>
#include <opentelemetry_c/otlp_trace_exporter.h>
#include <opentelemetry_c/batch_span_processor.h>
int main(void) {
    otel_otlp_trace_exporter_builder_t* eb = otel_otlp_trace_exporter_builder_new();
    otel_otlp_trace_exporter_builder_set_endpoint(eb, otel_cstr("http://localhost:4318/v1/traces"));
    otel_otlp_trace_exporter_builder_set_timeout_millis(eb, 5000);
    otel_trace_exporter_t* exporter = (void*)0;
    otel_otlp_trace_exporter_builder_build(eb, &exporter);
    otel_otlp_trace_exporter_builder_destroy(eb);

    otel_batch_span_processor_builder_t* pb = otel_batch_span_processor_builder_new();
    otel_batch_span_processor_builder_set_exporter(pb, exporter);
    otel_batch_span_processor_builder_set_max_queue_size(pb, 2048);
    otel_span_processor_t* processor = (void*)0;
    otel_batch_span_processor_builder_build(pb, &processor);
    otel_batch_span_processor_builder_destroy(pb);

    otel_sdk_builder_t* sb = otel_sdk_builder_new();
    otel_sdk_builder_set_service_name(sb, otel_cstr("hdr-check"));
    otel_sdk_builder_add_span_processor(sb, processor);
    otel_sdk_t* sdk = (void*)0;
    otel_sdk_build(sb, &sdk);
    otel_sdk_builder_destroy(sb);
    (void)sdk;
    return 0;
}
"#,
    )
    .expect("write pipeline source");
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
            pipeline.as_os_str(),
        ],
    );
    let _ = std::fs::remove_file(&pipeline);

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
