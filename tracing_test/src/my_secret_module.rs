use tracing::error;

pub fn test_tracing_error_inside_module() {
    error!("This is a test error message from my_secret_module.");
}
