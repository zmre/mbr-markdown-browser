---
title: CLI Reference
description: Command line options and flags
order: 1
---

# CLI Reference

Complete reference for mbr's command-line interface.

For configuration file settings, environment variables, and feature-specific behavior tuning, see the [Configuration Reference](configuration/).

## Usage

```bash
mbr [OPTIONS] [PATH]
```

If `PATH` is omitted, mbr uses the current directory.

## Mode Flags

These flags are mutually exclusive:

| Flag | Description |
|------|-------------|
| (none) | Launch GUI window (default with `gui` feature) |
| `-s, --server` | Start web server only (no GUI) |
| `-g, --gui` | Launch native GUI window (explicit) |
| `-b, --build` | Generate static site |
| `--extract-video-metadata` | Extract video metadata to sidecar files (requires `media-metadata` feature) |
| `--extract-pdf-cover` | Extract cover images from PDF files (requires `media-metadata` feature) |

### Media File Arguments in GUI Mode

When a media file (video, audio, image, or PDF) is passed as the `PATH` argument in GUI mode, mbr automatically opens the appropriate media viewer instead of displaying the raw file. For example, `mbr -g videos/demo.mp4` opens the video viewer at `/.mbr/videos/?path=%2Fvideos%2Fdemo.mp4`.

Supported media types are detected by file extension:

| Type | Extensions |
|------|------------|
| Video | mp4, m4v, mov, webm, flv, mpg, mpeg, avi, 3gp, wmv, mkv, ts, mts, m2ts, vob, divx, xvid, asf, rm, rmvb, f4v, ogv |
| Audio | mp3, wav, ogg, flac, aac, m4a, aiff, aif, oga, opus, wma |
| Image | jpg, jpeg, png, webp, gif, bmp, tif, tiff, svg |
| PDF | pdf |

## Options

| Option | Description | Default |
|--------|-------------|---------|
| `--output <PATH>` | Output directory for static build | `build` |
| `--template-folder <PATH>` | Custom template folder | (uses `.mbr/`) |
| `--oembed-timeout-ms <MS>` | Timeout for URL metadata fetch (0 to disable) | `500` (server/GUI), `0` (build) |
| `--oembed-cache-size <BYTES>` | Max oembed cache size (0 to disable) | `2097152` (2MB) |
| `--build-concurrency <N>` | Files to process in parallel during build | auto (2x cores, max 32) |
| `--skip-link-checks` | Skip internal link validation during build | `false` |
| `--no-link-tracking` | Disable bidirectional link tracking | `false` |
| `--mark-incomplete` | Highlight blocks starting with TK/TODO/FIXME/XXX | server/GUI: on, build: off |
| `--no-mark-incomplete` | Disable incomplete-block highlighting | server/GUI: off, build: on (no effect) |
| `--title-prefix <TEXT>` | Text to prepend to all page titles | `""` (empty) |
| `--title-suffix <TEXT>` | Text to append to all page titles | `""` (empty) |
| `--transcode` | [EXPERIMENTAL] Enable dynamic video transcoding (server/GUI mode only) | `false` |
| `-v, --verbose` | Increase log verbosity | warn level |
| `-q, --quiet` | Suppress output except errors | |
| `--help` | Print help message | |
| `--version` | Print version | |

### Boolean Flag Naming Convention

mbr uses two patterns for boolean flags that disable behavior:

- **`--skip-X`**: Skips a build-time operation. Example: `--skip-link-checks` skips link validation during static builds.
- **`--no-X`**: Disables a runtime feature. Example: `--no-link-tracking` disables bidirectional link tracking.

### Verbosity Levels

| Flag | Level |
|------|-------|
| (none) | warn |
| `-v` | info |
| `-vv` | debug |
| `-vvv` | trace |

The `RUST_LOG` environment variable overrides these flags.

## Examples

```bash
# Launch GUI (default mode)
mbr ~/notes
mbr README.md

# Render single file to stdout (CLI mode)
mbr -o README.md
mbr -o README.md > output.html

# Start server on default port
mbr -s ~/notes

# Start server with debug logging
mbr -s -vv ~/notes

# Launch GUI window (explicit)
mbr -g ~/notes

# Open a media file in the GUI media viewer
mbr -g videos/example.mp4    # Opens video viewer
mbr -g music/song.mp3        # Opens audio player
mbr -g images/photo.jpg      # Opens image viewer
mbr -g docs/paper.pdf        # Opens PDF viewer

# Build static site
mbr -b ~/notes

# Build to custom directory
mbr -b --output ./public ~/notes

# Use custom template folder
mbr -s --template-folder ./my-theme ~/notes

# Increase oembed timeout
mbr -s --oembed-timeout-ms 2000 ~/notes

# Disable oembed fetching (uses plain links)
mbr -s --oembed-timeout-ms 0 ~/notes
```

---

# Media Viewer Endpoints

mbr provides dedicated viewer pages for media files. These endpoints render media content within the site's navigation chrome (header, breadcrumbs, theme) for a consistent browsing experience.

## Available Endpoints

| Endpoint | Description |
|----------|-------------|
| `/.mbr/videos/` | Video player page |
| `/.mbr/pdfs/` | PDF viewer page |
| `/.mbr/audio/` | Audio player page |

## Usage

Each endpoint accepts a `path` query parameter specifying the media file location (relative to repository root):

```bash
# Video viewer
http://localhost:5200/.mbr/videos/?path=/videos/demo.mp4

# PDF viewer
http://localhost:5200/.mbr/pdfs/?path=/docs/report.pdf

# Audio player
http://localhost:5200/.mbr/audio/?path=/music/track.mp3
```

The `path` parameter should be URL-encoded if it contains spaces or special characters:

```bash
# Path with spaces
http://localhost:5200/.mbr/videos/?path=/videos/My%20Video.mp4
```

## Features

**Video viewer:**
- Native HTML5 video player with controls
- Automatic poster image from `.cover.jpg` sidecar files
- Chapter navigation via `mbr-video-extras` component (if `.chapters.en.vtt` exists)
- Captions/transcripts support (if `.captions.en.vtt` exists)

**PDF viewer:**
- Embedded PDF viewer using native browser support
- Fallback link to open PDF in new tab

**Audio player:**
- Native HTML5 audio player with controls
- Cover art display from `.cover.jpg` sidecar files
- Filename display

## Security

The media viewer validates all paths to prevent directory traversal attacks:

- Paths containing `..` are rejected
- Paths must resolve within the repository root
- URL-encoded traversal attempts are detected and blocked

Invalid paths return an error page rather than exposing file system contents.

## Static Builds

During static site generation (`-b`), media viewer pages are generated at:

```
build/
└── .mbr/
    ├── videos/
    │   └── index.html
    ├── pdfs/
    │   └── index.html
    └── audio/
        └── index.html
```

These pages work identically in static builds using client-side JavaScript to load and display media based on the `path` query parameter.

## Examples

```bash
# Start server and open video viewer
mbr -s ~/notes
open "http://localhost:5200/.mbr/videos/?path=/videos/demo.mp4"

# Generate static site with media viewers
mbr -b ~/notes
# Viewer pages are at build/.mbr/videos/index.html, etc.

# Test directory traversal protection (should show error)
curl -s "http://localhost:5200/.mbr/videos/?path=/../../../etc/passwd"
# Returns error page, not file contents
```
