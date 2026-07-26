---
title: Static Site Generation
description: Build static sites with mbr -b
order: 3
---

# Static Site Generation

Build mode generates a complete static website that can be deployed to any hosting platform.

## Basic Usage

```bash
# Build to ./build/ directory
mbr -b /path/to/notes

# Build to custom directory
mbr -b --output ./public /path/to/notes
```

## Build Process

```mermaid
flowchart LR
    SCAN[Scan Repository] --> RENDER[Render Markdown]
    RENDER --> SECTIONS[Generate Sections]
    SECTIONS --> ASSETS[Symlink Assets]
    ASSETS --> INDEX[Pagefind Index]
    INDEX --> LINKS[Validate Links]
    LINKS --> OUTPUT[build/]
```

### 1. Scan Repository

mbr scans all markdown files, extracting:
- File paths and names
- YAML frontmatter metadata
- Directory structure

### 2. Render Markdown

Each markdown file becomes an HTML page:
- `README.md` → `README/index.html`
- `docs/guide.md` → `docs/guide/index.html`

### 3. Generate Section Pages

Directories get index pages listing their contents:
- Files with titles and descriptions
- Subdirectory links
- Breadcrumb navigation

### 4. Place Assets

Static assets (images, PDFs, videos) are symlinked rather than copied:

```
build/images/ → ../images/
```

> **Note**: Symlinking is used on macOS and Linux. On Windows, assets are
> **copied** instead, because creating symlinks there requires Developer Mode or
> an elevated process. Builds work the same either way, but on Windows the
> output directory is as large as your assets rather than nearly empty. If you
> are publishing the `build/` directory, note that symlinks must be dereferenced
> when archiving (for example `tar -h`) whereas copies need no special handling.

### 5. Pagefind Index

mbr generates a [Pagefind](https://pagefind.app/) search index:

```
build/.mbr/pagefind/
├── pagefind.js
├── pagefind-ui.js
├── pagefind-ui.css
└── *.pf_*  # Index files
```

Search works entirely client-side with no server required.

### 6. Validate Links

mbr checks all internal links and reports broken references:

```
Warning: Broken link found
  Source: /docs/guide/index.html
  Target: /docs/missing-page/
```

### Frontmatter Parse Errors

While rendering, mbr also reports any pages whose YAML frontmatter fails to
parse. A parse failure discards the **entire** frontmatter block (so otherwise
valid fields like `title:` or `style: slides` are silently lost), which is why
it is surfaced explicitly:

```
⚠️  Frontmatter parse errors (1 total):
   /presentations/talk/ → while parsing a block mapping, did not find expected key
```

A common cause is using tabs or `*` list markers instead of spaces and `-` in a
YAML list. In server/GUI mode these same errors appear in the per-page problems
panel (the ⚠ indicator in the navigation bar).

## Output Structure

```
build/
├── index.html              # Home page
├── README/
│   └── index.html          # README.md rendered
├── docs/
│   ├── index.html          # Directory listing
│   └── guide/
│       └── index.html      # docs/guide.md
├── images/ → ../images     # Symlinked assets
└── .mbr/
    ├── site.json           # Site metadata
    ├── theme.css           # Styling
    ├── pagefind/           # Search index
    └── *.js                # Components
```

## Deployment

### GitHub Pages

See [Integration](../reference/integration/) for a complete GitHub Actions workflow.

Quick setup:

1. Build: `mbr -b --output ./docs /path/to/notes`
2. Enable GitHub Pages in repository settings
3. Select `/docs` folder as source

### Netlify

```toml
# netlify.toml
[build]
  publish = "build"
  command = "mbr -b ."
```

### Any Static Host

The `build/` folder contains plain HTML/CSS/JS that works anywhere:

- Amazon S3 + CloudFront
- Cloudflare Pages
- Vercel
- Firebase Hosting
- Any web server (nginx, Apache)

## Search Configuration

Pagefind search is automatically configured and works out of the box.

### Search Features

- **Instant results**: Searches as you type
- **Fuzzy matching**: Finds close matches
- **Heading navigation**: Results link to specific sections
- **Mobile friendly**: Works on all devices

### Customizing Search

Pagefind respects data attributes in your HTML. Advanced customization is available through Pagefind's configuration options.

## Build Performance

mbr is optimized for fast builds:

- Parallel rendering with Tokio
- Efficient file scanning with rayon
- Symlinks instead of copies for assets

Typical performance:
- ~100 files: < 1 second
- ~1,000 files: 2-5 seconds
- ~10,000 files: 10-30 seconds

## Incremental Builds

mbr does not currently support incremental builds. Each build regenerates all files.

For faster iteration during development, use server mode (`mbr -s`) instead.

## Troubleshooting

### Windows Output Is Larger Than Expected

On Windows, assets are copied into the output directory rather than symlinked
(symlink creation requires Developer Mode or an elevated process). A repository
with large images or video will therefore produce a correspondingly large
`build/` directory. To get symlink behavior, build under WSL instead:

```bash
# In WSL
mbr -b /mnt/c/Users/you/notes
```

### Broken Links Reported

If mbr reports broken links:

1. Check that the target file exists
2. Verify the link path is correct (case-sensitive on Linux)
3. Ensure the file has a recognized markdown extension

### Large Asset Files

For repositories with large assets (videos, etc.):

1. Assets are symlinked, not copied
2. Deployment may need to handle symlinks specially
3. Consider using a CDN for large media files

### Build Output Too Large

If the build output is unexpectedly large:

1. Check for accidentally included directories
2. Use `ignore_dirs` in configuration
3. Verify symlinks are being created (not copies)
