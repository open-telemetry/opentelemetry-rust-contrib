#!/bin/bash

set -e  # Exit immediately if a command exits with a non-zero status

echo "================================================================================"
echo "IMPORTANT DISCLAIMER: Bond is deprecated. This script is intended only for CI testing and"
echo "should not be used in any other prod/non-prod environment."
echo "================================================================================"

if [ "$GITHUB_ACTIONS" != "true" ]; then
  echo "ERROR: This script should only be run in a GitHub Actions CI environment. Exiting."
  exit 1
fi

for cmd in git cmake make; do
    command -v $cmd >/dev/null 2>&1 || { echo >&2 "$cmd is required but not installed. Aborting."; exit 1; }
done

# Allow custom install dir as first argument
INSTALL_DIR="${1:-$(pwd)/bond_build}"

# Variables
BOND_REPO_URL="https://github.com/microsoft/bond.git"
BOND_CLONE_DIR="bond_repo"
BOND_BUILD_DIR="bond_build_temp"

# Step 1: Clone the Bond repository if not already cloned
if [ ! -d "$BOND_CLONE_DIR" ]; then
    echo "Cloning Bond repository..."
    git clone --recurse-submodules "$BOND_REPO_URL" "$BOND_CLONE_DIR"
else
    echo "Bond repository already cloned."
fi

# Step 2: Create the build directory
mkdir -p "$BOND_BUILD_DIR"

# Step 3: Build the Bond library
echo "Building Bond..."
cd "$BOND_BUILD_DIR"
cmake ../"$BOND_CLONE_DIR" -DCMAKE_INSTALL_PREFIX="$INSTALL_DIR"
make -j$(nproc)

# Step 4: Install Bond locally
echo "Installing Bond locally in $INSTALL_DIR ..."
make install

# Step 5: Display and export paths for integration
echo "Bond build and installation completed."
echo "Include directory: $INSTALL_DIR/include"
echo "Library directory: $INSTALL_DIR/lib"

echo ""
echo "To use with your Rust build, export these variables:"
echo "export BOND_INCLUDE_DIR=\"$INSTALL_DIR/include\""
echo "export BOND_LIB_DIR=\"$INSTALL_DIR/lib\""
export BOND_INCLUDE_DIR="$INSTALL_DIR/include"
export BOND_LIB_DIR="$INSTALL_DIR/lib/bond"

# Step 6: Clean up if required
# Uncomment the following lines to clean up after the build
# echo "Cleaning up temporary files..."
# cd ..
# rm -rf "$BOND_CLONE_DIR" "$BOND_BUILD_DIR"

echo "Done."