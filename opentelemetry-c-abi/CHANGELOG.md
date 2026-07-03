# Changelog

## vNext

### Added

- Initial release of `opentelemetry-c-abi` as part of the split of `opentelemetry-c` into
  separate C **API** and **SDK** artifacts. This internal crate holds the shared
  `#[repr(C)]` value types and the internal implementation vtable used across the API/SDK
  C-ABI boundary. It has no global state and no exported symbols.
