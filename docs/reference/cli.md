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

`PATH` selects the page (or folder) to open; the *repository root* is discovered
separately, by searching upward for a marker such as `.mbr/`, `.git/` or
`.obsidian/` (see [Configuration](configuration.md)). The two are usually
different, and deliberately: `mbr -s docs/guide.md` inside a checkout serves the
whole checkout with that page open.

**Hidden directories named on the command line are scanned.** mbr normally skips
every path component beginning with `.`, since those hold tooling state rather
than notes. Naming one is a statement that it holds notes, so `mbr -s .scratch`
(or `mbr -b .scratch`) indexes `.scratch` and everything under it, even though
the root is the enclosing repository. The waiver covers only the directories on
the path from the root down to `PATH` — a `.git` or `.obsidian` *inside*
`.scratch` is still skipped, and so is a hidden directory elsewhere in the repo.

## Mode Flags

These flags are mutually exclusive:

| Flag | Description |
|------|-------------|
| (none) | Launch GUI window (default with `gui` feature) |
| `-s, --server` | Start web server only (no GUI) |
| `-g, --gui` | Launch native GUI window (explicit) |
| `-b, --build` | Generate static site |
| `--check-links-only` | Report broken links (and suggested fixes) without building anything; see [Checking links](#checking-links) |
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
| `--host <HOST>` | Server IP address to bind. **Warning:** binding to a non-loopback address (e.g. `0.0.0.0`) makes the entire repository readable to the network with no authentication; mbr logs a warning at startup. | `127.0.0.1` |
| `--output <PATH>` | Output directory for static build | `build` |
| `--template-folder <PATH>` | Custom template folder | (uses `.mbr/`) |
| `--oembed-timeout-ms <MS>` | Timeout for URL metadata fetch (0 to disable) | `500` (server/GUI), `0` (build) |
| `--oembed-cache-size <BYTES>` | Max oembed cache size (0 to disable). Affects link-preview metadata only — the video/PDF metadata cache is sized by the `media_cache_size` config option / `MBR_MEDIA_CACHE_SIZE`, which has no CLI flag. | `2097152` (2MB) |
| `--build-concurrency <N>` | Files to process in parallel during build (and `--check-links-only`) | auto (2x cores, max 32) |
| `--format <FORMAT>` | Report format for `--check-links-only`: `markdown` or `json`. Refused without `--check-links-only` | `markdown` |
| `--skip-link-checks` | Skip internal link validation during build | `false` |
| `--fail-on-broken-links` | Exit with a non-zero status if the static build (`-b`) finds broken internal links. For CI. No effect with `--skip-link-checks` (the build warns on stderr when both are set, since the combination disables the gate). | `false` |
| `--no-link-tracking` | Disable bidirectional link tracking | `false` |
| `--no-relationship-tracking` | Disable typed relationship tracking | `false` |
| `--no-tasks` | Disable the task browser. The `/.mbr/tasks` endpoint returns `404` and no task index is ever built. Server/GUI only — static builds never include tasks either way. | `false` |
| `--no-symlink-mounts` | Disable [read-only symlink mounts](configuration.md#read-only-symlink-mounts): a symlink to a folder or file outside the repository (or the static folder) is not served (`404`) and not built, exactly as before mounts existed. Hidden files stay blocked either way. Applies to `-s`, `-g` (including folders opened later from the GUI) and `-b`. Also `MBR_NO_SYMLINK_MOUNTS=1`. Operator-only: a repository's `.mbr/config.toml` cannot set it, in either direction. QuickLook and mobile do not use mounts and are unaffected. Use it when serving or building a repository you did not create. | `false` (mounts on) |
| `--no-review` | Disable `data-mbr-line` source-line attributes on block elements. Server/GUI only — static builds, the CLI and QuickLook never emit them either way. With editing on, definition-list terms (`<dt>`) keep theirs, since a [flashcard](../features/flashcards.md) review addresses its card by that line. | `false` |
| `--mark-incomplete` | Highlight TK/TODO/FIXME/XXX anywhere in a line | server/GUI: on, build: off |
| `--no-mark-incomplete` | Disable incomplete-marker highlighting | server/GUI: off, build: on (no effect) |
| `--title-prefix <TEXT>` | Text to prepend to all page titles | `""` (empty) |
| `--title-suffix <TEXT>` | Text to append to all page titles | `""` (empty) |
| `--transcode` | [EXPERIMENTAL] Enable dynamic video transcoding to 720p/480p (server/GUI mode only). Not needed for [automatic playback recovery](configuration.md#automatic-playback-recovery), which is a stream copy and is always available. | `false` |
| `--edit` | Enable the in-browser markdown editing endpoints (server/GUI mode). Loopback callers may edit without a token (still CSRF-protected); non-loopback callers require a token. Beats `edit_enabled` in config. See [editing](../features/editing.md). | GUI: on, server: off |
| `--no-edit` | Disable in-browser editing, whatever `.mbr/config.toml` or `MBR_EDIT_ENABLED` says. Mainly for the GUI, where editing is on by default; applies to every folder the GUI opens. Conflicts with `--edit`. | (unset) |
| `--generate-edit-token` | Prompt for a password (blank to auto-generate a random token), print the token and the `edit_token_hash` config line, then exit. Writes nothing to disk. | |
| `-v, --verbose` | Increase log verbosity | warn level |
| `-q, --quiet` | Suppress output except errors | |
| `--help` | Print help message | |
| `--version` | Print version | |

Flags keep winning after a running server reloads `.mbr/config.toml` (on save,
or Cmd/Ctrl+R in the GUI): the reload re-applies the command line. `--host` and
`--port` are fixed for the life of the process. See
[Reloading Configuration](configuration.md#reloading-configuration).

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

# List broken links with suggested fixes (no build)
mbr --check-links-only ~/notes

# The same report as JSON, for tooling or an LLM
mbr --check-links-only --format json ~/notes > broken-links.json

# Build to custom directory
mbr -b --output ./public ~/notes

# Use custom template folder
mbr -s --template-folder ./my-theme ~/notes

# Increase oembed timeout
mbr -s --oembed-timeout-ms 2000 ~/notes

# Disable oembed fetching (uses plain links)
mbr -s --oembed-timeout-ms 0 ~/notes

# Enable in-browser editing (loopback: no token needed)
mbr -s --edit ~/notes

# Open the GUI read-only (editing is on by default there)
mbr --no-edit ~/notes

# Generate a hashed editing token for remote use
mbr --generate-edit-token

# Remote editing with the hash from the environment (no config.toml change)
MBR_EDIT_TOKEN_HASH='$argon2id$...' mbr -s --edit --host 0.0.0.0 ~/notes
```

A startup failure (an invalid flag value, a broken `.mbr/config.toml`, `--edit`
or `edit_enabled = true` on a non-loopback host without an `edit_token_hash`) prints `Error: <message>`
to stderr, followed by one `caused by:` line per underlying cause, and exits
with status 1.

---

## Checking links

```bash
mbr --check-links-only [--format markdown|json] [PATH]
```

Scans the repository `PATH` belongs to and reports every broken reference,
without building a site: nothing is written, nothing is fetched from the
network (oembed is off) and no search index is made. Configuration is read as
for any other mode (`.mbr/config.toml`, `MBR_*`, flags), so ignore globs, the
`static_folder` overlay, symlink mounts (`--no-symlink-mounts`), hidden-file
rules, markdown extensions, `index_file` and tag sources all apply. The whole
repository is checked, whatever page `PATH` names.

Each note is rendered exactly as the server renders it, and judged by the same
checks as the [page-problems panel](../features/links.md#broken-links), so the
report and the panel agree. It lists:

- **broken links** — a markdown link, wikilink or raw HTML `href` whose target
  is not served (`missing`), that climbs above the repository root
  (`escapes_root`), or that names a markdown page without its trailing slash
  (`missing_trailing_slash`, in practice only an unquoted raw HTML `href`);
- **broken media** — `![]()`, `<img>`, `<video>`, `<audio>` and `<source>`
  `src`s, slide `data-background-*` URLs and a deck's `slides.background_image`
  (`srcset` candidates are not checked);
- **unresolved wikilinks** — a literal `[[…]]` left in the page;
- **ambiguous wikilinks** — a `[[Name]]` several notes answer to, with every
  candidate.

Each problem carries the 1-based **line** it is on (frontmatter included) and
the construct **as written**. A broken target used on several lines is reported
once per line. A reference that can only be found by its target's file name
(a link inside a chat bubble, a section's `--- {data-background-image=…}`) is
reported on the lines naming that file, else without a line.

**Suggestions** come from the matcher behind the
[404 page](../features/links.md#when-a-page-is-not-found): up to three notes or
files with the same name elsewhere (the usual result of a move), then names
that were extended or shortened. Each is written as a ready-to-paste
replacement in the style of the broken one: a relative `.md` path stays a path
relative to the note's folder (`../install/setup.md`), a root-relative URL stays
root-relative (`/install/setup/`), a wikilink stays a wikilink (`[[New Name]]`,
or a path such as `[[../people/a/John Doe]]` when the name is shared), media
gets a relative file path, and a `#fragment` is kept. When nothing matches but
the missing target's folder still exists, the report says so instead.

**Output.** The report goes to stdout; progress and log lines go to stderr
(`-q` silences them). The default is a markdown todo list, grouped by file in
path order, problems in line order:

```markdown
# Broken links: 3 in 2 files

## docs/guide.md

- [ ] L6 `[Setup](../setup.md)` — no such page or file
  - did you mean: `../install/setup.md` (Setup)
- [ ] L8 `![diagram](img/flow.png)` — no such file
  - did you mean: `../assets/flow.png`

## notes/todo.md

- [ ] L4 `[[New Nam|the new one]]` — no note by that name
  - did you mean: `[[New Name|the new one]]` (New Name)
```

A clean repository prints `No broken links in N files.` Notes that could not be
read (a cloud file not downloaded yet, a permission error) are listed in a
final `## Could not be checked` section.

**Exit status:** `0` nothing broken (unreadable notes alone do not fail the
run), `1` at least one problem, `2` the check could not run (bad configuration,
missing path, or a usage error).

### JSON report

`--format json` prints one JSON document (`schema_version` changes only with
an incompatible change):

```json
{
  "schema_version": 1,
  "summary": { "files_checked": 56, "files_with_problems": 2, "problems": 3, "unreadable": 0 },
  "files": [
    {
      "path": "docs/guide.md",
      "url": "/docs/guide/",
      "problems": [
        {
          "line": 6,
          "kind": "broken_link",
          "reason": "missing",
          "syntax": "link",
          "authored": "[Setup](../setup.md)",
          "destination": "../setup.md",
          "text": "Setup",
          "href": "../../setup/",
          "target": "/setup/",
          "suggestions": [
            { "replacement": "../install/setup.md", "url": "/install/setup/",
              "path": "install/setup.md", "title": "Setup" }
          ]
        }
      ]
    }
  ],
  "unreadable": [ { "path": "notes/remote.md", "error": "…" } ]
}
```

| Field | Meaning |
|-------|---------|
| `files[].path` | Source path, relative to the repository root, `/`-separated |
| `files[].url` | The note's site URL |
| `line` | 1-based line in the file, frontmatter included; `null` when it could not be located |
| `kind` | `broken_link`, `broken_media`, `unresolved_wikilink`, `ambiguous_wikilink` |
| `reason` | `missing`, `escapes_root`, `missing_trailing_slash`, `unresolved`, `ambiguous` |
| `syntax` | `link`, `wikilink`, `image` or `html` (a raw HTML attribute); `null` when located only by file name |
| `media` | For `broken_media`: `image`, `video`, `audio` or `source` (omitted otherwise) |
| `authored` | The construct as written (`[Setup](../setup.md)`, `[[Old Name]]`, `<img src="x.png">`), whitespace collapsed; the whole line when located only by file name |
| `destination` | The destination as written (`../setup.md`, `Old Name`) |
| `text` | The link's text (links only, omitted when empty) |
| `href` | What the renderer emitted (what a browser follows) |
| `target` | The site URL it resolves to, decoded; `null` when it escapes the root |
| `suggestions[]` | Up to three, best first: `replacement` (paste in place of the destination; the whole `[[…]]` for a wikilink), `url`, `path` (`null` outside the repository), `title` (`null` for a file) |
| `folder` | When nothing matched and the target's folder still exists: `{ "url", "title" }` (omitted otherwise) |
| `resolved_to`, `candidates[]` | For `ambiguous_wikilink`: the note it opens, and every note answering to the name as a `suggestions`-shaped entry whose `replacement` names exactly that note |

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
