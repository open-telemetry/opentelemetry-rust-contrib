#[cfg(feature = "detector-aws-lambda")]
mod lambda;
#[cfg(feature = "detector-aws-lambda")]
pub use lambda::LambdaResourceDetector;
