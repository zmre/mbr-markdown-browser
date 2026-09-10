#!/usr/bin/env bash
#
# Dev-only tool: unregisters stale macOS LaunchServices entries for a
# Nix-built MBR.app.
#
# Every `nix build`/`nix profile install`/flake update places MBR.app at a
# new immutable /nix/store/<hash>-mbr-<version>/Applications/MBR.app path.
# LaunchServices never forgets a path it has seen, even after the store path
# is garbage collected, so these pile up as duplicate "MBR" entries in
# Finder's "Open With" menu. This unregisters everything for the app's
# bundle id except the canonical install path, then compacts the database.
#
# Not part of the build or release process; run manually when Open With
# gets cluttered.

set -euo pipefail

APP_PATH="${1:-/Applications/Nix Apps/MBR.app}"
LSREGISTER="/System/Library/Frameworks/CoreServices.framework/Versions/A/Frameworks/LaunchServices.framework/Versions/A/Support/lsregister"

if [[ ! -x "$LSREGISTER" ]]; then
  echo "error: lsregister not found at $LSREGISTER (macOS version changed its layout?)" >&2
  exit 1
fi

if [[ ! -d "$APP_PATH" ]]; then
  echo "error: $APP_PATH does not exist" >&2
  exit 1
fi

BUNDLE_ID=$(defaults read "$APP_PATH/Contents/Info" CFBundleIdentifier)
echo "Canonical app: $APP_PATH"
echo "Bundle id: $BUNDLE_ID"

mapfile -t stale_paths < <(
  "$LSREGISTER" -dump 2>/dev/null | awk -v id="$BUNDLE_ID" -v canonical="$APP_PATH" '
    /^path:/ {
      path = $0
      sub(/^path:[ \t]+/, "", path)
      sub(/ \(0x[0-9a-f]+\)$/, "", path)
    }
    /^identifier:/ {
      ident = $0
      sub(/^identifier:[ \t]+/, "", ident)
    }
    /^--[ \t]*$/ {
      if (ident == id && path ~ /\.app$/ && path != canonical) print path
      path = ""; ident = ""
    }
  '
)

if [[ ${#stale_paths[@]} -eq 0 ]]; then
  echo "No stale entries found."
else
  echo "Unregistering ${#stale_paths[@]} stale entries:"
  for p in "${stale_paths[@]}"; do
    echo "  $p"
    "$LSREGISTER" -u "$p" || true
  done
fi

echo "Garbage collecting LaunchServices database..."
"$LSREGISTER" -gc

echo "Restarting Finder..."
killall Finder || true

echo "Done. Remaining entries for $BUNDLE_ID:"
"$LSREGISTER" -dump 2>/dev/null | grep -B6 "identifier:.*$BUNDLE_ID" | grep "^path:" || true
