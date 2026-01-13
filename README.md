<div align="center">

# ![mbr - the markdown browser](docs/images/banner.png)

**The fast, complete markdown browser and static site generator**

  [![Docs](https://img.shields.io/badge/docs-latest-blue.svg?style=flat-square)](https://zmre.github.io/mbr-markdown-browser/)
[![GitHub Release](https://img.shields.io/github/v/release/zmre/mbr-markdown-browser?style=flat&logo=github&logoColor=white&label=Release&color=8B5CF6)](https://github.com/zmre/mbr-markdown-browser/releases)
  ![Language](https://img.shields.io/github/languages/top/zmre/mbr-markdown-browser)
  ![Last Commit](https://img.shields.io/github/last-commit/zmre/mbr-markdown-browser)
  [![CI status](https://github.com/zmre/mbr-markdown-browser/actions/workflows/ci.yml/badge.svg)](https://github.com/zmre/mbr-markdown-browser/actions)
  [![License](https://img.shields.io/github/license/zmre/mbr-markdown-browser?style=flat&color=22C55E)](LICENSE)
</div>

---

## Why mbr?

> **No crazy syntax. No required directory structures. Just markdown.**

Most static site generators force you into their world: custom folder layouts, proprietary frontmatter, bespoke shortcodes. mbr takes a different approach. Point it at any collection of markdown files and it just works for previewing a single file, browsing thousands of notes, or building a deployable website.

And unlike other markdown previewers, this one allows quick navigation between markdown files.

## Features

| Feature | Description |
|---------|-------------|
| **Instant Preview** | Sub-second markdown rendering with live reload |
| **Native GUI** | macOS/Linux app with native menus and shortcuts (Windows should work, but is untested) |
| **Static Sites** | Generate deployable websites with full-text search |
| **Smart Navigation** | Browse by folders, tags, recents, and full-text search |
| **Keyboard Friendly** | Vim-like shortcuts are available for everything in the UI |
| **Rich Media** | Embed videos, audio, PDFs, and YouTube with simple markdown standard syntax |
| **Fully Customizable** | Override themes, templates, and UX per-repo |

## Quick Start

### Install

```bash
# Using Nix to quick run without installing
nix run github:zmre/mbr -- -g /path/to/notes

# Using Cargo
cargo install --git https://github.com/zmre/mbr
```

_File an issue if you want it packaged in a particular way._

### Run

```bash
mbr README.md         # Render to stdout
mbr -s ~/notes        # Start web server at http://127.0.0.1:5200/
mbr -g ~/notes        # Launch native GUI window
mbr -b ~/notes        # Build static site to ./build/
```

## Documentation

See the [full documentation](https://zmre.github.io/mbr-markdown-browser/) for detailed guides. The documentation site itself is built with mbr, serving as a live example of its capabilities.

- [Getting Started](https://zmre.github.io/mbr-markdown-browser/getting-started/) - Installation and first steps
- [Modes of Operation](https://zmre.github.io/mbr-markdown-browser/modes/) - GUI, Server, Build, and QuickLook
- [Customization](https://zmre.github.io/mbr-markdown-browser/customization/) - Themes, templates, and components
- [Markdown Extensions](https://zmre.github.io/mbr-markdown-browser/markdown/) - Extended syntax reference
- [Architecture](https://zmre.github.io/mbr-markdown-browser/reference/architecture/) - Technical overview
- [Development](https://zmre.github.io/mbr-markdown-browser/reference/development/) - Building and contributing

## License

MIT - see [LICENSE](LICENSE) for details.
