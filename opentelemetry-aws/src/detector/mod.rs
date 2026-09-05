#[cfg(feature = "detector-aws-lambda")]
mod lambda;
#[cfg(feature = "detector-aws-lambda")]
pub use lambda::LambdaResourceDetector;

#[cfg(feature = "detector-aws-ec2")]
mod ec2;
#[cfg(feature = "detector-aws-ec2")]
pub use ec2::Ec2ResourceDetector;

#[cfg(feature = "detector-aws-ecs")]
mod ecs;
#[cfg(feature = "detector-aws-ecs")]
pub use ecs::EcsResourceDetector;

#[cfg(feature = "detector-aws-eks")]
mod eks;
#[cfg(feature = "detector-aws-eks")]
pub use eks::EksResourceDetector;

#[cfg(any(
    feature = "detector-aws-ec2",
    feature = "detector-aws-ecs",
    feature = "detector-aws-eks"
))]
mod imds;

#[cfg(any(
    feature = "detector-aws-lambda",
    feature = "detector-aws-ec2",
    feature = "detector-aws-ecs",
    feature = "detector-aws-eks"
))]
mod utils;
