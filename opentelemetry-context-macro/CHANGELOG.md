# Changelog

## vNext

- Initial release of `#[propagate_context]`, an attribute macro that attaches the OpenTelemetry
  context of an async function to every future the function awaits, so work after a suspension
  point stays in the trace it started in. The macro creates no span.
