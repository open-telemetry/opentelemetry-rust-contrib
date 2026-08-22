$ErrorActionPreference = "Stop"
$PSNativeCommandUseErrorActionPreference = $True

function cargo_feature {
    param (
        $crate,
        $features
    )
    Write-Host "checking '$crate' with features '$features'"
    cargo clippy --manifest-path=$crate/Cargo.toml --all-targets --features "$features" --no-default-features -- -Dwarnings
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
}

# Exit with a nonzero code if there are clippy warnings
cargo clippy --workspace --all-targets --all-features -- -Dwarnings
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

cargo_feature opentelemetry-c/sdk "native-tls"
cargo_feature opentelemetry-c/sdk "rustls-tls"
# OTLP exporter with no TLS backend (HTTP only).
cargo_feature opentelemetry-c/sdk "otlp"
# SDK core with OTLP compiled out entirely (no opentelemetry-otlp / reqwest / TLS).
Write-Host "checking 'opentelemetry-c/sdk' with no default features (SDK core)"
cargo clippy --manifest-path=opentelemetry-c/sdk/Cargo.toml --all-targets --no-default-features -- -Dwarnings
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

cargo_feature opentelemetry-etw-logs "default"
cargo_feature opentelemetry-etw-logs "serde_json"
cargo_feature opentelemetry-etw-logs "logs_unstable_etw_event_name_from_callback"

cargo_feature opentelemetry-etw-metrics ""

exit $LASTEXITCODE