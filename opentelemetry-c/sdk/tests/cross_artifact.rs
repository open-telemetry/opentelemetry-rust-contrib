//! Cross-artifact proof: the API and SDK are **separate** dynamic libraries that share the
//! API-owned global provider slot.
//!
//! This test compiles a small C program, links it against BOTH `libopentelemetry_c_api` and
//! `libopentelemetry_c_sdk`, and runs it. The program installs the SDK and then emits spans
//! using ONLY the API's global-provider path (as an instrumentation library would). A
//! self-contained mock OTLP/HTTP collector (a plain `TcpListener`) confirms the spans were
//! exported through the SDK — proving the SDK registered into the API-owned global slot and
//! that API-only calls dispatch to it across the artifact boundary.
//!
//! The test **self-skips** when a C compiler is unavailable or the cdylibs have not been
//! built yet (run `cargo build -p opentelemetry-c-api -p opentelemetry-c-sdk` first).
//! Self-skipping is a **local developer convenience only**: when `CI` is set the test
//! instead **fails hard** if either prerequisite is missing, so the cross-artifact proof
//! can never silently no-op in CI.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

fn find_cc() -> Option<String> {
    if let Ok(cc) = std::env::var("CC") {
        if !cc.is_empty() {
            return Some(cc);
        }
    }
    for candidate in ["cc", "clang", "gcc"] {
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

fn dylib_names(stem: &str) -> [String; 3] {
    [
        format!("lib{stem}.dylib"),
        format!("lib{stem}.so"),
        format!("{stem}.dll"),
    ]
}

/// Whether we are running under CI. When true, this test must **fail** rather than
/// self-skip if its prerequisites (C compiler, built cdylibs) are missing — otherwise the
/// cross-artifact global-provider proof could silently never run in CI.
fn is_ci() -> bool {
    std::env::var("CI")
        .map(|v| !v.is_empty() && v != "0" && !v.eq_ignore_ascii_case("false"))
        .unwrap_or(false)
}

/// Find a target profile dir that contains BOTH cdylibs.
fn find_lib_dir() -> Option<PathBuf> {
    // This crate lives at `<workspace>/opentelemetry-c/sdk`, so the workspace root is two
    // parents up: opentelemetry-c/sdk -> opentelemetry-c -> <workspace>.
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    // Honor CARGO_TARGET_DIR: an absolute value is used as-is; a relative value is resolved
    // against the workspace root (NOT the SDK crate dir). Otherwise default to <root>/target.
    let target_dir = match std::env::var_os("CARGO_TARGET_DIR") {
        Some(dir) if !dir.is_empty() => {
            let dir = PathBuf::from(dir);
            if dir.is_absolute() {
                dir
            } else {
                workspace_root.join(dir)
            }
        }
        _ => workspace_root.join("target"),
    };
    for profile in ["release", "debug"] {
        let dir = target_dir.join(profile);
        let has = |stem: &str| dylib_names(stem).iter().any(|n| dir.join(n).exists());
        if has("opentelemetry_c_api") && has("opentelemetry_c_sdk") {
            return Some(dir);
        }
    }
    None
}

const HARNESS_C: &str = r#"
#include <stdint.h>
#include <string.h>
#include <stddef.h>
typedef struct { const char* ptr; size_t len; } otel_string_view_t;
typedef struct otel_sdk_builder_t otel_sdk_builder_t;
typedef struct otel_sdk_t otel_sdk_t;
typedef struct otel_tracer_provider_t otel_tracer_provider_t;
typedef struct otel_tracer_t otel_tracer_t;
typedef struct otel_span_t otel_span_t;
typedef struct { uint32_t kind; const otel_span_t* parent; } otel_span_start_options_t;
extern otel_tracer_provider_t* otel_global_tracer_provider(void);
extern otel_tracer_t* otel_tracer_provider_get_tracer(const otel_tracer_provider_t*, otel_string_view_t, otel_string_view_t, otel_string_view_t);
extern otel_span_t* otel_tracer_start_span(const otel_tracer_t*, otel_string_view_t, const otel_span_start_options_t*);
extern int otel_span_set_string_attribute(otel_span_t*, otel_string_view_t, otel_string_view_t);
extern int otel_span_end(otel_span_t*);
extern void otel_span_destroy(otel_span_t*);
extern void otel_tracer_destroy(otel_tracer_t*);
extern void otel_tracer_provider_destroy(otel_tracer_provider_t*);
extern otel_sdk_builder_t* otel_sdk_builder_new(void);
extern int otel_sdk_builder_set_service_name(otel_sdk_builder_t*, otel_string_view_t);
extern int otel_sdk_builder_set_otlp_endpoint(otel_sdk_builder_t*, otel_string_view_t);
extern int otel_sdk_builder_set_otlp_timeout_millis(otel_sdk_builder_t*, uint64_t);
extern int otel_sdk_build(const otel_sdk_builder_t*, otel_sdk_t**);
extern void otel_sdk_builder_destroy(otel_sdk_builder_t*);
extern int otel_sdk_set_as_global(otel_sdk_t*);
extern int otel_sdk_force_flush(otel_sdk_t*, uint64_t);
extern int otel_sdk_shutdown(otel_sdk_t*, uint64_t);
extern void otel_sdk_destroy(otel_sdk_t*);
static otel_string_view_t cs(const char* s){ otel_string_view_t v; v.ptr=s; v.len=s?strlen(s):0; return v; }
static otel_string_view_t emp(void){ otel_string_view_t v; v.ptr=(void*)0; v.len=0; return v; }
extern char* getenv(const char*);
static void work(void){
    otel_tracer_provider_t* p = otel_global_tracer_provider();
    otel_tracer_t* t = otel_tracer_provider_get_tracer(p, cs("instr"), cs("1.0"), emp());
    otel_span_t* parent = otel_tracer_start_span(t, cs("parent"), (void*)0);
    otel_span_set_string_attribute(parent, cs("k"), cs("v"));
    otel_span_start_options_t o; o.kind=2; o.parent=parent;
    otel_span_t* child = otel_tracer_start_span(t, cs("child"), &o);
    otel_span_end(child); otel_span_destroy(child);
    otel_span_end(parent); otel_span_destroy(parent);
    otel_tracer_destroy(t); otel_tracer_provider_destroy(p);
}
int main(void){
    work(); /* API-only no-op before install (must be safe) */
    otel_sdk_builder_t* b = otel_sdk_builder_new();
    otel_sdk_builder_set_service_name(b, cs("cross-artifact"));
    otel_sdk_builder_set_otlp_endpoint(b, cs(getenv("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT")));
    otel_sdk_builder_set_otlp_timeout_millis(b, 5000);
    otel_sdk_t* sdk=(void*)0;
    if (otel_sdk_build(b,&sdk)!=0||!sdk) return 2;
    otel_sdk_builder_destroy(b);
    if (otel_sdk_set_as_global(sdk)!=0) return 3;
    work(); /* API-only calls AFTER install must export through the SDK */
    otel_sdk_force_flush(sdk, 5000);
    otel_sdk_shutdown(sdk, 5000);
    otel_sdk_destroy(sdk);
    return 0;
}
"#;

/// Minimal mock OTLP/HTTP collector: accepts POSTs and accumulates total body bytes.
fn start_mock() -> (u16, Arc<AtomicUsize>, Arc<AtomicBool>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock");
    let port = listener.local_addr().unwrap().port();
    listener.set_nonblocking(true).unwrap();
    let bytes = Arc::new(AtomicUsize::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    let (b2, s2) = (Arc::clone(&bytes), Arc::clone(&stop));
    std::thread::spawn(move || {
        while !s2.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((mut sock, _)) => {
                    sock.set_read_timeout(Some(Duration::from_secs(2))).ok();
                    let mut buf = Vec::new();
                    let mut tmp = [0u8; 4096];
                    // Read headers to find Content-Length, then the body.
                    let mut content_len = 0usize;
                    let mut header_end = None;
                    loop {
                        match sock.read(&mut tmp) {
                            Ok(0) => break,
                            Ok(n) => {
                                buf.extend_from_slice(&tmp[..n]);
                                if header_end.is_none() {
                                    if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n")
                                    {
                                        header_end = Some(pos + 4);
                                        let headers =
                                            String::from_utf8_lossy(&buf[..pos]).to_lowercase();
                                        for line in headers.lines() {
                                            if let Some(v) = line.strip_prefix("content-length:") {
                                                content_len = v.trim().parse().unwrap_or(0);
                                            }
                                        }
                                    }
                                }
                                if let Some(he) = header_end {
                                    if buf.len() >= he + content_len {
                                        break;
                                    }
                                }
                            }
                            Err(_) => break,
                        }
                    }
                    if let Some(he) = header_end {
                        b2.fetch_add(buf.len().saturating_sub(he), Ordering::Relaxed);
                    }
                    let _ = sock.write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Type: application/x-protobuf\r\nContent-Length: 0\r\n\r\n",
                    );
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(_) => break,
            }
        }
    });
    (port, bytes, stop)
}

#[test]
fn api_only_calls_after_sdk_install_export_through_sdk() {
    // This proof relies on Unix dynamic-linking semantics (rpath plus DYLD_LIBRARY_PATH /
    // LD_LIBRARY_PATH). Windows dynamic linking of the split is not a supported/claimed
    // model, so skip cleanly on non-Unix targets — even under CI — rather than fail
    // confusingly. Unix CI fail-hard behavior (missing cc / cdylibs) is unchanged.
    if !cfg!(unix) {
        eprintln!(
            "skipping: the cross-artifact proof requires Unix dynamic linking (non-Unix target)"
        );
        return;
    }
    let cc = match find_cc() {
        Some(cc) => cc,
        None => {
            if is_ci() {
                panic!(
                    "CI=true but no C compiler was found: the cross-artifact global-provider \
                     proof cannot run. Install a C compiler or set the CC environment variable."
                );
            }
            eprintln!("skipping: no C compiler (set CC to enable)");
            return;
        }
    };
    let lib_dir = match find_lib_dir() {
        Some(d) => d,
        None => {
            if is_ci() {
                panic!(
                    "CI=true but the cdylibs are not built: the cross-artifact global-provider \
                     proof cannot run. Build them first with: \
                     `cargo build -p opentelemetry-c-api -p opentelemetry-c-sdk`."
                );
            }
            eprintln!(
                "skipping: cdylibs not built. Run: cargo build -p opentelemetry-c-api -p opentelemetry-c-sdk"
            );
            return;
        }
    };

    let out = std::env::temp_dir().join("otel_c_cross_artifact");
    let src = out.with_extension("c");
    std::fs::write(&src, HARNESS_C).expect("write harness");

    let mut cmd = Command::new(&cc);
    cmd.arg("-std=c11")
        .arg(&src)
        .arg("-L")
        .arg(&lib_dir)
        .arg("-lopentelemetry_c_api")
        .arg("-lopentelemetry_c_sdk")
        .arg(format!("-Wl,-rpath,{}", lib_dir.display()))
        .arg("-o")
        .arg(&out);
    let compile = cmd.output().expect("invoke cc");
    assert!(
        compile.status.success(),
        "harness failed to compile/link:\n{}",
        String::from_utf8_lossy(&compile.stderr)
    );

    let (port, bytes, stop) = start_mock();
    let endpoint = format!("http://127.0.0.1:{port}/v1/traces");
    let run = Command::new(&out)
        .env("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT", &endpoint)
        .env("DYLD_LIBRARY_PATH", &lib_dir)
        .env("LD_LIBRARY_PATH", &lib_dir)
        .output()
        .expect("run harness");
    // Give the collector a moment to finish reading the final POST.
    std::thread::sleep(Duration::from_millis(300));
    stop.store(true, Ordering::Relaxed);

    let _ = std::fs::remove_file(&src);
    let _ = std::fs::remove_file(&out);

    assert!(
        run.status.success(),
        "harness exited with failure ({:?}):\nstdout: {}\nstderr: {}",
        run.status.code(),
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr),
    );
    let received = bytes.load(Ordering::Relaxed);
    assert!(
        received > 0,
        "the mock collector received no exported span bytes — API-only calls after SDK \
         install did NOT reach the SDK across the artifact boundary"
    );
    eprintln!("cross-artifact export OK: {received} protobuf bytes via API-only path");
}
