#!/usr/bin/env bash
# Update script for fetching new versions of highlight.js and mermaid.js assets
# Usage: ./scripts/update-assets.sh [--hljs VERSION] [--mermaid VERSION]
#
# Examples:
#   ./scripts/update-assets.sh                           # Show current versions
#   ./scripts/update-assets.sh --hljs 11.11.1           # Update highlight.js
#   ./scripts/update-assets.sh --mermaid 11.12.2        # Update mermaid.js
#   ./scripts/update-assets.sh --hljs 11.11.1 --mermaid 11.12.2  # Update both

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TEMPLATES_DIR="$SCRIPT_DIR/../templates"

# highlight.js language modules to download
HLJS_LANGUAGES=(
    javascript
    typescript
    rust
    python
    bash
    java
    scala
    go
    ruby
    css
    json
    yaml
    xml
    sql
    dockerfile
    markdown
)

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

info() { echo -e "${GREEN}[INFO]${NC} $1"; }
warn() { echo -e "${YELLOW}[WARN]${NC} $1"; }
error() { echo -e "${RED}[ERROR]${NC} $1"; exit 1; }

show_current_versions() {
    echo "Current asset versions in templates/:"
    echo

    # Find hljs version
    hljs_file=$(ls "$TEMPLATES_DIR"/hljs.*.js 2>/dev/null | grep -v lang | head -1 || true)
    if [[ -n "$hljs_file" ]]; then
        hljs_version=$(basename "$hljs_file" | sed 's/hljs\.\(.*\)\.js/\1/')
        echo "  highlight.js: $hljs_version"
    else
        echo "  highlight.js: not found"
    fi

    # Find mermaid version
    mermaid_file=$(ls "$TEMPLATES_DIR"/mermaid.*.min.js 2>/dev/null | head -1 || true)
    if [[ -n "$mermaid_file" ]]; then
        mermaid_version=$(basename "$mermaid_file" | sed 's/mermaid\.\(.*\)\.min\.js/\1/')
        echo "  mermaid.js: $mermaid_version"
    else
        echo "  mermaid.js: not found"
    fi

    echo
}

download_hljs() {
    local version="$1"
    local base_url="https://cdnjs.cloudflare.com/ajax/libs/highlight.js/${version}"

    info "Downloading highlight.js v${version}..."

    # Download core
    info "  Core: highlight.min.js"
    curl -sL "${base_url}/highlight.min.js" -o "$TEMPLATES_DIR/hljs.${version}.js" || error "Failed to download highlight.min.js"

    # Download dark theme CSS
    info "  Theme: dark.min.css"
    curl -sL "${base_url}/styles/dark.min.css" -o "$TEMPLATES_DIR/hljs.dark.${version}.css" || error "Failed to download dark.min.css"

    # Download language modules
    for lang in "${HLJS_LANGUAGES[@]}"; do
        info "  Language: ${lang}"
        curl -sL "${base_url}/languages/${lang}.min.js" -o "$TEMPLATES_DIR/hljs.lang.${lang}.${version}.js" || warn "Failed to download ${lang}.min.js"
    done

    echo
    info "highlight.js v${version} downloaded successfully!"
    echo
    warn "Remember to update server.rs DEFAULT_FILES with new version numbers:"
    echo "  - hljs.${version}.js"
    echo "  - hljs.dark.${version}.css"
    for lang in "${HLJS_LANGUAGES[@]}"; do
        echo "  - hljs.lang.${lang}.${version}.js"
    done
    echo
}

download_mermaid() {
    local version="$1"
    local url="https://cdn.jsdelivr.net/npm/mermaid@${version}/dist/mermaid.min.js"

    info "Downloading mermaid.js v${version}..."
    info "  Source: ${url}"

    curl -sL "$url" -o "$TEMPLATES_DIR/mermaid.${version}.min.js" || error "Failed to download mermaid.min.js"

    echo
    info "mermaid.js v${version} downloaded successfully!"
    echo
    warn "Remember to update server.rs DEFAULT_FILES with new version:"
    echo "  - mermaid.${version}.min.js"
    echo
}

# Parse arguments
HLJS_VERSION=""
MERMAID_VERSION=""

while [[ $# -gt 0 ]]; do
    case $1 in
        --hljs)
            HLJS_VERSION="$2"
            shift 2
            ;;
        --mermaid)
            MERMAID_VERSION="$2"
            shift 2
            ;;
        -h|--help)
            echo "Usage: $0 [--hljs VERSION] [--mermaid VERSION]"
            echo
            echo "Options:"
            echo "  --hljs VERSION     Download highlight.js at specified version"
            echo "  --mermaid VERSION  Download mermaid.js at specified version"
            echo "  -h, --help         Show this help message"
            echo
            echo "Examples:"
            echo "  $0                           # Show current versions"
            echo "  $0 --hljs 11.11.1           # Update highlight.js"
            echo "  $0 --mermaid 11.12.2        # Update mermaid.js"
            exit 0
            ;;
        *)
            error "Unknown option: $1"
            ;;
    esac
done

# If no arguments, show current versions
if [[ -z "$HLJS_VERSION" && -z "$MERMAID_VERSION" ]]; then
    show_current_versions
    echo "To update assets, run with version arguments:"
    echo "  $0 --hljs 11.11.1 --mermaid 11.12.2"
    exit 0
fi

# Download requested assets
if [[ -n "$HLJS_VERSION" ]]; then
    download_hljs "$HLJS_VERSION"
fi

if [[ -n "$MERMAID_VERSION" ]]; then
    download_mermaid "$MERMAID_VERSION"
fi

info "Done! Don't forget to:"
echo "  1. Update include_bytes! paths in src/server.rs"
echo "  2. Remove old version files from templates/"
echo "  3. Run 'cargo build' to verify"
echo "  4. Run 'cargo test' to ensure everything works"
