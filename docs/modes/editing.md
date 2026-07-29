---
title: In-Browser Editing
description: Edit markdown files from the browser in server/GUI mode
order: 5
---

# In-Browser Editing

mbr can let you edit markdown files directly from the browser while running in
**server or GUI mode**. It is **off by default** and intended for private use
(for example, the GUI window on your own machine). When enabled, a pencil
button (✎) appears next to the info button; pressing it — or the `e` shortcut —
opens a [Milkdown/Crepe](https://milkdown.dev/) editor for the current file.

> [!WARNING]
> Editing writes to your files. Only enable it on repositories you intend to
> modify, and read the [security model](#security-model) before exposing it
> beyond localhost.

## Enabling editing

Enable it with a CLI flag:

```bash
mbr -s --edit ~/notes
```

or in `.mbr/config.toml`:

```toml
edit_enabled = true
```

On loopback (127.0.0.1), that's all you need — local edits require no token
(they are still CSRF- and Host-checked). Note that as soon as an
`edit_token_hash` is configured it is required for **every** request, loopback
included. For remote access, see [Remote editing](#remote-editing).

## Using the editor

1. Open a markdown page and click the ✎ button (top right) or press `e`.
2. The editor loads the file. The **body** is edited in the WYSIWYG Crepe
   editor; any **YAML frontmatter** is shown in a separate raw-text field (open
   the "YAML frontmatter" section to edit it). Frontmatter is preserved
   verbatim — comments and key order are not reformatted.
3. Click **Save**. On success the file is written and the page live-reloads.
4. Press `Esc` or click outside the modal to close it.

### Editing media

To embed an image, video, audio clip, or PDF, click **Insert media** in the
footer and pick a file. To change an embed that's already in the document,
**double-click it** — the media picker reopens prefilled with its current file
and caption so you can swap the file or edit the caption. You can also place the
cursor on (or next to) an embed and use the footer button: when an embed is the
current target it reads **Edit media** instead of **Insert media**.

### Error handling

| Result | Meaning |
|--------|---------|
| Authentication required | A token is required (an `edit_token_hash` is configured, the caller is remote, or `edit_require_token_on_loopback`). Enter it and retry. |
| Editing is disabled or blocked | Editing is off, or the request failed the CSRF/same-origin/`Host` check. |
| File changed on disk | The file was modified since you loaded it (e.g. by the watcher or another editor). Reload the page before saving. |

## Creating, renaming, and moving files

Beyond editing an existing file's contents, the editor can also create new
markdown files, create folders, and rename or move files. These operations are
**server/GUI-mode only** and gated by the same access checks as editing (see
[Security model](#security-model)); the editor does **not** delete files.

- **New file** — create a markdown file at a chosen destination path. Missing
  parent folders can be created as part of the request.
- **New folder** — create an (empty) directory. Creating an existing folder is a
  no-op, so it is safe to retry.
- **Rename / move** — move a markdown file to a new path (a rename is just a move
  to a new filename in the same folder). Moving a file **automatically rewrites
  links across the whole repository** so nothing breaks:
  - Inbound links to the moved page — inline `[text](path)`, reference
    definitions `[ref]: path`, and path-style `[[dir/note]]` wiki links — are
    updated, preserving each link's original style (absolute vs relative vs
    `./`), any `.md`/trailing slash, `#anchor`, `?query`, and wiki `|display`.
  - The moved file's own relative links are re-expressed against its new folder.
  - On a **rename** (the filename stem changes), bare `[[Name]]` wiki links that
    resolved to the file are rewritten to the new name. Bare links that resolve
    to a *different* note are left untouched.

  The response reports how many pages were rewritten.

### File-management endpoints

All three require the same `X-MBR-Edit: 1` header, same-origin, and (where
applicable) token as `/.mbr/edit`. The `{*path}` segment is a repo-relative
**filesystem** path *with* extension (e.g. `docs/new.md`) — the same convention
as `/.mbr/raw` and `/.mbr/edit` — and each returns the computed `url_path` in
its JSON response.

| Endpoint | Body | Purpose |
|----------|------|---------|
| `POST /.mbr/create/{*path}` | `{ "content": "…", "create_dirs": false }` | Create a new markdown file (409 if it already exists). |
| `POST /.mbr/move/{*path}` | `{ "to": "docs/new.md", "create_dirs": false }` | Move/rename `{*path}` to `to`, rewriting links repo-wide (409 on collision). |
| `POST /.mbr/mkdir/{*path}` | *(none)* | Create a folder (idempotent). |

Set `create_dirs: true` to create any missing parent directories at the
destination; otherwise a missing parent is rejected with `400`.

## Generating a token

For remote editing you need a shared token. Generate one:

```bash
mbr --generate-edit-token
```

You'll be prompted for a password (leave it blank to auto-generate a random
token). It prints the **token** (give this to whoever edits) and an
`edit_token_hash = "..."` line to paste into `.mbr/config.toml`. Nothing is
written to disk for you, and only the Argon2 hash is stored — never the token
itself.

```toml
# .mbr/config.toml
edit_enabled = true
edit_token_hash = "$argon2id$v=19$m=19456,t=2,p=1$...."
```

The editor UI has a token field (revealed automatically on an auth failure);
the token is kept only in memory for the page session and never persisted.

## Remote editing

mbr serves **plain HTTP with no built-in TLS**. The editing token is sent with
every request, so for any non-loopback use you should put mbr behind a
**TLS-terminating reverse proxy** (nginx, Caddy, etc.). Startup validation
refuses to enable editing on a non-loopback host (`--host` other than a
loopback address) unless an `edit_token_hash` is configured.

Behind such a proxy every request reaches mbr from 127.0.0.1, so peer-address
trust would silently disable authentication. A configured `edit_token_hash` is
therefore enforced for **all** callers regardless of the peer address — you do
not need `edit_require_token_on_loopback` to make the proxied deployment safe.
Because the proxy presents its own public `Host`, the `Host` check below is
skipped once a token is configured: the token is then the authority.

## Security model

- **CSRF protection (always on):** every editing request must carry an
  `X-MBR-Edit: 1` header and be same-origin. Browsers won't send that custom
  header cross-origin without a CORS preflight (which mbr never grants), so a
  malicious web page cannot silently write to a localhost mbr.
- **DNS-rebinding protection (when no token is configured):** the request's
  `Host` header must name `localhost`, a loopback IP, or the configured bind
  address; anything else is rejected with `403`. Same-origin alone does not stop
  rebinding — a rebound attacker page genuinely *is* same-origin — so the name
  used to reach the server is the deciding signal. This check is skipped when an
  `edit_token_hash` is configured, so a reverse proxy may present any `Host`.
- **Loopback trust (only without a token):** requests from 127.0.0.1 need no
  token *when no `edit_token_hash` is configured*. Set
  `edit_require_token_on_loopback = true` only if you want the token prompt in
  the local GUI as well — a configured token is enforced either way.
- **Token auth:** a request must present `Authorization: Bearer <token>`,
  verified against the Argon2 `edit_token_hash`, whenever a token is configured,
  the caller is non-loopback, or `edit_require_token_on_loopback` is set.
- **Upload restrictions:** `/.mbr/upload` accepts only image/audio/video/PDF (and
  `.vtt`/`.srt`) files, and refuses any destination inside the `.mbr/` template
  folder — the uploader can never create an HTML template, a script, or a
  stylesheet that later runs in the site.
- **Optimistic concurrency:** saves send a hash of the loaded content and are
  rejected with `409 Conflict` if the on-disk file changed, preventing silent
  lost updates.
- **Atomic writes:** files are written to a temp file and renamed into place.

See the [Configuration Reference](../reference/configuration/#editing-settings)
for the full list of options and the [CLI Reference](../reference/cli/) for the
flags.

## Keyboard shortcuts

| Key | Action |
|-----|--------|
| `e` | Open the editor for the current file (when editing is enabled) |
| `Esc` | Close the editor |
