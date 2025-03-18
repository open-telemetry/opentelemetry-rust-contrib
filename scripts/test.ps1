$ErrorActionPreference = "Stop"
$PSNativeCommandUseErrorActionPreference = $True

cargo test --manifest-path=opentelemetry-etw-logs/Cargo.toml --all-features
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

cargo test --manifest-path=opentelemetry-etw-metrics/Cargo.toml --all-features
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

exit $LASTEXITCODE