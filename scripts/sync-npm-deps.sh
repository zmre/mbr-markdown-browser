#!/usr/bin/env bash
# sync-npm-deps.sh - Sync package-lock.json and update npmDepsHash in flake.nix
#
# This script ensures the Nix build stays in sync with npm dependencies.
# Run manually or as part of a pre-commit hook.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
COMPONENTS_DIR="$PROJECT_DIR/components"
FLAKE_FILE="$PROJECT_DIR/flake.nix"

cd "$PROJECT_DIR"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# All logging goes to stderr so it doesn't interfere with command substitution
info() { echo -e "${GREEN}[sync-npm-deps]${NC} $*" >&2; }
warn() { echo -e "${YELLOW}[sync-npm-deps]${NC} $*" >&2; }
error() { echo -e "${RED}[sync-npm-deps]${NC} $*" >&2; }

# Check if package.json has changes that need syncing
needs_sync() {
    local pkg_json="$COMPONENTS_DIR/package.json"
    local pkg_lock="$COMPONENTS_DIR/package-lock.json"

    # If package-lock.json doesn't exist, we need to sync
    if [[ ! -f "$pkg_lock" ]]; then
        info "package-lock.json doesn't exist, needs generation"
        return 0
    fi

    # If package.json is newer than package-lock.json, we need to sync
    if [[ "$pkg_json" -nt "$pkg_lock" ]]; then
        info "package.json is newer than package-lock.json"
        return 0
    fi

    return 1
}

# Generate/update package-lock.json from package.json
update_package_lock() {
    info "Updating package-lock.json..."
    cd "$COMPONENTS_DIR"

    # Use npm to generate/update the lockfile
    npm install --package-lock-only --ignore-scripts 2>/dev/null || {
        error "Failed to update package-lock.json"
        return 1
    }

    cd "$PROJECT_DIR"
    info "package-lock.json updated"
}

# Calculate new npmDepsHash
calculate_hash() {
    info "Calculating npmDepsHash..."

    local hash
    # Run prefetch and capture only the hash line (starts with sha256-)
    # Redirect stderr to /dev/null and strip any ANSI color codes
    hash=$(nix run nixpkgs#prefetch-npm-deps -- "$COMPONENTS_DIR/package-lock.json" 2>/dev/null \
        | grep -o 'sha256-[A-Za-z0-9+/=]*' \
        | tail -1) || {
        # Fallback: try nix-prefetch-npm-deps
        hash=$(nix-prefetch-npm-deps "$COMPONENTS_DIR/package-lock.json" 2>/dev/null \
            | grep -o 'sha256-[A-Za-z0-9+/=]*' \
            | tail -1) || {
            error "Failed to calculate npmDepsHash"
            error "Make sure nix is available and package-lock.json is valid"
            return 1
        }
    }

    if [[ -z "$hash" ]]; then
        error "Failed to extract hash from prefetch output"
        return 1
    fi

    echo "$hash"
}

# Update flake.nix with new hash
update_flake() {
    local new_hash="$1"
    local current_hash

    # Extract current hash from flake.nix
    current_hash=$(grep -o 'npmDepsHash = "sha256-[^"]*"' "$FLAKE_FILE" | head -1 | sed 's/npmDepsHash = "//;s/"//')

    if [[ "$current_hash" == "$new_hash" ]]; then
        info "npmDepsHash is already up to date"
        return 0
    fi

    info "Updating npmDepsHash in flake.nix..."
    info "  Old: $current_hash"
    info "  New: $new_hash"

    # Use sed to replace the hash (works on both macOS and Linux)
    if [[ "$OSTYPE" == "darwin"* ]]; then
        sed -i '' "s|npmDepsHash = \"sha256-[^\"]*\"|npmDepsHash = \"$new_hash\"|" "$FLAKE_FILE"
    else
        sed -i "s|npmDepsHash = \"sha256-[^\"]*\"|npmDepsHash = \"$new_hash\"|" "$FLAKE_FILE"
    fi

    info "flake.nix updated"
    return 0
}

# Main
main() {
    local force=false

    # Parse arguments
    while [[ $# -gt 0 ]]; do
        case "$1" in
            -f|--force)
                force=true
                shift
                ;;
            -h|--help)
                echo "Usage: $0 [-f|--force]"
                echo ""
                echo "Sync package-lock.json and update npmDepsHash in flake.nix"
                echo ""
                echo "Options:"
                echo "  -f, --force    Force sync even if files appear up to date"
                echo "  -h, --help     Show this help message"
                exit 0
                ;;
            *)
                error "Unknown option: $1"
                exit 1
                ;;
        esac
    done

    # Check if sync is needed
    if ! $force && ! needs_sync; then
        info "Dependencies appear to be in sync (use -f to force)"
        exit 0
    fi

    # Update package-lock.json
    update_package_lock || exit 1

    # Calculate new hash
    local new_hash
    new_hash=$(calculate_hash) || exit 1

    # Update flake.nix
    update_flake "$new_hash" || exit 1

    info "Done! Remember to stage the updated files:"
    info "  git add components/package-lock.json flake.nix"
}

main "$@"
