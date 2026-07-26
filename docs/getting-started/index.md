---
title: Getting Started - Installation
description: Install mbr on your system
order: 1
---
# Installation

mbr can be installed via Nix (recommended), Cargo, or from binary releases.  More can be added so file an [issue](https://github.com/zmre/mbr-markdown-browser/issues) if you have a request.

## Using Nix (Recommended)

Nix provides reproducible builds and includes all dependencies.

### Run Without Installing

```bash
# Run directly from GitHub
nix run github:zmre/mbr -- -g /path/to/notes
```

### Build from Source

```bash
# Build the binary from inside the source dir
nix build 

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

* Rust 1.75 or later

* A C compiler (for some dependencies)

## Binary Releases

Pre-built binaries are available on the [GitHub Releases](https://github.com/zmre/mbr-markdown-browser/releases) page.

| Platform | File | Notes |
|----------|------|-------|
| macOS | `mbr-macos-universal.dmg` | Universal app, Intel and Apple Silicon |
| macOS | `mbr-macos-arm64.tar.gz`, `mbr-macos-x86_64.tar.gz` | Single-architecture app bundles |
| macOS | `mbr-cli-macos-*.tar.gz` | Command-line binary only |
| Linux | `mbr-linux-x86_64.tar.gz` | Command-line binary |
| Windows | `mbr-windows-x86_64.zip` | See limitations below |

### macOS App Bundle

The macOS release includes `MBR.app`, a native application bundle with:

* Application icon

* Native menu bar integration

* QuickLook extension for Finder previews

The easiest install is the DMG: open it and drag `MBR.app` to the Applications
folder. You can also extract a `.tar.gz` and move `MBR.app` yourself.

These builds are ad-hoc signed but not notarized, so macOS blocks the first
launch. Open **System Settings › Privacy & Security** and click **Open Anyway**,
or clear the quarantine flag from a terminal:

```bash
xattr -dr com.apple.quarantine /Applications/MBR.app
```

Extracting the tarball with `tar` in a terminal avoids the quarantine flag
entirely, since command-line tools do not set it.

### Windows

Unzip `mbr-windows-x86_64.zip` and run `mbr.exe`. GUI mode needs the Microsoft
Edge WebView2 runtime, which ships with Windows 11 and current Windows 10.

Two features are missing from Windows builds because their dependencies do not
build on `windows-msvc` without a prebuilt ffmpeg SDK:

* Video metadata and HLS transcoding

* PDF cover image extraction

Server mode, GUI mode, static site generation, and search all work normally.

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

* [Quick Start Guide](quickstart/) - Get productive in 5 minutes

* [Modes of Operation](../modes/) - Learn about GUI, Server, and Build modes

