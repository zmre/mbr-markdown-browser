#!/usr/bin/env bash
# Update the vendored front-end assets under templates/.
#
# Usage:
#   ./scripts/update-assets.sh                    # report vendored vs. latest upstream versions
#   ./scripts/update-assets.sh --all              # update everything that is behind
#   ./scripts/update-assets.sh --hljs 11.11.2     # update one asset to an exact version
#   ./scripts/update-assets.sh --katex latest     # ...or to whatever upstream calls latest
#
# Assets covered: highlight.js, mermaid, KaTeX (+ fonts), reveal.js, Pico CSS.
#
# Most of these files carry their version in the filename and are named by
# `include_bytes!` in src/, so a version bump renames files the Rust code
# references. Every downloader therefore rewrites those paths in src/*.rs
# itself: the old two-step (download here, hand-edit include_bytes! there) is
# how you end up with a build that fails or, worse, one that still embeds the
# old bytes because only some of the paths got updated.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
TEMPLATES_DIR="$PROJECT_DIR/templates"
SRC_DIR="$PROJECT_DIR/src"
HLJS_COMPONENT="$PROJECT_DIR/components/src/mbr-hljs.ts"

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
    nix
    css
    json
    yaml
    xml
    sql
    dockerfile
    markdown
)

# Pico colour variants. Each exists twice upstream: `pico.<colour>.min.css` and
# `pico.fluid.classless.<colour>.min.css`. embedded_pico.rs names all of them.
PICO_COLORS=(
    amber blue cyan fuchsia green grey indigo jade lime orange
    pink pumpkin purple red sand slate violet yellow zinc
)

# Options shared by every download.
#
# --fail is the load-bearing one: without it curl exits 0 on an HTTP error,
# writes the error body into the asset file, and neither `set -e` nor the
# `|| error` handlers below ever fire — the script then reports "downloaded
# successfully" over a 52-byte error page that gets include_bytes!'d into the
# binary. --proto '=https' and --tlsv1.2 keep the transfer on modern TLS.
CURL_OPTS=(--silent --location --fail --proto '=https' --tlsv1.2)

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

info() { echo -e "${GREEN}[INFO]${NC} $1"; }
warn() { echo -e "${YELLOW}[WARN]${NC} $1"; }
error() { echo -e "${RED}[ERROR]${NC} $1"; exit 1; }

# Downloads land here first and are only moved into templates/ once every file
# for that asset has arrived, so a mid-run failure cannot leave templates/
# holding half of one version and half of another.
STAGE=""
cleanup() { [[ -n "$STAGE" && -d "$STAGE" ]] && rm -rf "$STAGE"; }
trap cleanup EXIT
STAGE="$(mktemp -d)"

# --- version discovery ---------------------------------------------------

# Latest published version of an npm package, or "" if the registry is
# unreachable. Used for reporting and for `--<asset> latest`.
npm_latest() {
    curl "${CURL_OPTS[@]}" --max-time 20 "https://registry.npmjs.org/$1/latest" 2>/dev/null \
        | sed -n 's/.*"version":"\([^"]*\)".*/\1/p' | head -1 || true
}

# Version currently vendored, read back from what is on disk. Filenames are the
# source of truth for everything except Pico, whose files are unversioned and
# whose version only survives in the CSS banner comment.
current_hljs() {
    local f
    f=$(ls "$TEMPLATES_DIR"/hljs.*.js 2>/dev/null | grep -v '\.lang\.' | head -1 || true)
    [[ -n "$f" ]] && basename "$f" | sed 's/hljs\.\(.*\)\.js/\1/'
    return 0
}
current_mermaid() {
    local f
    f=$(ls "$TEMPLATES_DIR"/mermaid.*.min.js 2>/dev/null | head -1 || true)
    [[ -n "$f" ]] && basename "$f" | sed 's/mermaid\.\(.*\)\.min\.js/\1/'
    return 0
}
current_katex() {
    local f
    f=$(ls "$TEMPLATES_DIR"/katex.*.min.js 2>/dev/null | head -1 || true)
    [[ -n "$f" ]] && basename "$f" | sed 's/katex\.\(.*\)\.min\.js/\1/'
    return 0
}
current_reveal() {
    local f
    f=$(ls "$TEMPLATES_DIR"/reveal.*.js 2>/dev/null | grep -v '\.notes\.' | head -1 || true)
    [[ -n "$f" ]] && basename "$f" | sed 's/reveal\.\(.*\)\.js/\1/'
    return 0
}
current_pico() {
    [[ -f "$TEMPLATES_DIR/pico-main/pico.min.css" ]] || return 0
    sed -n 's/.*Pico CSS[^v]*v\([0-9][0-9.]*\).*/\1/p' "$TEMPLATES_DIR/pico-main/pico.min.css" | head -1
}

report_versions() {
    local check_remote="$1"
    local name cur latest
    echo "Vendored asset versions in templates/:"
    echo
    printf "  %-14s %-12s %s\n" "ASSET" "VENDORED" "LATEST"
    for name in highlight.js mermaid katex reveal.js pico; do
        case "$name" in
            highlight.js) cur=$(current_hljs) ;;
            mermaid)      cur=$(current_mermaid) ;;
            katex)        cur=$(current_katex) ;;
            reveal.js)    cur=$(current_reveal) ;;
            pico)         cur=$(current_pico) ;;
        esac
        latest=""
        if [[ "$check_remote" == "true" ]]; then
            case "$name" in
                pico) latest=$(npm_latest "@picocss/pico") ;;
                *)    latest=$(npm_latest "$name") ;;
            esac
        fi
        printf "  %-14s %-12s %s\n" "$name" "${cur:-not found}" "${latest:-?}"
    done
    echo
}

# --- helpers -------------------------------------------------------------

# sed -i differs between GNU and BSD; same detection as scripts/sync-npm-deps.sh.
sed_inplace() {
    local expr="$1"; shift
    if sed --version 2>/dev/null | grep -q GNU; then
        sed -i "$expr" "$@"
    else
        sed -i '' "$expr" "$@"
    fi
}

# Repoint every `include_bytes!`/route reference from one vendored filename to
# another. Dots are escaped so `hljs.11.11.1.js` cannot match `hljs.11X11X1.js`.
rename_refs() {
    local old="$1" new="$2"
    [[ "$old" == "$new" ]] && return 0
    local old_escaped="${old//./\\.}"
    sed_inplace "s|$old_escaped|$new|g" "$SRC_DIR"/*.rs
}

fetch() {
    local url="$1" dest="$2"
    curl "${CURL_OPTS[@]}" "$url" -o "$dest" || error "Failed to download $url"
}

# --- highlight.js --------------------------------------------------------

# Sourced from the highlightjs/cdn-release repo rather than cdnjs: cdnjs lags
# upstream by releases at a time (it was still on 11.11.1 when 11.11.2 shipped),
# and cdn-release is the tree cdnjs itself mirrors.
download_hljs() {
    local version="$1"
    local old; old=$(current_hljs)
    local build="https://cdn.jsdelivr.net/gh/highlightjs/cdn-release@${version}/build"
    local src="https://cdn.jsdelivr.net/gh/highlightjs/highlight.js@${version}/src"

    info "Downloading highlight.js v${version}..."
    mkdir -p "$STAGE/hljs"

    info "  Core: highlight.min.js"
    fetch "${build}/highlight.min.js" "$STAGE/hljs/hljs.${version}.js"

    info "  Theme: dark.min.css"
    fetch "${build}/styles/dark.min.css" "$STAGE/hljs/hljs.dark.${version}.css"

    # From src/styles/, not build/styles/: the build tree prepends layout rules
    # (`pre code.hljs{display:block;overflow-x:auto;padding:1em}` and
    # `code.hljs{padding:3px 5px}`) on top of the colour theme. mbr vendors the
    # colour-only source file and leaves pre/code layout to Pico + theme.css.
    info "  Theme: atom-one-dark.css (source tree, colours only)"
    fetch "${src}/styles/atom-one-dark.css" "$STAGE/hljs/hljs.atom-one-dark.${version}.css"

    local lang
    for lang in "${HLJS_LANGUAGES[@]}"; do
        info "  Language: ${lang}"
        fetch "${build}/languages/${lang}.min.js" "$STAGE/hljs/hljs.lang.${lang}.${version}.js"
    done

    # Everything arrived: retire the old version and repoint src/.
    if [[ -n "$old" && "$old" != "$version" ]]; then
        rm -f "$TEMPLATES_DIR"/hljs."$old".js \
              "$TEMPLATES_DIR"/hljs.dark."$old".css \
              "$TEMPLATES_DIR"/hljs.atom-one-dark."$old".css
        for lang in "${HLJS_LANGUAGES[@]}"; do
            rm -f "$TEMPLATES_DIR/hljs.lang.${lang}.${old}.js"
        done
        rename_refs "hljs.${old}.js" "hljs.${version}.js"
        rename_refs "hljs.dark.${old}.css" "hljs.dark.${version}.css"
        rename_refs "hljs.atom-one-dark.${old}.css" "hljs.atom-one-dark.${version}.css"
        for lang in "${HLJS_LANGUAGES[@]}"; do
            rename_refs "hljs.lang.${lang}.${old}.js" "hljs.lang.${lang}.${version}.js"
        done
    fi
    mv "$STAGE"/hljs/* "$TEMPLATES_DIR/"

    # mbr-hljs.ts pins the same version for its CDN fallback (languages outside
    # HLJS_LANGUAGES are fetched from cdn-release at runtime). Nothing makes
    # that constant fail if it drifts — it just silently serves grammars from a
    # different release than the embedded core — so bump it here.
    if [[ -f "$HLJS_COMPONENT" ]]; then
        sed_inplace "s/const HLJS_VERSION = '[^']*'/const HLJS_VERSION = '${version}'/" "$HLJS_COMPONENT"
    fi

    info "highlight.js v${version} installed; src/ and mbr-hljs.ts references updated."
    warn "components/src/mbr-hljs.ts changed — rerun 'cd components && bun run build'"
    warn "to regenerate templates/components-js/, which embeds the version string."
    echo
}

# --- mermaid -------------------------------------------------------------

download_mermaid() {
    local version="$1"
    local old; old=$(current_mermaid)

    info "Downloading mermaid.js v${version}..."
    mkdir -p "$STAGE/mermaid"
    fetch "https://cdn.jsdelivr.net/npm/mermaid@${version}/dist/mermaid.min.js" \
          "$STAGE/mermaid/mermaid.${version}.min.js"

    if [[ -n "$old" && "$old" != "$version" ]]; then
        rm -f "$TEMPLATES_DIR/mermaid.${old}.min.js"
        rename_refs "mermaid.${old}.min.js" "mermaid.${version}.min.js"
    fi
    mv "$STAGE"/mermaid/* "$TEMPLATES_DIR/"

    info "mermaid.js v${version} installed; src/ references updated."
    echo
}

# --- KaTeX ---------------------------------------------------------------

# The font set is read out of the downloaded CSS rather than hardcoded, because
# it is not stable across KaTeX releases and a stale list fails in the worst
# way: a missing font is a silent @font-face fallback, not an error. mbr serves
# woff2 + woff and ignores the ttf fallbacks KaTeX also references.
download_katex() {
    local version="$1"
    local old; old=$(current_katex)
    local base="https://cdn.jsdelivr.net/npm/katex@${version}/dist"

    info "Downloading KaTeX v${version}..."
    mkdir -p "$STAGE/katex/fonts"

    info "  Core: katex.min.css"
    fetch "${base}/katex.min.css" "$STAGE/katex/katex.${version}.min.css"
    info "  Core: katex.min.js"
    fetch "${base}/katex.min.js" "$STAGE/katex/katex.${version}.min.js"

    local fonts
    fonts=$(grep -oE 'fonts/KaTeX_[A-Za-z0-9_-]+\.woff2?' "$STAGE/katex/katex.${version}.min.css" \
        | sed 's|fonts/||' | sort -u)
    [[ -n "$fonts" ]] || error "No woff/woff2 fonts referenced by katex.min.css — refusing to continue"

    local font
    while IFS= read -r font; do
        info "  Font: ${font}"
        fetch "${base}/fonts/${font}" "$STAGE/katex/fonts/${font}"
    done <<< "$fonts"

    # Font filenames are unversioned, so an upstream change adds or drops files
    # rather than renaming them — and embedded_katex.rs names each one
    # individually. Report the delta; it cannot be fixed by sed.
    local added removed
    added=$(comm -23 <(echo "$fonts") <(ls "$TEMPLATES_DIR/katex-fonts" 2>/dev/null | sort) || true)
    removed=$(comm -13 <(echo "$fonts") <(ls "$TEMPLATES_DIR/katex-fonts" 2>/dev/null | sort) || true)

    if [[ -n "$old" && "$old" != "$version" ]]; then
        rm -f "$TEMPLATES_DIR/katex.${old}.min.css" "$TEMPLATES_DIR/katex.${old}.min.js"
        rename_refs "katex.${old}.min.css" "katex.${version}.min.css"
        rename_refs "katex.${old}.min.js" "katex.${version}.min.js"
    fi
    mkdir -p "$TEMPLATES_DIR/katex-fonts"
    if [[ -n "$removed" ]]; then
        while IFS= read -r font; do
            [[ -n "$font" ]] && rm -f "$TEMPLATES_DIR/katex-fonts/$font"
        done <<< "$removed"
    fi
    mv "$STAGE"/katex/fonts/* "$TEMPLATES_DIR/katex-fonts/"
    mv "$STAGE"/katex/*.min.* "$TEMPLATES_DIR/"

    info "KaTeX v${version} installed; src/ references updated."
    if [[ -n "$added" || -n "$removed" ]]; then
        echo
        warn "The KaTeX font set changed. src/embedded_katex.rs lists every font"
        warn "by name and asserts KATEX_FILES.len(); edit it by hand:"
        [[ -n "$added" ]]   && { echo "  added:"; echo "$added" | sed 's/^/    + /'; }
        [[ -n "$removed" ]] && { echo "  removed:"; echo "$removed" | sed 's/^/    - /'; }
    fi
    echo
}

# --- reveal.js -----------------------------------------------------------

# Core and the notes plugin only. The three vendored themes
# (reveal.theme.{black,white,blank}.*.css) are mbr-modified — the Source Sans
# Pro @import was stripped so a 404 cannot trip a link onerror, global variables
# were removed so they inherit from Pico via theme.css, and `blank` has no
# upstream counterpart at all. Re-downloading them would silently revert those
# edits, so a theme refresh stays a deliberate, hand-diffed job.
download_reveal() {
    local version="$1"
    local old; old=$(current_reveal)
    local base="https://cdn.jsdelivr.net/npm/reveal.js@${version}"

    info "Downloading reveal.js v${version}..."
    mkdir -p "$STAGE/reveal"

    info "  Core: dist/reveal.js"
    fetch "${base}/dist/reveal.js" "$STAGE/reveal/reveal.${version}.js"
    info "  Core: dist/reveal.css"
    fetch "${base}/dist/reveal.css" "$STAGE/reveal/reveal.${version}.css"

    # The notes plugin moved in 6.0.0: plugin/notes/notes.js -> dist/plugin/notes.js.
    info "  Plugin: speaker notes"
    if [[ "${version%%.*}" -ge 6 ]]; then
        fetch "${base}/dist/plugin/notes.js" "$STAGE/reveal/reveal.notes.${version}.js"
    else
        fetch "${base}/plugin/notes/notes.js" "$STAGE/reveal/reveal.notes.${version}.js"
    fi

    if [[ -n "$old" && "$old" != "$version" ]]; then
        rm -f "$TEMPLATES_DIR/reveal.${old}.js" \
              "$TEMPLATES_DIR/reveal.${old}.css" \
              "$TEMPLATES_DIR/reveal.notes.${old}.js"
        rename_refs "reveal.${old}.js" "reveal.${version}.js"
        rename_refs "reveal.${old}.css" "reveal.${version}.css"
        rename_refs "reveal.notes.${old}.js" "reveal.notes.${version}.js"
    fi
    mv "$STAGE"/reveal/* "$TEMPLATES_DIR/"

    info "reveal.js v${version} installed; src/ references updated."
    echo
    warn "Themes were NOT updated — they carry mbr modifications:"
    ls "$TEMPLATES_DIR"/reveal.theme.*.css 2>/dev/null | sed 's|.*/|    |'
    warn "Re-diff them against ${base}/dist/theme/ by hand if the core bump needs it."
    echo
}

# --- Pico CSS ------------------------------------------------------------

# Pico's filenames carry no version, so there is nothing to rename and nothing
# in src/ to repoint — the whole update is an in-place overwrite of 42 files.
download_pico() {
    local version="$1"
    local old; old=$(current_pico)
    local base="https://cdn.jsdelivr.net/npm/@picocss/pico@${version}/css"

    info "Downloading Pico CSS v${version}..."
    mkdir -p "$STAGE/pico"

    local names=(pico.css pico.min.css pico.fluid.classless.css pico.fluid.classless.min.css)
    local color
    for color in "${PICO_COLORS[@]}"; do
        names+=("pico.${color}.min.css" "pico.fluid.classless.${color}.min.css")
    done

    local name
    for name in "${names[@]}"; do
        info "  ${name}"
        fetch "${base}/${name}" "$STAGE/pico/${name}"
    done

    mkdir -p "$TEMPLATES_DIR/pico-main"
    mv "$STAGE"/pico/* "$TEMPLATES_DIR/pico-main/"

    info "Pico CSS v${old:-?} -> v${version} installed (filenames unversioned; no src/ changes)."
    echo
}

# --- argument parsing ----------------------------------------------------

HLJS_VERSION=""
MERMAID_VERSION=""
KATEX_VERSION=""
REVEAL_VERSION=""
PICO_VERSION=""
UPDATE_ALL=false

usage() {
    cat <<'EOF'
Usage: update-assets.sh [--all] [--hljs V] [--mermaid V] [--katex V] [--reveal V] [--pico V]

With no arguments, reports the vendored version of each asset alongside the
latest published upstream version.

Options:
  --all              Update every asset to its latest published version
  --hljs VERSION     highlight.js       (VERSION may be "latest")
  --mermaid VERSION  mermaid
  --katex VERSION    KaTeX, including its font files
  --reveal VERSION   reveal.js core + notes plugin (themes are mbr-modified; skipped)
  --pico VERSION     Pico CSS
  -h, --help         Show this help message

Examples:
  update-assets.sh                              # what is vendored, what is available
  update-assets.sh --all
  update-assets.sh --hljs 11.11.2 --mermaid 11.16.1
EOF
}

while [[ $# -gt 0 ]]; do
    case $1 in
        --all)     UPDATE_ALL=true; shift ;;
        --hljs)    HLJS_VERSION="${2:?--hljs needs a version}"; shift 2 ;;
        --mermaid) MERMAID_VERSION="${2:?--mermaid needs a version}"; shift 2 ;;
        --katex)   KATEX_VERSION="${2:?--katex needs a version}"; shift 2 ;;
        --reveal)  REVEAL_VERSION="${2:?--reveal needs a version}"; shift 2 ;;
        --pico)    PICO_VERSION="${2:?--pico needs a version}"; shift 2 ;;
        -h|--help) usage; exit 0 ;;
        *)         error "Unknown option: $1" ;;
    esac
done

if $UPDATE_ALL; then
    HLJS_VERSION="${HLJS_VERSION:-latest}"
    MERMAID_VERSION="${MERMAID_VERSION:-latest}"
    KATEX_VERSION="${KATEX_VERSION:-latest}"
    REVEAL_VERSION="${REVEAL_VERSION:-latest}"
    PICO_VERSION="${PICO_VERSION:-latest}"
fi

# Resolve "latest" against the registry.
resolve() {
    local pkg="$1" requested="$2"
    if [[ "$requested" == "latest" ]]; then
        local v; v=$(npm_latest "$pkg")
        [[ -n "$v" ]] || error "Could not resolve the latest version of $pkg"
        echo "$v"
    else
        echo "$requested"
    fi
}

if [[ -z "$HLJS_VERSION$MERMAID_VERSION$KATEX_VERSION$REVEAL_VERSION$PICO_VERSION" ]]; then
    report_versions true
    echo "To update, pass versions explicitly or use --all:"
    echo "  $0 --all"
    echo "  $0 --hljs 11.11.2 --mermaid 11.16.1"
    exit 0
fi

if [[ -n "$HLJS_VERSION" ]];    then download_hljs    "$(resolve highlight.js "$HLJS_VERSION")"; fi
if [[ -n "$MERMAID_VERSION" ]]; then download_mermaid "$(resolve mermaid "$MERMAID_VERSION")"; fi
if [[ -n "$KATEX_VERSION" ]];   then download_katex   "$(resolve katex "$KATEX_VERSION")"; fi
if [[ -n "$REVEAL_VERSION" ]];  then download_reveal  "$(resolve reveal.js "$REVEAL_VERSION")"; fi
if [[ -n "$PICO_VERSION" ]];    then download_pico    "$(resolve @picocss/pico "$PICO_VERSION")"; fi

report_versions false

info "Done! Now:"
echo "  1. Review the src/*.rs diff this script produced"
echo "  2. git add templates/   # REQUIRED before 'nix build'"
echo "  3. cargo build   # confirms every include_bytes! path still resolves"
echo "  4. cargo test"
echo
# Step 2 is not optional bookkeeping. A version bump writes *new* filenames,
# and `nix build` on a flake copies only git-tracked files into the sandbox —
# so untracked assets are invisible there while cargo, which reads the working
# tree, compiles fine. The failure lands far from the cause: 40+ "couldn't read
# ../templates/hljs.lang.<x>.<new>.js" errors from include_bytes!, after the
# whole dependency tree has rebuilt.
warn "New asset files are untracked. 'nix build' will fail with include_bytes!"
warn "errors until you 'git add templates/' — cargo build will pass either way."
