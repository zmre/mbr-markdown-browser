#!/usr/bin/env bash
# Bump version in Cargo.toml and update Cargo.lock
#
# Usage: ./scripts/bump-version.sh <new_version>
# Example: ./scripts/bump-version.sh 0.4.0
#
# This script updates only Cargo.toml since flake.nix reads the version from it.

set -euo pipefail

if [[ $# -ne 1 ]]; then
    echo "Usage: $0 <new_version>"
    echo "Example: $0 0.4.0"
    exit 1
fi

NEW_VERSION="$1"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

# Validate version format (semver-ish)
if ! [[ "$NEW_VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[a-zA-Z0-9.]+)?$ ]]; then
    echo "Error: Invalid version format. Use semver (e.g., 0.4.0 or 0.4.0-beta.1)"
    exit 1
fi

# Run benchmarks for this release
if [[ "${SKIP_BENCHMARKS:-}" != "1" ]]; then
    echo "Running benchmarks for v$NEW_VERSION..."
    "$SCRIPT_DIR/save-benchmarks.sh" "$NEW_VERSION"
else
    echo "Skipping benchmarks (SKIP_BENCHMARKS=1)"
fi

# Get current version
CURRENT_VERSION=$(grep '^version' "$PROJECT_DIR/Cargo.toml" | head -1 | sed 's/.*"\(.*\)"/\1/')
echo "Current version: $CURRENT_VERSION"
echo "New version: $NEW_VERSION"
echo ""

if [[ "$CURRENT_VERSION" == "$NEW_VERSION" ]]; then
    echo "Error: New version is same as current version"
    exit 1
fi

# Update Cargo.toml
echo "Updating Cargo.toml..."
sed -i.bak "s/^version = \".*\"/version = \"$NEW_VERSION\"/" "$PROJECT_DIR/Cargo.toml"
rm -f "$PROJECT_DIR/Cargo.toml.bak"

# Build and place components (required for cargo check)
echo "Building frontend components..."
cd "$PROJECT_DIR"
nix build .#mbr-components
mkdir -p templates/components-js
cp -r result/* templates/components-js/

# Update Cargo.lock by running cargo check
echo "Updating Cargo.lock..."
cargo check --quiet 2>/dev/null || cargo check

echo ""
echo "Version updated to $NEW_VERSION"
echo ""
echo "Next steps:"
echo "  1. Review changes:  git diff"
echo "  2. Commit:          git commit -am 'Bump version to $NEW_VERSION'"
echo "  3. Tag:             git tag v$NEW_VERSION"
echo "  4. Push:            git push && git push --tags"
echo ""
echo "Note: flake.nix automatically reads version from Cargo.toml, no manual update needed."
