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

Everything you use to get around — the [file browser sidebar](../features/browsing/),
[search](../features/search/), the [link explorer and info
panel](../features/links/), [tags](../features/tags/) and the
[calendar](../features/calendar/) — works the same in server mode as in the GUI.
The endpoints behind them are in the [HTTP API](../reference/http-api/) reference.
What is specific to server mode:

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
- `.mbr/config.toml`: the server applies the new settings on the same port
  without a restart, then every open page reloads (see
  [Reloading Configuration](../reference/configuration.md#reloading-configuration))

### Editing Is Off Unless Asked For

Unlike the GUI, `mbr -s` serves **read-only** by default. Turn
[in-browser editing](../features/editing.md) on with `--edit`, `edit_enabled = true` in
`.mbr/config.toml`, or `MBR_EDIT_ENABLED=true`; `--no-edit` turns it off
whatever the configuration says. On a non-loopback `--host`, editing also needs
an `edit_token_hash`.

### External Links

**Server mode deliberately does nothing with external links.** `https://` links,
`mailto:`, `message:`, `zoommtg:` and every other scheme are left entirely to the
browser you are visiting with, which already opens them the way you expect. Same
for static builds.

This is the opposite of [GUI mode](gui.md#external-links), where there is no
browser to defer to and mbr hands the URL to the operating system itself. That
hand-off is **GUI-only by design and enforced at runtime**: the launcher refuses,
before touching the OS, unless a GUI window is actually running in the process,
and logs the refusal at `warn`.

The reason is worth stating plainly: **a server must never be induced to launch
applications on its host.** Launching an application is something an interactive
window does for the person in front of it. `mbr -s` has no such person, and if a
page could make the serving process start programs, anyone who could reach that
page — especially on a shared or non-loopback bind — could start programs on the
machine running it. So the capability simply is not available outside GUI mode,
regardless of what a page, a template override, or a future code path asks for.

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
