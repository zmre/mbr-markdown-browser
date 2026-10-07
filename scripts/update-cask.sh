#!/usr/bin/env bash
# Point the Homebrew cask (Casks/mbr.rb) at a new release
#
# Usage: ./scripts/update-cask.sh <version> <sha256>
# Example: ./scripts/update-cask.sh 0.6.2 c542109a48d6...690b57
#
# <sha256> is the checksum of mbr-macos-arm64.dmg from the release's SHA256SUMS.
# The release workflow's `update-homebrew-cask` job calls this after every
# stable release; run it by hand only to repair a bad bump (see
# docs/releasing.md).
#
# Rewrites exactly the cask's top-level `  version "..."` and `  sha256 "..."`
# lines and nothing else. awk rather than `sed -i`: BSD and GNU sed disagree on
# `-i`, and both are common on macOS (Nix puts GNU sed first on PATH).
#
# CASK_FILE overrides the target file (used by CI to smoke-test this script on
# a scratch copy).

set -euo pipefail

if [[ $# -ne 2 ]]; then
    echo "Usage: $0 <version> <sha256>" >&2
    echo "Example: $0 0.6.2 <sha256 of mbr-macos-arm64.dmg>" >&2
    exit 1
fi

VERSION="$1"
SHA256="$2"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
CASK_FILE="${CASK_FILE:-$PROJECT_DIR/Casks/mbr.rb}"

# Same pattern as bump-version.sh. CI never calls this for a prerelease, but a
# hand-run against one is the operator's call.
if ! [[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[a-zA-Z0-9.]+)?$ ]]; then
    echo "Error: Invalid version '$VERSION'. Use semver (e.g., 0.6.2)" >&2
    exit 1
fi

# Lowercase only: that is what sha256sum/shasum print and what `brew style`
# expects.
if ! [[ "$SHA256" =~ ^[0-9a-f]{64}$ ]]; then
    echo "Error: Invalid sha256 '$SHA256'. Expected 64 lowercase hex characters" >&2
    exit 1
fi

if [[ ! -f "$CASK_FILE" ]]; then
    echo "Error: Cask file not found: $CASK_FILE" >&2
    exit 1
fi

# Each line must occur exactly once. Zero means the cask was restructured and
# this script no longer knows what it is editing; two means a rewrite would be
# ambiguous. Either way, refuse rather than produce a half-updated cask.
for key in version sha256; do
    count=$(grep -cE "^  $key \"[^\"]*\"$" "$CASK_FILE" || true)
    if [[ "$count" -ne 1 ]]; then
        echo "Error: Expected exactly one '  $key \"...\"' line in $CASK_FILE, found $count" >&2
        exit 1
    fi
done

# Write to a temp file in the same directory, then rename over the original,
# so a failure part-way never leaves a truncated cask behind.
tmp=$(mktemp "$CASK_FILE.XXXXXX")
trap 'rm -f "$tmp"' EXIT

awk -v version="$VERSION" -v sha="$SHA256" '
    /^  version "[^"]*"$/ { print "  version \"" version "\""; next }
    /^  sha256 "[^"]*"$/  { print "  sha256 \"" sha "\""; next }
    { print }
' "$CASK_FILE" > "$tmp"

# mktemp creates the file 0600; git tracks the cask as 0644.
chmod 0644 "$tmp"
mv "$tmp" "$CASK_FILE"
trap - EXIT

echo "Updated $CASK_FILE to version $VERSION (sha256 $SHA256)"
