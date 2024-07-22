#[cfg(feature = "aws-lambda")]
mod lambda;
#[cfg(feature = "aws-lambda")]
pub use lambda::LambdaResourceDetector;

