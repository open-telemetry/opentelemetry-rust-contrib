[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    $version
)

$ErrorActionPreference = "Stop"
$PSNativeCommandUseErrorActionPreference = $True

# function to check if specified toolchain is installed
function check_rust_toolchain_installed {
    param (
        $version
    )

    if (!(rustup toolchain list | Select-String -Pattern $version -Quiet)) {
        Write-Host "Rust toolchain $version is not installed. Please install it using 'rustup toolchain install $version'."
        exit 1
    }
}

$RUST_VERSION = $version

# Determine the directory containing the script
$SCRIPT_DIR = $PSScriptRoot

# Path to the configuration file
$CONFIG_FILE="$SCRIPT_DIR/msrv_config.json"

if (-not (Test-Path $CONFIG_FILE)) {
    Write-Host "Configuration file $CONFIG_FILE not found."
    exit 1
}

# check if specified toolchain is installed
check_rust_toolchain_installed "$RUST_VERSION"
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}

# Extract the exact installed rust version string
$installed_version = $(rustup toolchain list | Select-String -pattern 1.75.0).ToString().Split(" ")[0]

# Read the configuration file and get the packages for the specified version
$packages = $(Get-Content "$CONFIG_FILE" | ConvertFrom-Json )."$RUST_VERSION"
if (-not $packages) {
    Write-Host "No packages found for Rust version $RUST_VERSION in the configuration file."
    exit 1
}

# Check MSRV for the packages
foreach ($package in $packages) {
    Write-Host "Verifying MSRV version $installed_version for $package"
    rustup run $installed_version cargo msrv verify --path $package --output-format json
    if ($LASTEXITCODE -ne 0) {
        exit $LASTEXITCODE
    }
    Write-Host "" # just for nicer separation between packages
}
