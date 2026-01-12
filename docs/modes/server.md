---
title: Server Mode
description: Local web server with mbr -s
order: 2
---

# Server Mode

Server mode starts a local web server, allowing you to browse your markdown files in any web browser with live reload.

## Starting the Server

```bash
mbr -s /path/to/notes
```

By default, the server runs at [http://127.0.0.1:5200/](http://127.0.0.1:5200/).

## Features

### Live Reload

When you save changes to a file, the browser automatically reloads:

1. mbr watches for file changes via the filesystem
2. Changes trigger a WebSocket notification
3. Connected browsers refresh the current page

This works for:
- Markdown file content
- YAML frontmatter changes
- Template modifications (in `.mbr/`)
- CSS/style changes

### Full-Text Search

Press **/** or click the search icon to open search:

- Search across all markdown files
- Filter by metadata, content, or both
- Scope to current folder or everywhere
- Results show snippets with highlighted matches

### Directory Browsing

Navigate your markdown repository:

- Folder listings with file counts
- Subdirectory navigation
- Breadcrumb trail for context
- Sort by name, date, or title

### Tag Navigation

If your files use YAML frontmatter with tags:

```yaml
---
tags: project, important, review
---
```

The sidebar shows a tag tree for filtering files.

### Recent Files

mbr tracks recently viewed files (stored in browser localStorage), providing quick access to your working set.

## File Browser Sidebar

Press **`-`** (minus key) to open the file browser sidebar. This is a three-pane navigator for exploring your entire markdown collection.

### Layout

```
┌──────────────────────────────┐
│  × Close                     │
├──────────────────────────────┤
│  🔍 Filter...                │
├──────────────────────────────┤
│  SHORTCUTS                   │
│    ⭐ My Important File      │
│    ⭐ Daily Notes            │
├──────────────────────────────┤
│  RECENT                      │
│    📄 guide.md               │
│    📄 notes.md               │
├──────────────────────────────┤
│  TAGS                        │
│    ▸ project (12)            │
│    ▸ review (5)              │
│    ▾ important               │
│        ▸ urgent (3)          │
├──────────────────────────────┤
│  FOLDERS                     │
│    ▸ docs/ (24)              │
│    ▸ notes/ (156)            │
│    ▾ projects/               │
│        ▸ website/ (8)        │
└──────────────────────────────┘
```

### Browser Sections

| Section | Description |
|---------|-------------|
| **Filter** | Quick text filter across all visible items |
| **Shortcuts** | Files you've pinned for quick access |
| **Recent** | Recently viewed files (persisted in localStorage) |
| **Tags** | Hierarchical tag tree with file counts |
| **Folders** | Directory structure mirroring your filesystem |

### Keyboard Shortcuts

| Key | Action |
|-----|--------|
| `-` | Toggle sidebar open/closed |
| `Escape` | Close sidebar |
| `↑` / `↓` | Navigate items |
| `Enter` | Open selected file or expand folder |
| `→` | Expand folder or tag |
| `←` | Collapse folder or tag |

### Pinning Shortcuts

Right-click any file in the browser to add it to your shortcuts. Shortcuts appear at the top for instant access to frequently used files.

To remove a shortcut, right-click it and select "Remove from shortcuts."

### Tag Hierarchy

Tags support hierarchical organization using `/` as a separator:

```yaml
---
tags: project/website, project/website/frontend
---
```

This creates a nested tag tree:
- project (parent)
  - website (child with count)
    - frontend (grandchild)

Click a parent tag to see all files with that tag or any descendant tags.

### Persistence

The browser remembers your preferences across sessions:

| Data | Storage | Cleared By |
|------|---------|------------|
| Recent files | localStorage | Clear browser data |
| Shortcuts | localStorage | Clear browser data |
| Expanded folders | Session only | Page refresh |
| Selected tags | Session only | Page refresh |

## Full-Text Search

Press **`/`** (forward slash) to open the search dialog.

### Search Features

| Feature | Description |
|---------|-------------|
| **Fuzzy matching** | Finds partial matches and typos |
| **Ranked results** | Best matches appear first |
| **Snippets** | Shows matching text with highlighted terms |
| **Metadata search** | Searches titles, tags, and descriptions |
| **Folder scoping** | Limit search to current directory |

### Search Syntax

| Query | Matches |
|-------|---------|
| `rust async` | Files containing both "rust" AND "async" |
| `"exact phrase"` | Files containing the exact phrase |
| `tag:project` | Files with the "project" tag |
| `title:guide` | Files with "guide" in the title |

### Keyboard Navigation

| Key | Action |
|-----|--------|
| `/` | Open search |
| `Escape` | Close search |
| `↑` / `↓` | Navigate results |
| `Enter` | Open selected result |

## Server Endpoints

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/` | GET | Home page (root directory listing) |
| `/{path}/` | GET | Markdown page or directory |
| `/.mbr/site.json` | GET | Full site metadata as JSON |
| `/.mbr/search` | POST | Search endpoint |
| `/.mbr/ws/changes` | WS | WebSocket for live reload |
| `/.mbr/*` | GET | Static assets (CSS, JS, fonts) |

### Search API

The search endpoint accepts POST requests:

```json
{
  "q": "search query",
  "limit": 50,
  "scope": "all",
  "filetype": "markdown",
  "folder": "/docs"
}
```

Response:

```json
{
  "query": "search query",
  "total_matches": 42,
  "results": [
    {
      "url_path": "/docs/guide/",
      "title": "Guide",
      "description": "...",
      "tags": "...",
      "score": 95,
      "snippet": "..."
    }
  ],
  "duration_ms": 15
}
```

## Configuration

### Port Configuration

Default port is 5200. Change via configuration:

```toml
# .mbr/config.toml
port = 3000
```

Or environment variable:

```bash
MBR_PORT=3000 mbr -s ~/notes
```

If the configured port is in use, mbr automatically tries the next port.

### IP Binding

By default, mbr binds to `127.0.0.1` (localhost only). To allow network access:

```toml
# .mbr/config.toml
ip = "0.0.0.0"
```

> **Warning**: Binding to `0.0.0.0` exposes your files to the network.

### oEmbed Timeout

Control how long mbr waits for URL metadata:

```bash
mbr -s --oembed-timeout 1000 ~/notes  # 1 second
```

## Development Workflow

A typical writing workflow:

```bash
# Terminal 1: Start server
mbr -s ~/notes

# Terminal 2: Edit files
vim ~/notes/new-article.md
```

Changes appear in the browser automatically.

### With Editor Integration

Many editors can open URLs on save. Configure your editor to:

1. Save the file
2. Trigger browser refresh (or let live reload handle it)

### Multiple Repositories

Run multiple servers on different ports:

```bash
mbr -s ~/notes &                    # Default port 5200
MBR_PORT=5201 mbr -s ~/docs &       # Custom port 5201
```

## Caching

mbr provides proper cache headers for browser caching:

- **ETag**: Content-based cache validation
- **Last-Modified**: Time-based cache validation

Browsers can cache static assets and validate on subsequent requests.

## Troubleshooting

### Port Already in Use

If you see "Address already in use":

1. Another mbr instance may be running
2. Another application is using the port
3. Try a different port: `MBR_PORT=3000 mbr -s ~/notes`

### Live Reload Not Working

If changes don't appear:

1. Check WebSocket connection in browser DevTools
2. Ensure file is being saved (not just buffered)
3. Verify file is in the watched directory
4. Check for errors with `-v` flag

### Slow Initial Load

For large repositories:

1. First load scans all files for metadata
2. Subsequent loads are faster (browser caching)
3. Consider using `ignore_dirs` to skip large directories
