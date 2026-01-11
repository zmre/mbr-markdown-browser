---
title: Installation
description: Install mbr on your system
---

# Installation

mbr can be installed via Nix (recommended), Cargo, or from binary releases.

## Using Nix (Recommended)

Nix provides reproducible builds and includes all dependencies.

### Run Without Installing

```bash
# Run directly from GitHub
nix run github:zmre/mbr -- -g /path/to/notes
```

### Build and Install

```bash
# Build the binary
nix build github:zmre/mbr

# Run from build output
./result/bin/mbr -s /path/to/notes
```

### Add to Your Flake

```nix
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    mbr.url = "github:zmre/mbr";
  };

  outputs = { self, nixpkgs, mbr }: {
    # Use mbr.packages.${system}.default in your configuration
  };
}
```

## Using Cargo

If you have Rust installed, you can build from source:

```bash
cargo install --git https://github.com/zmre/mbr
```

### Prerequisites

- Rust 1.75 or later
- A C compiler (for some dependencies)

## Binary Releases

Pre-built binaries are available on the [GitHub Releases](https://github.com/zmre/mbr-markdown-browser/releases) page.

### macOS App Bundle

The macOS release includes `MBR.app`, a native application bundle with:

- Application icon
- Native menu bar integration
- QuickLook extension for Finder previews

To install, move `MBR.app` to your Applications folder.

## Verify Installation

```bash
# Check version
mbr --version

# Display help
mbr --help

# Test with a markdown file
mbr -s README.md
```

## Next Steps

- [Quick Start Guide](quickstart/) - Get productive in 5 minutes
- [Modes of Operation](../modes/) - Learn about GUI, Server, and Build modes
