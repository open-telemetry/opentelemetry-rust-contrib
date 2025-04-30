//! Process resource detector
//!
//! Detect process related information like pid, executable name.

use opentelemetry::{KeyValue, StringValue, Value};
use opentelemetry_sdk::resource::ResourceDetector;
use opentelemetry_sdk::Resource;
use std::env::args_os;
use std::process::id;

/// Detect process information.
///
/// This resource detector returns the following information:
///
/// - process command line arguments(`process.command_args`), the full command arguments of this
///   application.
/// - OS assigned process id(`process.pid`).
pub struct ProcessResourceDetector;

impl ResourceDetector for ProcessResourceDetector {
    fn detect(&self) -> Resource {
        let arguments = args_os();
        let cmd_arg_val = arguments
            .into_iter()
            .map(|arg| arg.to_string_lossy().into_owned().into())
            .collect::<Vec<StringValue>>();
        Resource::builder_empty()
            .with_attributes(
                vec![
                    Some(KeyValue::new(
                        opentelemetry_semantic_conventions::attribute::PROCESS_COMMAND_ARGS,
                        Value::Array(cmd_arg_val.into()),
                    )),
                    Some(KeyValue::new(
                        opentelemetry_semantic_conventions::attribute::PROCESS_PID,
                        id() as i64,
                    )),
                    Some(KeyValue::new(
                        opentelemetry_semantic_conventions::attribute::PROCESS_RUNTIME_NAME,
                        "rustc",
                    )),
                    option_env!("RUSTC_VERSION").map(|rustc_version| {
                        KeyValue::new(
                            opentelemetry_semantic_conventions::attribute::PROCESS_RUNTIME_VERSION,
                            rustc_version,
                        )
                    }),
                ]
                .into_iter()
                .flatten(),
            )
            .build()
    }
}

#[cfg(test)]
mod tests {
    use super::ProcessResourceDetector;
    use opentelemetry_sdk::resource::ResourceDetector;

    #[cfg(target_os = "linux")]
    #[test]
    fn test_processor_resource_detector() {
        let resource = ProcessResourceDetector.detect();
        assert_eq!(resource.len(), 4); // we cannot assert on the values because it changes along with runtime.
    }

    #[test]
    fn test_processor_resource_detector_runtime() {
        use opentelemetry_semantic_conventions::attribute::{
            PROCESS_RUNTIME_NAME, PROCESS_RUNTIME_VERSION,
        };

        let resource = ProcessResourceDetector.detect();

        assert_eq!(
            resource.get(&PROCESS_RUNTIME_NAME.into()),
            Some("rustc".into())
        );

        if option_env!("RUSTC_VERSION").is_some() {
            assert_eq!(
                resource.get(&PROCESS_RUNTIME_VERSION.into()),
                Some(env!("RUSTC_VERSION").into())
            );
        }
    }
}
