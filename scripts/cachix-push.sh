#!/usr/bin/env bash

nix build --no-link --print-out-paths .#mbr-components .#mbr | cachix push zmre

