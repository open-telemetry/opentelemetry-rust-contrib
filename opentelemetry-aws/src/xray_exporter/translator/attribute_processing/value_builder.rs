//! Deferred attribute processing for X-Ray fields requiring multiple source attributes.
//!
//! This module implements the second phase of the two-phase attribute processing system.
//! [`ValueBuilder`] implementations accumulate attribute values via [`SpanAttributeProcessor`]
//! during the first pass, then compute and set the final X-Ray field in their `resolve()` method.
//! This enables complex field computations like assembling `http.request.url` from `url.scheme`,
//! `url.domain`, `url.port`, `url.path`, and `url.query`. Contrast with immediate processing
//! where [`SpanAttributeProcessor`] is implemented directly on [`AnyDocumentBuilder`] to set
//! X-Ray fields without accumulation.

use super::{DispatchTable, HandlerId, SpanAttributeProcessor};
use crate::xray_exporter::translator::{error::Result, AnyDocumentBuilder};
use opentelemetry::Value;

mod aws_operation_builder;
mod aws_xray_sdk_builder;
mod beanstalk_deployment_id_builder;
mod cause_builder;
mod cloudwatch_log_group_builder;
mod http_request_url_builder;
mod http_response_content_lenght_builder;
mod segment_name_builder;
mod segment_origin_builder;
mod sql_url_builder;
mod subsegment_namespace_builder;

pub(in crate::xray_exporter::translator) use aws_operation_builder::AwsOperationBuilder;
pub(in crate::xray_exporter::translator) use aws_xray_sdk_builder::AwsXraySdkBuilder;
pub(in crate::xray_exporter::translator) use beanstalk_deployment_id_builder::BeanstalkDeploymentIdBuilder;
pub(in crate::xray_exporter::translator) use cause_builder::CauseBuilder;
pub(in crate::xray_exporter::translator) use cloudwatch_log_group_builder::CloudwatchLogGroupBuilder;
pub(in crate::xray_exporter::translator) use http_request_url_builder::HttpRequestUrlBuilder;
pub(in crate::xray_exporter::translator) use http_response_content_lenght_builder::HttpResponseContentLengthBuilder;
pub(in crate::xray_exporter::translator) use segment_name_builder::SegmentNameBuilder;
pub(in crate::xray_exporter::translator) use segment_origin_builder::SegmentOriginBuilder;
pub(in crate::xray_exporter::translator) use sql_url_builder::SqlUrlBuilder;
pub(in crate::xray_exporter::translator) use subsegment_namespace_builder::SubsegmentNamespaceBuilder;

/// Builders that accumulate multiple attribute values before computing a final X-Ray field.
///
/// This trait enables two-phase attribute processing: builders implement [`SpanAttributeProcessor`]
/// to collect attribute values during the first pass, then `resolve()` is called to compute and
/// set the final X-Ray field value. This is necessary when a single X-Ray field requires combining
/// multiple OpenTelemetry attributes (e.g., `http.request.url` requires `url.scheme`, `url.domain`,
/// `url.port`, `url.path`, `url.query`). Differs from immediate processing via [`SpanAttributeProcessor`]
/// on [`AnyDocumentBuilder`], which sets X-Ray fields directly without accumulation.
pub(in super::super) trait ValueBuilder<'value> {
    /// Computes and sets the final X-Ray field value after all attributes are processed.
    fn resolve(self, segment_builder: &mut AnyDocumentBuilder<'value>) -> Result<()>;
}

macro_rules! count_tts {
    () => { 0 };
    ($odd:tt $($a:tt $b:tt)*) => { (count_tts!($($a)*) << 1) | 1 };
    ($($a:tt $even:tt)*) => { count_tts!($($a)*) << 1 };
}

/// Generates the [`AnyValueBuilder`] enum and its dispatch infrastructure.
///
/// Creates an enum with variants for each concrete [`ValueBuilder`] type, constructor methods
/// for each variant, a `process()` method that dispatches to the correct builder based on
/// `builder_id`, and a [`ValueBuilder`] implementation that delegates `resolve()` to the inner
/// builder. This reduces boilerplate when adding new value builders to the system.
macro_rules! any_builder_enum {
    ($($module:ident :: $variant:ident $(:: <$lt:lifetime>)? , )*) => {
        /// Type-erased wrapper around concrete [`ValueBuilder`] implementations.
        ///
        /// Provide helper methods to register the [ValueBuilder]s on the [DispatchTable]
        /// and to construct the [AnyValueBuilder] array in a manner that ensures
        /// coherence between [ProcessorId]s and array indexes.
        ///
        /// [ProcessorId]: super::ProcessorId
        #[allow(clippy::enum_variant_names)]
        #[derive(Debug)]
        pub(in super::super) enum AnyValueBuilder<'v> {
            $($variant($module::$variant$(<$lt>)?),)*
        }

        impl<'v> AnyValueBuilder<'v> {

            /// Dispatches attribute processing to the correct builder variant.
            ///
            /// Matches `builder_id` against the variant's `SpanAttributeProcessor::ID` and invokes
            /// the handler at `handler_index` from the builder's `HANDLERS` array.
            #[inline(always)]
            pub fn process(&mut self, handler_index: HandlerId, value: &'v Value) -> bool {
                match self {
                    $(
                        Self::$variant(builder) => $module::$variant::HANDLERS[handler_index].1(builder, value),
                    )*
                }
            }

            /// Return the number of additional builders
            pub const fn count() -> usize {
                count_tts!($($module)*)
            }

            /// Register every builders on the DispatchTable with a deterministic index
            pub fn register_builders(dp: &mut DispatchTable) {
                let mut processor_id = 0;
                $(
                    processor_id += 1;
                    dp.register::<
                    {
                        const fn __len<T, const N: usize>(_: &[T; N]) -> usize { N }
                        __len(&$module::$variant::HANDLERS)
                    },
                    $module::$variant
                    >(processor_id - 1);
                )*
            }

            /// Ensure the additional_builders array is in the correct order
            #[allow(clippy::too_many_arguments)]
            #[inline(always)]
            pub fn array($($module: $module::$variant$(<$lt>)?),*) -> [Self; count_tts!($($module)*)] {
                [
                    $(Self::$variant($module)),*
                ]
            }
        }

        impl<'v> ValueBuilder<'v> for AnyValueBuilder<'v> {
            fn resolve(self, segment_builder: &mut AnyDocumentBuilder<'v>) -> Result<()> {
                match self {
                    $(
                        Self::$variant(builder) => builder.resolve(segment_builder),
                    )*
                }
            }
        }
    };
}
// Impl a AnyValueBuilder type with constructor for each possible builder
any_builder_enum!(
    aws_operation_builder::AwsOperationBuilder::<'v>,
    aws_xray_sdk_builder::AwsXraySdkBuilder::<'v>,
    beanstalk_deployment_id_builder::BeanstalkDeploymentIdBuilder::<'v>,
    cause_builder::CauseBuilder::<'v>,
    cloudwatch_log_group_builder::CloudwatchLogGroupBuilder::<'v>,
    http_request_url_builder::HttpRequestUrlBuilder::<'v>,
    http_response_content_lenght_builder::HttpResponseContentLengthBuilder,
    segment_name_builder::SegmentNameBuilder::<'v>,
    segment_origin_builder::SegmentOriginBuilder::<'v>,
    sql_url_builder::SqlUrlBuilder::<'v>,
    subsegment_namespace_builder::SubsegmentNamespaceBuilder,
);
