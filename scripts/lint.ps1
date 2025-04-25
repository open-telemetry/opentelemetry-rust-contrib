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

cargo_feature opentelemetry-etw-logs "default"
cargo_feature opentelemetry-etw-logs "spec_unstable_logs_enabled"
cargo_feature opentelemetry-etw-logs "serde_json"

cargo_feature opentelemetry-etw-metrics ""

exit $LASTEXITCODE