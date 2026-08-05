#!/usr/bin/env bash
#
# Manual push to the zmre binary cache, for the case where CI has not (yet)
# built something you want other machines to substitute.
#
# Keep the pinned set in sync with the "Push and pin consumer artifacts" step in
# .github/workflows/ci.yml. Pinning is what keeps the consumer artifacts alive
# through Cachix's least-recently-used garbage collection: they are the coldest
# paths in the cache (downloaded only when a stranger runs `nix run`), so a push
# without a pin leaves them first in line for eviction. See the long comment on
# that CI step for the full reasoning.
set -euo pipefail

sys=$(nix eval --raw --impure --expr builtins.currentSystem)

# Dependency layer: pushed so other machines skip the build, deliberately NOT
# pinned. It churns on every lockfile bump and CI keeps it warm by itself.
nix build --no-link --print-out-paths .#mbr-components | cachix push zmre

push_and_pin() {
  local name="$1" attr="$2" path
  path=$(nix build --no-link --print-out-paths ".#$attr")
  echo "==> $name-$sys -> $path"
  cachix push zmre "$path"
  cachix pin zmre "$name-$sys" "$path" --keep-revisions 2
}

# `.#default` rather than `.#mbr`: it is what `nix run`, `nix build github:...`
# and `nix profile install` all resolve to.
push_and_pin mbr default
# Fixed-version and expensive (30-60 min from source), so pinned even though
# they are strictly build dependencies. x264 is pinned in its own right because
# Cachix does not document pins as extending to the closure.
push_and_pin ffmpeg-minimal-static ffmpegMinimalStatic
push_and_pin x264-static x264Static
