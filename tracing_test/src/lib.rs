use tracing::error;

pub mod my_secret_module;

pub fn test_tracing_error_inside_lib() {
    error!("This is a test error message from lib.rs.");
}
