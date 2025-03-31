//! ## zPages processor
//!
//! ZPages processor collect span information when span starts or ends and send it to [`SpanAggregator`]
//! for further process.
//!
//! [`SpanAggregator`]:../struct.SpanAggregator.html
use crate::trace::TracezMessage;
use async_channel::Sender;
use opentelemetry::Context;
use opentelemetry_sdk::{
    error::OTelSdkResult,
    trace::SpanData,
    trace::{Span, SpanProcessor},
};
use std::fmt::Formatter;

/// ZPagesSpanProcessor is an alternative to external exporters. It sends span data to zPages server
/// where it will be archive and user can use this information for debug purpose.
///
/// ZPagesSpanProcessor employs a `SpanAggregator` running as another task to aggregate the spans
/// using the name of spans.
#[deprecated(note = "This crate is deprecated and no longer maintained.")]
pub struct ZPagesSpanProcessor {
    tx: Sender<TracezMessage>,
}

impl std::fmt::Debug for ZPagesSpanProcessor {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str("ZPageProcessor")
    }
}

impl ZPagesSpanProcessor {
    /// Create a new `ZPagesSpanProcessor`.
    pub fn new(tx: Sender<TracezMessage>) -> ZPagesSpanProcessor {
        ZPagesSpanProcessor { tx }
    }
}

impl SpanProcessor for ZPagesSpanProcessor {
    fn on_start(&self, span: &mut Span, _cx: &Context) {
        // if the aggregator is already dropped. This is a no-op
        if let Some(data) = span.exported_data() {
            let _ = self.tx.try_send(TracezMessage::SampleSpan(data));
        }
    }

    fn on_end(&self, span: SpanData) {
        // if the aggregator is already dropped. This is a no-op
        let _ = self.tx.try_send(TracezMessage::SpanEnd(span));
    }

    fn force_flush(&self) -> OTelSdkResult {
        // do nothing
        Ok(())
    }

    fn shutdown(&self) -> OTelSdkResult {
        // do nothing
        Ok(())
    }
}
