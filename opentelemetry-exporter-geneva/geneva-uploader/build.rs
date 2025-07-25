//! Build script for Geneva Uploader with optional MSI authentication support

fn main() {
    println!("cargo:warning=---> Running build script for Geneva Uploader");
    // Always output cargo rerun instructions for environment changes
    println!("cargo:rerun-if-env-changed=MSINATIVE_LIB_PATH");
    println!("cargo:rerun-if-changed=build.rs");
    
    // Tell cargo about our custom cfg
    println!("cargo:rustc-check-cfg=cfg(msi_native_available)");
    
    // Only run MSI build logic if the msi_auth feature is enabled
    #[cfg(feature = "msi_auth")]
    {
        println!("cargo:warning=---> Checking for MSI native library support");
        let msi_available = check_msi_library();
        if msi_available {
            println!("cargo:rustc-cfg=msi_native_available");
            eprintln!("INFO: MSI native authentication support enabled");
        } else {
            eprintln!("INFO: MSI native authentication support disabled - using stub implementation");
        }
    }
}

#[cfg(feature = "msi_auth")]
fn check_msi_library() -> bool {
    use std::env;
    use std::path::Path;
    
    // Check if MSINATIVE_LIB_PATH is provided
    match env::var("MSINATIVE_LIB_PATH") {
        Ok(msinative_lib_path) => {
            println!("cargo:warning=---> MSINATIVE_LIB_PATH is set to: {}", msinative_lib_path);
            println!("cargo:rerun-if-changed={}", msinative_lib_path);
            
            // Check if the path points to a valid static library file
            let lib_path = Path::new(&msinative_lib_path);
            if !lib_path.exists() {
                eprintln!("WARNING: MSINATIVE_LIB_PATH points to non-existent file: {}", msinative_lib_path);
                eprintln!("MSI authentication will be disabled.");
                return false;
            }
            
            if !lib_path.is_file() {
                eprintln!("WARNING: MSINATIVE_LIB_PATH must point to a static library file, not a directory: {}", msinative_lib_path);
                eprintln!("MSI authentication will be disabled.");
                return false;
            }
            
            // Check file extension to ensure it's a static library
            let extension = lib_path.extension().and_then(|ext| ext.to_str()).unwrap_or("");
            let is_static_lib = match extension {
                "a" => true,      // Unix static library
                "lib" => true,    // Windows static library
                _ => false,
            };
            
            if !is_static_lib {
                eprintln!("WARNING: MSINATIVE_LIB_PATH should point to a static library file (.a or .lib): {}", msinative_lib_path);
                eprintln!("Found file with extension: {}", extension);
                eprintln!("MSI authentication will be disabled.");
                return false;
            }

            println!("cargo:warning=---> Valid static library found: {}", msinative_lib_path);
            
            // Extract library directory and name for linking
            if let (Some(lib_dir), Some(lib_name)) = (lib_path.parent(), lib_path.file_stem().and_then(|n| n.to_str())) {
                println!("cargo:warning=---> Library directory: {}", lib_dir.display());
                
                // Determine XPlatLib include directory
                let xplatlib_inc = env::var("XPLATLIB_INC_PATH")
                    .unwrap_or_else(|_| "/home/labhas/strato/Compute-Runtime-Tux/external/GenevaMonAgent-Shared-CrossPlat/src/XPlatLib/inc".to_string());
                
                // Compile the C++ bridge file
                let bridge_path = "src/msi/native/bridge.cpp";
                println!("cargo:warning=---> Compiling C++ bridge: {}", bridge_path);
                println!("cargo:rerun-if-changed={}", bridge_path);
                
                cc::Build::new()
                    .cpp(true)
                    .file(bridge_path)
                    .include(&xplatlib_inc)
                    .flag("-std=c++17")
                    .flag("-fPIC")
                    .compile("msi_bridge");
                
                // Add library search path
                println!("cargo:rustc-link-search=native={}", lib_dir.display());
                
                // Add library to link against (remove 'lib' prefix if present)
                let link_name = if lib_name.starts_with("lib") {
                    &lib_name[3..]
                } else {
                    lib_name
                };
                println!("cargo:warning=---> Linking against library: {}", link_name);
                println!("cargo:rustc-link-lib=static={}", link_name);
                
                // Add platform-specific system libraries that MSI typically depends on
                add_platform_libraries();
                
                eprintln!("INFO: Successfully configured MSI native library: {}", msinative_lib_path);
                true
            } else {
                eprintln!("WARNING: Could not extract library name from path: {}", msinative_lib_path);
                eprintln!("MSI authentication will be disabled.");
                false
            }
        }
        Err(_) => {
            // MSINATIVE_LIB_PATH not set - provide helpful message but don't fail
            eprintln!("INFO: MSINATIVE_LIB_PATH environment variable is not set.");
            eprintln!("MSI authentication will use stub implementation (disabled at runtime).");
            eprintln!("");
            eprintln!("To enable full MSI authentication, please:");
            eprintln!("1. Set MSINATIVE_LIB_PATH to point to your pre-built MSI static library file");
            eprintln!("2. Ensure the library file exists and is accessible");
            eprintln!("");
            eprintln!("Example:");
            eprintln!("  export MSINATIVE_LIB_PATH=/path/to/libmsi.a        # Linux/macOS");
            eprintln!("  export MSINATIVE_LIB_PATH=/path/to/msi.lib         # Windows");
            eprintln!("  cargo build --features msi_auth");
            eprintln!("");
            eprintln!("If you don't need MSI authentication, build without the msi_auth feature:");
            eprintln!("  cargo build");
            
            // Return false to indicate MSI native support is not available
            false
        }
    }
}

#[cfg(feature = "msi_auth")]
fn add_platform_libraries() {
    use std::env;
    
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    
    // Add platform-specific system libraries that MSI authentication typically needs
    match target_os.as_str() {
        "windows" => {
            println!("cargo:rustc-link-lib=advapi32");
            println!("cargo:rustc-link-lib=winhttp");
            println!("cargo:rustc-link-lib=crypt32");
            println!("cargo:rustc-link-lib=ws2_32");
            println!("cargo:rustc-link-lib=secur32");
            println!("cargo:rustc-link-lib=bcrypt");
        }
        "linux" => {
            println!("cargo:rustc-link-lib=stdc++");
            println!("cargo:rustc-link-lib=pthread");
            println!("cargo:rustc-link-lib=dl");
            println!("cargo:rustc-link-lib=ssl");
            println!("cargo:rustc-link-lib=crypto");
            // Additional libraries required by XPlatLib (cpprestsdk dependencies)
            println!("cargo:rustc-link-lib=cpprest");
            println!("cargo:rustc-link-lib=boost_system");
            println!("cargo:rustc-link-lib=boost_thread");
            println!("cargo:rustc-link-lib=boost_atomic");
            println!("cargo:rustc-link-lib=boost_chrono");
            println!("cargo:rustc-link-lib=boost_regex");
        }
        "macos" => {
            println!("cargo:rustc-link-lib=c++");
            println!("cargo:rustc-link-lib=pthread");
            println!("cargo:rustc-link-lib=dl");
            println!("cargo:rustc-link-lib=ssl");
            println!("cargo:rustc-link-lib=crypto");
        }
        _ => {
            eprintln!("WARNING: Unsupported target OS for MSI authentication: {}", target_os);
        }
    }
}
