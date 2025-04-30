use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    let Ok(output) = Command::new("rustc").arg("-V").output() else {
        return;
    };

    let Ok(stdout) = String::from_utf8(output.stdout) else {
        return;
    };

    // rustc -V: rustc 1.76.0 (07dca489a 2024-02-04)
    // version is 1.76.0
    if let Some(version) = stdout.split_whitespace().nth(1) {
        println!("cargo:rustc-env=RUSTC_VERSION={}", version);
    }
}
