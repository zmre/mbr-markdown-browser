#!/usr/bin/env bash
#
# CI's explicit pushes to the zmre binary cache.
#
# Every cachix-action block in ci.yml is pull-only (`skipPush: true`): the
# action's post-job daemon would otherwise upload EVERYTHING a job builds — test
# runs, clippy output, per-commit binaries — and on the 5 GB free plan (LRU,
# shared with ledgeline) that churn is what evicts the things worth caching. So
# what CI pushes is named here, by flake attr:
#
#   push <attr>...
#       Push each attr's output, but only if this job put it in the local store
#       (built or substituted). Evaluating `outPath` builds nothing, so a layer
#       whose own build failed is skipped rather than attempted a second time.
#       Used for the crane dependency layers and mbr-components, on PRs too: a
#       Cargo.lock bump lands on a PR first, and that PR's runs are the cold ones.
#
#   push-and-pin <pin-name>=<attr>...
#       Build (normally already done), push, and pin as `<pin-name>-<system>`,
#       keeping 2 revisions. Pins are exempt from Cachix's LRU garbage
#       collection; see the "Push and pin consumer artifacts" step in ci.yml.
#
# Both do nothing (exit 0) without CACHIX_AUTH_TOKEN: secrets are absent on fork
# PRs, and a cache push is none of an outside contributor's business.
set -euo pipefail

if [ -z "${CACHIX_AUTH_TOKEN:-}" ]; then
  echo "No CACHIX_AUTH_TOKEN (fork or unset) - skipping cache push."
  exit 0
fi

cmd="${1:-}"
shift || true

case "$cmd" in
  push)
    for attr in "$@"; do
      path=$(nix eval --raw ".#$attr.outPath")
      if ! nix path-info "$path" > /dev/null 2>&1; then
        echo "$attr ($path) is not in the local store - nothing to push."
        continue
      fi
      echo "==> push $attr -> $path"
      cachix push zmre "$path"
    done
    ;;
  push-and-pin)
    # Derived rather than hardcoded: CI runs this on several systems, and
    # runner architectures change.
    sys=$(nix eval --raw --impure --expr builtins.currentSystem)
    for spec in "$@"; do
      name="${spec%%=*}"
      attr="${spec#*=}"
      path=$(nix build --no-link --print-out-paths ".#$attr")
      echo "==> $name-$sys -> $path"
      cachix push zmre "$path"
      cachix pin zmre "$name-$sys" "$path" --keep-revisions 2
    done
    ;;
  *)
    echo "usage: $0 push <attr>... | push-and-pin <pin-name>=<attr>..." >&2
    exit 2
    ;;
esac
