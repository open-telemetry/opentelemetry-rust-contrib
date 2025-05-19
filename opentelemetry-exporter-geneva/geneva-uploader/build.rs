use std::env;
use std::path::Path;
use std::process;

fn main() {
    println!("running build.rs");
    let bond_include =
        env::var("BOND_INCLUDE_DIR").unwrap_or_else(|_| "/usr/local/include".to_string());
    let bond_lib = env::var("BOND_LIB_DIR").unwrap_or_else(|_| "/usr/local/lib".to_string());

    // Check for bond header file existence
    let bond_header = Path::new(&bond_include).join("bond/core/bond.h");
    if !bond_header.exists() {
        eprintln!(
            "ERROR: Required Bond header not found at {}. \
             Set BOND_INCLUDE_DIR or install Bond.",
            bond_header.display()
        );
        process::exit(1);
    }

    // Optionally check for the bond library file existence (libbond.a or libbond.so)
    let bond_lib_file = Path::new(&bond_lib).join("libbond.a"); // Or .so/.dylib as needed
    if !bond_lib_file.exists() {
        eprintln!(
            "ERROR: Required Bond library not found at {}. \
             Set BOND_LIB_DIR or install Bond.",
            bond_lib_file.display()
        );
        process::exit(1);
    }

    cc::Build::new()
        .cpp(true)
        .file("src/payload_encoder/ffi/serialize_ffi.cpp")
        .include("src/payload_encoder/ffi/")
        .include(&bond_include)
        .flag("-std=c++14")
        .compile("bond_ffi");

    println!("cargo:rustc-link-search=native={}", bond_lib);
    println!("cargo:rustc-link-lib=bond");
    println!("cargo:rustc-link-lib=boost_system");
}
