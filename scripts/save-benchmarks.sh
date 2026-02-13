#!/usr/bin/env bash
# Save Criterion benchmark results to docs/benchmarks/data.json
#
# Usage:
#   ./scripts/save-benchmarks.sh <version>                  # Run benchmarks and save
#   ./scripts/save-benchmarks.sh <version> --no-run         # Save from existing 'new' results
#   ./scripts/save-benchmarks.sh <version> --no-run --from-baseline <name>  # Save from named baseline
#
# Examples:
#   ./scripts/save-benchmarks.sh 0.5.0                      # Run benchmarks, save as v0.5.0 baseline
#   ./scripts/save-benchmarks.sh 0.4.2 --no-run --from-baseline v0.4.2   # Import existing baseline

set -euo pipefail

VERSION=""
NO_RUN=false
FROM_BASELINE=""

# Parse arguments
while [[ $# -gt 0 ]]; do
    case "$1" in
        --no-run)
            NO_RUN=true
            shift
            ;;
        --from-baseline)
            FROM_BASELINE="$2"
            shift 2
            ;;
        -*)
            echo "Unknown option: $1" >&2
            exit 1
            ;;
        *)
            if [[ -z "$VERSION" ]]; then
                VERSION="$1"
            else
                echo "Error: unexpected argument '$1'" >&2
                exit 1
            fi
            shift
            ;;
    esac
done

if [[ -z "$VERSION" ]]; then
    echo "Usage: $0 <version> [--no-run] [--from-baseline <name>]"
    exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
DATA_FILE="$PROJECT_DIR/docs/benchmarks/data.json"
CRITERION_DIR="$PROJECT_DIR/target/criterion"
REPO_URL="https://github.com/zmre/mbr-markdown-browser"

# Ensure jq is available
if ! command -v jq &>/dev/null; then
    echo "jq not found. Trying nix..."
    if command -v nix &>/dev/null; then
        JQ="nix shell nixpkgs#jq -c jq"
    else
        echo "Error: jq is required. Install via nix or your package manager." >&2
        exit 1
    fi
else
    JQ="jq"
fi

# Run benchmarks unless --no-run
if [[ "$NO_RUN" == "false" ]]; then
    echo "Running benchmarks (saving baseline as v${VERSION})..."
    cd "$PROJECT_DIR"
    cargo bench --no-default-features --benches -- --save-baseline "v${VERSION}"
    echo "Benchmarks complete."
fi

# Determine which criterion subdirectory to read
if [[ -n "$FROM_BASELINE" ]]; then
    BASELINE_DIR="$FROM_BASELINE"
else
    BASELINE_DIR="new"
fi

# Verify criterion data exists
if [[ ! -d "$CRITERION_DIR" ]]; then
    echo "Error: $CRITERION_DIR not found. Run benchmarks first." >&2
    exit 1
fi

# Get git commit info
COMMIT_HASH=$(git -C "$PROJECT_DIR" log -1 --format='%H')
COMMIT_MESSAGE=$(git -C "$PROJECT_DIR" log -1 --format='%s')
COMMIT_TIMESTAMP=$(git -C "$PROJECT_DIR" log -1 --format='%aI')
COMMIT_AUTHOR=$(git -C "$PROJECT_DIR" log -1 --format='%an')
COMMIT_EMAIL=$(git -C "$PROJECT_DIR" log -1 --format='%ae')

# If reading from a named baseline, try to get commit info from the tag
if [[ -n "$FROM_BASELINE" ]]; then
    TAG_NAME="$FROM_BASELINE"
    if git -C "$PROJECT_DIR" rev-parse "$TAG_NAME" &>/dev/null; then
        COMMIT_HASH=$(git -C "$PROJECT_DIR" log -1 --format='%H' "$TAG_NAME")
        COMMIT_MESSAGE=$(git -C "$PROJECT_DIR" log -1 --format='%s' "$TAG_NAME")
        COMMIT_TIMESTAMP=$(git -C "$PROJECT_DIR" log -1 --format='%aI' "$TAG_NAME")
        COMMIT_AUTHOR=$(git -C "$PROJECT_DIR" log -1 --format='%an' "$TAG_NAME")
        COMMIT_EMAIL=$(git -C "$PROJECT_DIR" log -1 --format='%ae' "$TAG_NAME")
    fi
fi

DATE_MS=$(date +%s)000

echo "Collecting benchmark results from '$BASELINE_DIR'..."

# Collect all benchmark results
BENCHES_JSON="[]"
BENCH_COUNT=0

while IFS= read -r estimates_file; do
    # Extract the benchmark name from the path
    # e.g., target/criterion/markdown_render/render/large/new/estimates.json
    #   -> markdown_render/render/large
    rel_path="${estimates_file#"$CRITERION_DIR/"}"
    # Remove the baseline dir and estimates.json suffix
    bench_name="${rel_path%/"$BASELINE_DIR"/estimates.json}"

    # Skip the 'report' directory
    if [[ "$bench_name" == "report" || "$bench_name" == report/* ]]; then
        continue
    fi

    # Extract mean point estimate (nanoseconds) and confidence interval
    mean_ns=$($JQ '.mean.point_estimate' "$estimates_file")
    ci_lower=$($JQ '.mean.confidence_interval.lower_bound' "$estimates_file")
    ci_upper=$($JQ '.mean.confidence_interval.upper_bound' "$estimates_file")

    # Convert to microseconds and round to 2 decimal places
    value=$($JQ -n "$mean_ns / 1000 | . * 100 | floor / 100")
    range_width=$($JQ -n "($ci_upper - $ci_lower) / 1000 | . * 100 | floor / 100")

    # Build bench entry
    bench_entry=$($JQ -n \
        --arg name "$bench_name" \
        --argjson value "$value" \
        --arg unit "us" \
        --arg range "+/- $range_width" \
        '{name: $name, value: $value, unit: $unit, range: $range}')

    BENCHES_JSON=$($JQ --argjson entry "$bench_entry" '. + [$entry]' <<< "$BENCHES_JSON")
    BENCH_COUNT=$((BENCH_COUNT + 1))
done < <(find "$CRITERION_DIR" -path "*/${BASELINE_DIR}/estimates.json" -type f | sort)

if [[ "$BENCH_COUNT" -eq 0 ]]; then
    echo "Error: No benchmark results found in '$BASELINE_DIR'." >&2
    echo "Available baselines:" >&2
    find "$CRITERION_DIR" -name "estimates.json" -type f | sed "s|$CRITERION_DIR/||" | sed 's|/estimates.json||' | awk -F/ '{print $NF}' | sort -u >&2
    exit 1
fi

echo "Found $BENCH_COUNT benchmarks."

# Build the new entry
NEW_ENTRY=$($JQ -n \
    --arg id "$COMMIT_HASH" \
    --arg message "$COMMIT_MESSAGE" \
    --arg timestamp "$COMMIT_TIMESTAMP" \
    --arg url "$REPO_URL/commit/$COMMIT_HASH" \
    --arg author_name "$COMMIT_AUTHOR" \
    --arg author_email "$COMMIT_EMAIL" \
    --argjson date "$DATE_MS" \
    --arg tool "customSmallerIsBetter" \
    --argjson benches "$BENCHES_JSON" \
    --arg version "$VERSION" \
    '{
        commit: {
            author: { name: $author_name, email: $author_email, username: "" },
            committer: { name: $author_name, email: $author_email, username: "" },
            id: $id,
            message: $message,
            timestamp: $timestamp,
            url: $url
        },
        date: $date,
        tool: $tool,
        benches: $benches,
        version: $version
    }')

# Load existing data or create new structure
mkdir -p "$(dirname "$DATA_FILE")"

if [[ -f "$DATA_FILE" ]]; then
    EXISTING_JSON=$(cat "$DATA_FILE")
else
    EXISTING_JSON='{"lastUpdate": 0, "repoUrl": "'"$REPO_URL"'", "entries": {"mbr": []}}'
fi

# Append the new entry
UPDATED_JSON=$(echo "$EXISTING_JSON" | $JQ \
    --argjson entry "$NEW_ENTRY" \
    --argjson now "$DATE_MS" \
    '.lastUpdate = $now | .entries.mbr += [$entry]')

echo "$UPDATED_JSON" | $JQ '.' > "$DATA_FILE"

echo "Updated $DATA_FILE with v${VERSION} ($BENCH_COUNT benchmarks)"
echo "Commit: ${COMMIT_HASH:0:7} - $COMMIT_MESSAGE"
