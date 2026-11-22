#!/usr/bin/env bash
#
# CI: `mbr --pdf` end to end with the real platform webview, on the `./result`
# of `nix build .#mbr`. Shared by ci.yml's Linux `build` legs and the `macos`
# job.
#
# `.#tests` runs in the Nix sandbox, where no webview can open, so this is the
# only place the built macOS and Linux binaries actually print a PDF. Linux
# runs it under Xvfb: a CI check of the WebKitGTK backend, not a supported
# headless mode (without a display mbr refuses, which ci.yml's next step pins).
# Fails when no PDF is written, when the file is not a PDF or is implausibly
# small, or when the page never reported itself ready (the exporter then prints
# whatever was on screen after its timeout, which would hide a broken front
# end).
set -euo pipefail
out="$RUNNER_TEMP/smoke.pdf"
log="$RUNNER_TEMP/smoke.stderr"
if [ "$RUNNER_OS" = "macOS" ]; then
  # The bundle's executable, as users run it (Contents/MacOS/mbr).
  ./result/Applications/MBR.app/Contents/MacOS/mbr \
    --pdf tests/fixtures/pdf-export/note.md --output "$out" 2> "$log" \
    || { cat "$log"; echo "::error::mbr --pdf failed on macOS"; exit 1; }
else
  command -v xvfb-run > /dev/null || {
    sudo apt-get update && sudo apt-get install -y xvfb
  }
  # Ubuntu 24.04 restricts unprivileged user namespaces through
  # AppArmor, and WebKitGTK's bubblewrap sandbox needs them; a desktop
  # without the restriction is what users run. CI-only.
  if [ -e /proc/sys/kernel/apparmor_restrict_unprivileged_userns ]; then
    sudo sysctl -w kernel.apparmor_restrict_unprivileged_userns=0
  fi
  xvfb-run -a ./result/bin/mbr \
    --pdf tests/fixtures/pdf-export/note.md --output "$out" 2> "$log" \
    || { cat "$log"; echo "::error::mbr --pdf failed on Linux (Xvfb)"; exit 1; }
fi
cat "$log"
[ -f "$out" ] || { echo "::error::no PDF was written to $out"; exit 1; }
magic=$(head -c 5 "$out")
[ "$magic" = "%PDF-" ] || { echo "::error::$out does not start with %PDF- (got '$magic')"; exit 1; }
size=$(wc -c < "$out" | tr -d ' ')
[ "$size" -gt 10000 ] || { echo "::error::$out is only $size bytes"; exit 1; }
if grep -q 'did not finish rendering' "$log"; then
  echo "::error::the export page never reported ready (front end missing from the build?)"
  exit 1
fi
echo "OK: $out is a PDF of $size bytes"
