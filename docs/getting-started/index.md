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
nix run --accept-flake-config github:zmre/mbr-markdown-browser -- -g /path/to/notes
```

### Using the Binary Cache

Prebuilt binaries for `x86_64-linux`, `aarch64-linux`, and `aarch64-darwin` are
published to [zmre.cachix.org](https://zmre.cachix.org) on every push to `main`.
Without them, installing mbr means compiling ffmpeg, pdfium, and the full crate
dependency graph — tens of minutes.

Nix **ignores substituters declared in a flake's `nixConfig` unless the flake is
trusted**, so you have to opt in. Either pass the flag each time:

```bash
nix run --accept-flake-config github:zmre/mbr-markdown-browser -- -g /path/to/notes
```

...or configure it permanently in `~/.config/nix/nix.conf` (or
`/etc/nix/nix.conf`):

```
extra-substituters = https://zmre.cachix.org
extra-trusted-public-keys = zmre.cachix.org-1:WIE1U2a16UyaUVr+Wind0JM6pEXBe43PQezdPKoDWLE=
```

**Caveat:** `substituters` is a *trusted* Nix setting. On a multi-user install,
a user who is not listed in `trusted-users` gets

```
warning: ignoring untrusted flake configuration setting 'extra-substituters'
```

and builds from source regardless of which method above they used. Fixing that
requires adding the user to `trusted-users` in the system-level `nix.conf` and
restarting the daemon — an administrator action, not something the flake can do.

The cache is plain signed HTTP, so which Nix you run is irrelevant: Determinate
Nix, upstream Nix, Lix, and nix-darwin all consume it identically.

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
    mbr.url = "github:zmre/mbr-markdown-browser";
  };

  outputs = { self, nixpkgs, mbr }: {
    # Use mbr.packages.${system}.default in your configuration
  };
}
```

A flake's `nixConfig` applies only when *that* flake is the one being built, so
consuming mbr as an input does **not** inherit its substituters — even with
`--accept-flake-config`. Declare them in your own flake to avoid rebuilding mbr
from source:

```nix
{
  nixConfig = {
    extra-substituters = [ "https://zmre.cachix.org" ];
    extra-trusted-public-keys = [
      "zmre.cachix.org-1:WIE1U2a16UyaUVr+Wind0JM6pEXBe43PQezdPKoDWLE="
    ];
  };
  # ...
}
```

## Using Cargo

If you have Rust installed, you can build from source:

```bash
cargo install --git https://github.com/zmre/mbr-markdown-browser
```

Note that Cargo does not use the binary cache — this compiles everything
locally. Use the Nix install above if you want prebuilt binaries.

### Prerequisites

* Rust 1.75 or later

* A C compiler (for some dependencies)

## Binary Releases

Pre-built binaries are available on the [GitHub Releases](https://github.com/zmre/mbr-markdown-browser/releases) page.

| Platform | File | Notes |
|----------|------|-------|
| macOS | `mbr-macos-arm64.dmg` | App bundle, Apple Silicon |
| macOS | `mbr-macos-arm64.tar.gz` | App bundle, Apple Silicon |
| macOS | `mbr-cli-macos-arm64.tar.gz` | Command-line binary only |
| Linux | `mbr-linux-x86_64.tar.gz` | Command-line binary |
| Linux | `mbr-linux-arm64.tar.gz` | Command-line binary, aarch64 |
| Windows | `mbr-windows-x86_64.zip` | See limitations below |
| Windows | `mbr-windows-aarch64.zip` | ARM64, see limitations below |

> **Intel Macs are not supported.** Prebuilt macOS downloads are Apple Silicon
> (arm64) only. Intel (x86_64) macOS builds were dropped, because Apple has not
> shipped an Intel Mac since 2020 and macOS 26 is the last release to support
> them. Intel Macs can still build from source — both `cargo install` and
> `nix build .#mbr` work on `x86_64-darwin`.

### macOS App Bundle

The macOS release includes `MBR.app`, a native application bundle with:

* Application icon

* Native menu bar integration

* QuickLook extension for Finder previews

The easiest install is the DMG: open it and drag `MBR.app` to the Applications
folder. You can also extract a `.tar.gz` and move `MBR.app` yourself.

> [!IMPORTANT]
> Installing `MBR.app` makes it the **default app for markdown files**
> (`.md`, `.markdown`, `.mkd` and friends), replacing whatever you used before.
> It also registers as an *alternate* viewer for plain text, so it shows up
> under "Open With" for `.txt` and source files without displacing your text
> editor. mbr always registers as a viewer, never an editor. To restore a
> previous default, select a markdown file in Finder, press **Cmd+I**, pick
> your app under "Open with", and click **Change All**. See
> [QuickLook Preview](../modes/quicklook/) for the full list of claimed types.

These builds are ad-hoc signed but not notarized, so macOS blocks the first
launch. Open **System Settings › Privacy & Security** and click **Open Anyway**,
or clear the quarantine flag from a terminal:

```bash
xattr -dr com.apple.quarantine /Applications/MBR.app
```

Extracting the tarball with `tar` in a terminal avoids the quarantine flag
entirely, since command-line tools do not set it.

### Windows

Unzip the archive matching your machine — `mbr-windows-x86_64.zip` on Intel and
AMD, `mbr-windows-aarch64.zip` on ARM64 — and run `mbr.exe`. The binary is
self-contained and does not need the Visual C++ Redistributable, because the C
runtime is linked statically. GUI mode needs the Microsoft Edge WebView2
runtime, which ships with Windows 11 and current Windows 10.

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

