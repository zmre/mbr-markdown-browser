#!/usr/bin/env bash
# Submit one artifact to Apple's notary service, wait for the verdict, and fail
# loudly — with the full notary log — if it is anything but `Accepted`.
#
#   Usage: scripts/notarize.sh <artifact.zip|.dmg|.pkg>
#
# Required in the environment (see docs/releasing.md for how to produce them):
#   APPLE_API_KEY_P8      base64 of the App Store Connect AuthKey_XXXX.p8
#   APPLE_API_KEY_ID      the ten-character Key ID
#   APPLE_API_ISSUER_ID   the team's issuer UUID
#
# WHY THIS IS A SCRIPT AND NOT INLINE WORKFLOW/SHELL BLOCKS: a release
# notarizes twice — the .app, then the .dmg — and the verdict check below is
# subtle enough that having two copies of it is how one of them quietly loses
# the check.
#
# WHY THE STATUS LINE IS PARSED RATHER THAN TRUSTING THE EXIT CODE:
# `notarytool submit --wait` reports a completed *submission*, which is not the
# same as an accepted one — it has historically exited 0 while printing
# `status: Invalid`. Trusting `$?` alone therefore risks stapling nothing onto a
# rejected build and shipping it. Both signals are checked here, and the run
# fails unless BOTH say yes.
set -euo pipefail

artifact="${1:?usage: notarize.sh <artifact.zip|.dmg|.pkg>}"
: "${APPLE_API_KEY_P8:?APPLE_API_KEY_P8 is not set}"
: "${APPLE_API_KEY_ID:?APPLE_API_KEY_ID is not set}"
: "${APPLE_API_ISSUER_ID:?APPLE_API_ISSUER_ID is not set}"

[ -e "$artifact" ] || { echo "notarize: no such artifact: $artifact" >&2; exit 1; }

workdir=$(mktemp -d)
# The private key is written to disk because notarytool takes a path, not bytes.
# The trap is what keeps it from outliving this script on a reusable runner.
trap 'rm -rf "$workdir"' EXIT

key="$workdir/AuthKey.p8"
printf '%s' "$APPLE_API_KEY_P8" | base64 --decode > "$key"
chmod 600 "$key"

log="$workdir/submit.txt"

echo "notarize: submitting $(basename "$artifact")…"
# Deliberately not fatal on its own — see the header. The output is captured
# either way so the reason can be printed on the failure path.
set +e
xcrun notarytool submit "$artifact" \
  --key "$key" \
  --key-id "$APPLE_API_KEY_ID" \
  --issuer "$APPLE_API_ISSUER_ID" \
  --wait --timeout 30m > "$log" 2>&1
rc=$?
set -e

cat "$log"

# notarytool prints an indented `key: value` block. `^[[:space:]]*status:`
# deliberately does NOT match the progress line `Current status: Accepted....`,
# which appears while still polling and is not a verdict.
status=$(awk '/^[[:space:]]*status:/ { print $2; exit }' "$log")
id=$(awk '/^[[:space:]]*id:/ { print $2; exit }' "$log")

if [ "$rc" -eq 0 ] && [ "${status:-}" = "Accepted" ]; then
  echo "notarize: $(basename "$artifact") accepted (submission ${id:-unknown})"
  exit 0
fi

echo "ERROR: notarization of $(basename "$artifact") did not succeed." >&2
echo "  notarytool exit status: $rc" >&2
echo "  verdict:                ${status:-<none reported>}" >&2
if [ -n "${id:-}" ]; then
  echo "--- notary log for submission $id ---" >&2
  # `|| true`: the log fetch is diagnostics. If it fails too, the verdict above
  # is still the answer and must not be replaced by a log-fetch error.
  xcrun notarytool log "$id" \
    --key "$key" \
    --key-id "$APPLE_API_KEY_ID" \
    --issuer "$APPLE_API_ISSUER_ID" >&2 || true
else
  echo "  No submission id was reported — the upload itself failed." >&2
fi
exit 1
