$ErrorActionPreference = "Stop"

cargo_feature() {
    echo "checking $1 with features $2"
    cargo clippy --manifest-path=$1/Cargo.toml --all-targets --features "$2" --no-default-features -- \
    `# Exit with a nonzero code if there are clippy warnings` \
    -Dwarnings
}

# Exit with a nonzero code if there are clippy warnings
cargo clippy --workspace --all-targets --all-features -- -Dwarnings
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

cargo_feature opentelemetry-etw-logs "default"
cargo_feature opentelemetry-etw-logs "spec_unstable_logs_enabled"

cargo_feature opentelemetry-etw-metrics ""

exit $LASTEXITCODE