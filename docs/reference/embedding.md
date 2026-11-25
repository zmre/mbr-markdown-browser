---
title: Embedding mbr in an App
description: Use mbr's crates from another Rust program — serve its router, theme its pages, render notes from a string
order: 5.5
---

# Embedding mbr in an App

mbr is a set of Rust crates as well as an app. Another program can link them to
serve a repository's notes inside its own web server, apply its own theme to
mbr's pages, and render a note from text that has not been saved yet. Ledgeline
Invoicer works this way.

| Crate | What you use it for |
|-------|---------------------|
| `mbr-core` | Rendering: `mbr_core::standalone` (a note to a page), `mbr_core::host_head`, configuration (`mbr_core::Config`) |
| `mbr-server` | Serving: `mbr_server::server::{Server, ServerConfig}` and the `Server.router` to merge into yours |

Neither crate pulls in the GUI (wry, tao) or the command line.

## Serving a repository inside your server

Build a `ServerConfig` from the repository's configuration, start a `Server`,
and merge its axum router into yours. mbr's URLs are absolute (`/.mbr/…`,
`/notes/x/`), so mount it at the root and put your own routes under a prefix
of yours (`/app`, `/api`):

```rust
use mbr_core::Config;
use mbr_server::server::{Server, ServerConfig};

let config = Config::read(&root)?; // the repository's .mbr/config.toml + MBR_* env
let server = Server::init(ServerConfig::from(&config))?;
let app = axum::Router::new()
    .nest("/app", my_spa())
    .nest("/api", my_api())
    .merge(server.router.clone());
```

## Theming mbr pages

Two `ServerConfig` fields add markup to the `<head>` of every page the server
renders. Nothing is written into the repository.

```rust
let mut server_config = ServerConfig::from(&config);
server_config.theme_vars.insert(
    "--pico-primary".into(),
    "light-dark(#7c3aed, #a78bfa)".into(),
);
server_config.theme_vars.insert("--mbr-sidebar-width".into(), "300px".into());
server_config.extra_head_html =
    Some(r#"<link rel="stylesheet" href="/app/mbr-overrides.css">"#.into());
let server = Server::init(server_config)?;
```

- **`theme_vars`** (`BTreeMap<String, String>`) are CSS custom properties: any
  of [Pico's](../customization/themes/#pico-css-variables) or
  [mbr's](../customization/themes/#mbr-specific-variables). They are written
  as one rule, `<style id="mbr-host-theme">:root { --pico-primary: … !important; }</style>`.
  `!important` makes them win over Pico (whose colours sit on selectors that
  outrank a plain `:root`), `theme.css` and the repository's own `user.css`.
  For separate light and dark colours, write CSS
  [`light-dark()`](https://developer.mozilla.org/en-US/docs/Web/CSS/color_value/light-dark):
  it follows the colour scheme Pico sets, including the reader's system
  preference.
- **`extra_head_html`** (`Option<String>`) is any HTML you want in the head: a
  stylesheet, a `<meta>`, a script. It is emitted exactly as given, after the
  theme style, so it can override the variables too. Treat it as your own
  code: never put text from a repository or a user in it.

Both default to nothing; `ServerConfig::from` leaves them empty.

### What a theme value may contain

`Server::init` refuses any variable whose name or value could break out of its
declaration, its rule or the `<style>` element, and returns
`ServerError::HostHead` naming the variable:

- a name is `--` followed by ASCII letters, digits, `-` and `_`, at most 128
  bytes;
- a value is at most 1024 bytes, has no `<`, `>`, `{`, `}`, `;`, `!`, `\`,
  control characters or comments (`/*`, `*/`), and its quotes and parentheses
  are balanced. `rgb(1 2 3 / 50%)`, `"Inter", sans-serif` and
  `calc(100% - var(--x, 2px))` are fine.

### Where the markup appears

The default `_head.html` calls the template function `host_head()` as its last
line, after mbr's stylesheets, and every page template includes `_head.html`:
notes, the home page and folder listings, tag pages, media viewers, the
outside-link page and error pages.

- **A repository that overrides `_head.html`** in its `.mbr/` gets the theme
  only if its copy keeps `{{ host_head() }}`.
- **A note with its own [page template](../features/custom-page-templates/)**
  gets it when the template includes mbr's head
  (`{% include "/.mbr/_head.html" %}`). A template with a head of its own can
  call `{{ host_head() }}` itself. A template that does neither — an invoice
  meant for a customer — gets no host markup, which is usually what you want:
  the document should look the same whatever the app's theme is.
- **Static builds, QuickLook and `mbr -o`** have no host: `host_head()` renders
  nothing there.

`host_head()` is a function rather than a template variable on purpose. Every
frontmatter field is also a template variable, so a variable could be replaced
by a note's own `host_head:` field.

`LiveServer` (the desktop app's server, which rebuilds itself when
`config.toml` changes) builds its `ServerConfig` from the configuration file
alone, so it carries no host markup. An embedding app uses `Server` directly.

## Serving notes written by someone else

mbr's pages are served from your origin. A note or an `.html` file in the
repository that someone else wrote (a customer's shared note, a contract synced
from elsewhere) can contain `<script>`. By default mbr writes a note's HTML out
as written and serves a repository `.html` file as it is, so that script runs
with your origin's privileges: it can read your app's pages and call your API.
Two settings change that, and both are off unless you turn them on.

### Content Security Policy

`ServerConfig::content_security_policy` adds a `Content-Security-Policy` header
to each response, with one value per route class
(`mbr_core::csp::ContentClass`):

| Field | Responses |
|-------|-----------|
| `pages` | HTML mbr renders from the repository: notes, the home page, folder listings, tag pages, error pages |
| `user_files` | Repository files served as they are: static files (an `.html` file included), media, and a repository's own `.mbr/` files (`user.css`, overridden components, `.mbr/*.html`) |
| `mbr_assets` | mbr's own UI under `/.mbr/`: built-in CSS and scripts, `site.json` and the other JSON endpoints, the media viewers, the outside-link page |

`None` (the default) sends no header for that class. A value that is not a
valid header value fails `Server::init` with
`ServerError::ContentSecurityPolicy`. `ServerConfig::from` copies the
repository's `[content_security_policy]` table
([configuration](configuration/#content-security-policy)); whatever you assign
afterwards replaces it.

```rust
use mbr_core::csp::{ContentSecurityPolicy, SANDBOX};

let mut server_config = ServerConfig::from(&config);
server_config.content_security_policy = ContentSecurityPolicy {
    // mbr's own pages keep working: they need no inline script.
    pages: Some("default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: https:".into()),
    // A repository file opened in the browser runs nothing.
    user_files: Some(SANDBOX.into()), // "sandbox allow-same-origin"
    mbr_assets: None,
};
```

`SANDBOX` (`sandbox allow-same-origin`, no `allow-scripts`) is the preset for
content that must never run script. The document keeps its origin, so its
images and stylesheets still load, but no script, form, popup or top-level
navigation works. `ContentSecurityPolicy::sandbox_user_content()` sets it for
`pages` and `user_files` and leaves `mbr_assets` alone. On `pages` it stops
mbr's own components too (sidebar, search, editor, live reload): the note is
plain, read-only markup. That is your call; use a strict `script-src` on
`pages` instead to keep them.

### Strict `script-src` for mbr's pages

mbr's templates contain no inline script, no `on…=` handler attribute and no
`javascript:` URL, so its pages work under `script-src 'self'`
(`mbr_core::csp::STRICT_SCRIPT_SRC`) with no `'unsafe-inline'`. That covers
note pages (including the page `mbr --pdf` and Save as PDF… export), listings,
tag, error, media-viewer and outside-link pages.

The page data the components read (`window.__MBR_CONFIG__`,
`window.frontmatter`, `window.headings`, `window.extendedMeta`) is written as
`<script type="application/json" data-mbr-globals>` blocks, which `script-src`
does not govern, and turned into those globals by `/.mbr/mbr-bootstrap.js`
(`mbr_core::csp::BOOTSTRAP_SCRIPT`), a plain script loaded right after each
block, before any component runs. The globals and their timing are what they
were when the templates assigned them inline, so components and repository
templates that read them need no change.

`mbr_core::csp::TEMPLATE_INLINE_SCRIPT_HASHES` lists the `'sha256-…'` sources
of any inline script the default templates still carry, for a policy to name.
It is empty. `mbr_core::csp::strict_script_violations(html, hashes)` reports
what a strict `script-src` would block in a page, and
`mbr_core::csp::script_hash_source(text)` gives the hash source for an inline
script of your own.

Not covered:

- a repository's own `.mbr/` copy of `_head.html` (or another template) made
  before this change still has the inline scripts it was copied with; it keeps
  working without a policy;
- a note's [page template](../features/custom-page-templates/) may carry inline
  script of its own;
- HTML in a note, unless `raw_html` is `sanitize` or `escape` (below);
- a bare GitHub gist URL in a note, which embeds `gist.github.com`'s script
  under `passthrough`;
- `extra_head_html`: hash inline code you put there yourself, or load it from
  a URL.

mbr's pages also set `style` attributes and Lit's styles, so a policy that
restricts `style-src` needs `'unsafe-inline'` there. If your policy restricts
`style-src` with a hash instead, allow the theme's inline style by its hash:

```rust
use mbr_core::host_head::HostHead;

let head = HostHead::new(server_config.extra_head_html.as_deref(), &server_config.theme_vars)?;
let style_src = head.as_ref().and_then(HostHead::style_csp_source); // Some("'sha256-…'")
```

The hash depends only on the variables, so it is the same for every page.

### Raw HTML in notes

`ServerConfig::raw_html` (`mbr_core::raw_html::RawHtmlPolicy`) decides what a
page render does with HTML a note's author wrote:

- `Passthrough` (the default): written as is.
- `Sanitize`: formatting is kept, scripts, `on…` handlers, `style` attributes,
  frames, forms and script URLs are removed
  ([details](../markdown/#raw-html)).
- `Escape`: shown as text.

Only the author's HTML is affected: mbr's own markup (sections, media embeds,
task checkboxes, wikilinks) is untouched. `ServerConfig::from` takes `raw_html`
from the repository's configuration, which may raise it. To make sure the
repository cannot lower your choice, combine the two:

```rust
use mbr_core::raw_html::RawHtmlPolicy;

server_config.raw_html = server_config.raw_html.strictest(RawHtmlPolicy::Sanitize);
```

`mbr_core::standalone::Renderer` reads `raw_html` from the `Config` it is given.

## Rendering a note from a string

`mbr_core::standalone` renders a single note to a whole HTML page with no
server and no repository scan. It is what `mbr -o` prints, and it can render
text that is not on disk, such as the unsaved contents of an editor, for a live
preview:

```rust
use mbr_core::standalone::Renderer;
use std::path::Path;

let renderer = Renderer::new(&config)?; // loads the templates once
// On every edit:
let html = renderer.render_source(Path::new("invoices/2026-001.md"), &editor_text)?;
```

`render_source(relative_path, source)` gives the page `render_file` (and
`mbr -o`) would give for the file at `relative_path` holding `source`. It
produces the same HTML byte for byte:

- the note's own page template (`template:` in its frontmatter) is applied,
  found next to `relative_path` exactly as the server finds it. Template
  problems are errors (`MbrError::PageTemplate`), never a quiet fallback to the
  normal page;
- links and images are resolved relative to `relative_path`;
- the file does not have to exist yet.

The page is the standalone one, not the server's: absolute URLs, no sidebar
siblings or backlinks, no `data-mbr-line` source lines, and wikilinks left
unresolved. To resolve wikilinks, hand the renderer a running server's index,
which stays current as files change:

```rust
let renderer = Renderer::new(&config)?
    .with_wikilink_index(server.repo().wikilink_index.clone());
```

`render_source` is synchronous and never touches the network. A bare URL gets
an OpenGraph card only if this renderer's `render_file` already fetched it;
YouTube, Giphy and media embeds need no network and always work.

### Previewing an unsaved template

A template editor can preview the template it is editing before it is saved,
on every keystroke, with the same kept `Renderer`:

```rust
let html = renderer.render_source_with_template(
    Path::new("invoices/2026-001.md"),        // the note to preview with
    &note_text,                               // its text (saved or not)
    Path::new("/invoices/tpl/invoice.html"),  // the template being edited
    &template_editor_text,                    // its unsaved text
)?;
```

The note is rendered through `template_editor_text` as if it were the file at
the template path, whatever the note's own `template:` says. That path
follows the same rules as a `template:` value (inside the repository, not in
a hidden folder, an `.html` file) but the file need not exist yet, so a new
template can be previewed too. The `.html` files beside it, which it includes
or extends, and mbr's own templates still come from disk. Its own saved file
does not.

Nothing is cached. A template that does not parse or render is an
`MbrError::PageTemplate` error (`PageTemplateError::Render`) whose message
names the template and gives Tera's explanation, with the line and column of a
syntax error; show it in place of the preview. The next render, through the
saved template or a corrected draft, is unaffected. Each call costs what a
render through a saved template costs, less the template read, and never
scans the repository.

### Cost

A `Renderer` loads mbr's templates (`.mbr/` or the built-in ones) once. Each
render then costs the markdown render, one `stat` of the note and, for a note
with `template:`, reading and parsing that template and the `.html` files
beside it. Those are re-read on every render, so an edit to the template shows
at once. mbr's own templates are not re-read; call
`renderer.reload_templates()` after a `.mbr/` change. A `Renderer` is
`Send + Sync`, so one can serve every thread.

Measured on an Apple M-series Mac in a release build
(`cargo bench --no-default-features --bench standalone_render`):

| Render | Time |
|--------|------|
| A kept `Renderer`, a typical 2,000-word note | ~0.14 ms |
| A kept `Renderer`, an invoice through its page template | ~0.56 ms |
| The one-shot `render_note_source(config, rel, source)` | ~7 ms |

The one-shot functions `render_note_source` and `render_note` load the
templates on every call, so they suit a single render, not a preview that
updates on every keystroke.
