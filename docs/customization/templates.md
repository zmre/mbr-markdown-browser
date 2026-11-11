---
title: HTML Templates
description: Customize page structure
---

# HTML Templates

mbr uses [Tera](https://tera.netlify.app/) for templating. Customize any template by creating a file with the same name in your `.mbr/` folder.

## Template Files

| Template | Purpose | When Used |
|----------|---------|-----------|
| `index.html` | Markdown page layout | Rendering `.md` files |
| `section.html` | Directory listing | Subdirectory pages |
| `home.html` | Home page | Root directory |
| `error.html` | Error pages | 404 Not Found, server errors |

### Partial Templates

Partials are prefixed with underscore and included by other templates:

| Partial | Purpose |
|---------|---------|
| `_head.html` | Base `<head>` content |
| `_head_markdown.html` | Extended head for markdown |
| `_nav.html` | Navigation bar |
| `_breadcrumbs.html` | Breadcrumb bar |
| `_info_panel.html` | Document info sidebar |
| `_footer.html` | Page footer |
| `_scripts.html` | Base JavaScript includes |
| `_display_enhancements.html` | Display-enhancement loaders (mermaid, hljs; `<mbr-genealogy>` on person and organization pages) |
| `_contact_card.html` | Contact card at the top of `type: person` / `type: organization` pages, from the `contact` variable — see [Contacts](../features/contacts/#customizing) |
| `_calendar_nav.html` | Periodic-note navigation: the prev / up / next bar at the top of `<main>` on a daily, weekly, monthly, quarterly or yearly note, from the `calendar_nav` variable — see [Calendar](../features/calendar/#navigating-periodic-notes). The `[` / `]` keys follow its `rel="prev"` / `rel="next"` links |
| `_person_infobox.html` | Compatibility shim: renders the contact card without its headline, for custom `index.html` files written before the card existed |
| `_page_header_links.html` | Your links at the right-hand end of the page header (after search). Empty by default — see [Adding your own links](#adding-your-own-links) |
| `_page_footer_links.html` | Your links in the page footer of markdown, directory, home, tag and media pages. Empty by default |
| `_browse_footer.html` | Your links at the bottom of the browse sidebar (both `sidebar_style`s). Empty by default |

## Template Variables

### Global Variables (All Templates)

These variables are available in all templates:

| Variable | Type | Description |
|----------|------|-------------|
| `server_mode` | bool | `true` when running with live server (`-s` or `-g`), `false` for static builds |
| `gui_mode` | bool | `true` when running in native window (`-g`), `false` for browser or static |
| `relative_base` | string | Path prefix to `.mbr/` assets (e.g., `../../.mbr/` for static builds) |
| `relative_root` | string | Path prefix to site root (e.g., `../../` for static builds) |

**Mode combinations:**

| Mode | `server_mode` | `gui_mode` |
|------|---------------|------------|
| GUI (`-g`) | `true` | `true` |
| Server (`-s`) | `true` | `false` |
| Static build (`-b`) | `false` | `false` |

### Markdown Pages (`index.html`)

| Variable | Type | Description |
|----------|------|-------------|
| `markdown` | string | Rendered HTML content |
| `title` | string | From frontmatter or filename |
| `description` | string | From frontmatter |
| `date` | string | From frontmatter |
| `tags` | string | From frontmatter |
| `breadcrumbs` | string | Navigation breadcrumbs HTML |
| `headings` | string | Table of contents data (JSON) |
| `current_dir_name` | string | Current directory name |
| `current_path` | string | Current URL path |
| `frontmatter_json` | string | All frontmatter as JSON |
| `calendar_enabled` | bool | `[calendar] enabled`; gates `<mbr-calendar>` in `_footer.html` and the pre-paint script in `_head.html` |
| `calendar_nav` | object | Only on a [periodic note](../features/calendar/#periodic-notes): `period` (canonical key, e.g. `2026-10-31`, `2026-W44`), `kind` (`day`/`week`/`month`/`quarter`/`year`), `label` (English, e.g. `Saturday, October 31, 2026`), `datetime` (for `<time datetime>`; absent for a quarter), `prev` / `next` (the nearest notes of the same kind, or absent) and `up` (the containing week / month / quarter / year notes that exist). Each link has `kind`, `period`, `url` (page-relative in static builds), `label`, `title` and `datetime`. Drawn by `_calendar_nav.html` |

### Directory Pages (`section.html`, `home.html`)

| Variable | Type | Description |
|----------|------|-------------|
| `current_dir_name` | string | Directory name |
| `current_path` | string | Directory URL path |
| `parent_path` | string | Parent directory URL |
| `breadcrumbs` | string | Navigation breadcrumbs HTML |
| `subdirs` | string | Subdirectory list (JSON) |
| `files` | string | File list (JSON) |
| `is_home` | bool | True if root directory |

### Error Pages (`error.html`)

| Variable | Type | Description |
|----------|------|-------------|
| `error_code` | number | HTTP status code (e.g., 404) |
| `error_title` | string | Short error title (e.g., "Not Found") |
| `error_message` | string | Detailed error message |
| `requested_url` | string | The URL that was requested |
| `breadcrumbs` | array | Navigation breadcrumbs |

### File/Directory Data Structure

The `files` JSON array contains:

```json
[
  {
    "url_path": "/docs/guide/",
    "name": "guide.md",
    "title": "User Guide",
    "description": "Getting started...",
    "date": "2025-01-09",
    "tags": "guide, docs"
  }
]
```

The `subdirs` JSON array contains:

```json
[
  {
    "url_path": "/docs/",
    "name": "docs"
  }
]
```

## Tera Syntax

### Variables

```html
{{ title }}
{{ markdown | safe }}  <!-- safe filter for HTML -->
```

### Conditionals

```html
{% if title %}
  <h1>{{ title }}</h1>
{% elif current_dir_name %}
  <h1>{{ current_dir_name }}</h1>
{% else %}
  <h1>Untitled</h1>
{% endif %}
```

### Loops

```html
{% for file in files %}
  <a href="{{ file.url_path }}">{{ file.title }}</a>
{% endfor %}
```

### Includes

```html
{% include "_head.html" %}
```

### Icons

The `icon()` function draws one of mbr's built-in icons (the [Lucide](https://lucide.dev) set the rest of the interface uses) as an inline SVG in the current text colour:

```html
<a href="/" role="button" class="outline secondary mbr-icon-label">{{ icon(name="house", size=16) | safe }}Home</a>
```

`name` is required; `size` is in CSS pixels and defaults to 20. The names available are the files in `crates/mbr-core/templates/icons/` (for example `chevron-left`, `chevron-right`, `arrow-up`, `house`, `info`, `search`, `x`, `user`, `building-2`). An unknown name is a template error rather than an empty space. Icons are decorative (`aria-hidden`), so give the surrounding link or button its own text or `aria-label`. The `mbr-icon-label` class puts an 8px gap between an icon and its label.

## Common Gotchas

### Chained Defaults Don't Work

```html
<!-- BAD: Fails if variable doesn't exist -->
{{ title | default(value=current_dir_name) | default(value="") }}

<!-- GOOD: Use conditionals -->
{% if title %}{{ title }}{% elif current_dir_name %}{{ current_dir_name }}{% endif %}
```

### Check Variable Existence

```html
{% if description %}
  <meta name="description" content="{{ description }}">
{% endif %}
```

### JSON in JavaScript

```html
<script>
  const frontmatter = {{ frontmatter_json | safe }};
  const headings = {{ headings | safe }};
</script>
```

### JavaScript Configuration Object

The `_head.html` partial exposes a global `window.__MBR_CONFIG__` object for use by JavaScript:

```javascript
window.__MBR_CONFIG__ = {
  serverMode: true,     // false for static builds
  guiMode: true,        // true only in native window (-g)
  searchEndpoint: "/.mbr/search",
  basePath: "",         // relative path prefix for static builds
  calendar: {           // the browser's half of the [calendar] table (snake_case), on every page
    enabled: true, date_fields: ["date", "created"],
    week_start: "locale", companion: "auto", agenda_chips: true
  }
};
```

Use this in custom scripts to adapt behavior:

```javascript
// Only show certain features in GUI mode
if (window.__MBR_CONFIG__?.guiMode) {
  // Native window - no browser URL bar visible
  document.querySelector('.url-display')?.classList.remove('hidden');
}

// Conditionally enable features requiring server
if (window.__MBR_CONFIG__?.serverMode) {
  initLiveSearch();
}
```

## Example: Custom Page Template

Create `.mbr/index.html`:

```html
<!DOCTYPE html>
<html lang="en">
<head>
  {% include "_head_markdown.html" %}
  <title>{% if title %}{{ title }} - {% endif %}My Notes</title>
</head>
<body>
  {% include "_nav.html" %}
  {% include "_breadcrumbs.html" %}

  <main class="container">
    {% include "_calendar_nav.html" %}
    <article>
      {% if title %}
        <h1>{{ title }}</h1>
      {% endif %}

      {{ markdown | safe }}
    </article>

    {% include "_info_panel.html" %}
  </main>

  {% include "_footer.html" %}
  {% include "_scripts.html" %}
</body>
</html>
```

**Periodic-note navigation in a custom `index.html`.** The bar on daily,
weekly and other periodic notes is the `_calendar_nav.html` partial, which the
built-in `index.html` includes at the top of `<main>`. A custom `.mbr/index.html`
written before it existed does not include it, so add
`{% include "_calendar_nav.html" %}` inside your `<main>` (as above) to get the
bar and the `[` / `]` keys. Keep it inside `<main>` rather than the header: the
iOS app hides the header.

**The daily-note companion in a custom template.** On a day note (with
`[calendar] companion = "auto"`) mbr appends `mbr-has-companion` to the `style`
variable, so a template that renders `<body class="{{ style }}">` reserves the
right-hand column with no change. The column itself is the `<mbr-calendar>`
element `_footer.html` emits (when `calendar_enabled` is true); it must stay a
direct child of `<body>`, which it is as long as your template includes
`_footer.html` there. `_head.html` carries a one-line script that applies the
reader's dismissal before the first paint; a custom head should include
`_head.html` rather than copy it.

## Example: Add Analytics

Create `.mbr/_footer.html`:

```html
<footer>
  <p>Built with mbr</p>
</footer>

<!-- Analytics (only in production) -->
{% if not is_dev %}
<script async src="https://www.googletagmanager.com/gtag/js?id=GA_ID"></script>
<script>
  window.dataLayer = window.dataLayer || [];
  function gtag(){dataLayer.push(arguments);}
  gtag('js', new Date());
  gtag('config', 'GA_ID');
</script>
{% endif %}
```

## Adding Your Own Links

To put a link — text or an icon: a GitHub mark pointing at the repository, a home page, a social profile — in the page header, the page footer or the bottom of the browse sidebar, override one of three small partials rather than copying `_nav.html` or a page template. They are empty by default, so overriding one replaces nothing of mbr's, and the partials around them keep getting mbr's future fixes and features. A copied `_nav.html` would not.

| Partial | Where it appears | What to write |
|---------|------------------|---------------|
| `.mbr/_page_header_links.html` | Right-hand end of the header, after the search button | One `<li>` per link |
| `.mbr/_page_footer_links.html` | Page footer, on its own centred row | One `<li>` per link |
| `.mbr/_browse_footer.html` | Bottom of the browse sidebar: below the list in the docked sidebar (`sidebar_style = "single"`), at the end of the drawer otherwise | The links themselves, no `<ul>`/`<li>` |

The footer row and the sidebar strip appear only when the partial holds something, so an empty or comment-only file draws nothing. Error pages show the header links but have no page footer or sidebar.

**A brand icon (GitHub).** mbr's built-in icons are [Lucide](https://lucide.dev), which has no brand marks, so paste the mark as inline SVG. `fill="currentColor"` makes it follow the theme (light and dark), and a link holding only an icon needs an `aria-label`, since the SVG itself is hidden from screen readers:

```html
{# .mbr/_page_header_links.html #}
<li>
  <a href="https://github.com/you/your-repo" aria-label="Source on GitHub" title="Source on GitHub" rel="noopener">
    <svg viewBox="0 0 16 16" width="20" height="20" fill="currentColor" aria-hidden="true" focusable="false">
      <path d="M8 0c4.42 0 8 3.58 8 8a8.013 8.013 0 0 1-5.45 7.59c-.4.08-.55-.17-.55-.38 0-.27.01-1.13.01-2.2 0-.75-.25-1.23-.54-1.48 1.78-.2 3.65-.88 3.65-3.95 0-.88-.31-1.59-.82-2.15.08-.2.36-1.02-.08-2.12 0 0-.67-.22-2.2.82-.64-.18-1.32-.27-2-.27-.68 0-1.36.09-2 .27-1.53-1.03-2.2-.82-2.2-.82-.44 1.1-.16 1.92-.08 2.12-.51.56-.82 1.28-.82 2.15 0 3.06 1.86 3.75 3.64 3.95-.23.2-.44.55-.51 1.07-.46.21-1.61.55-2.33-.66-.15-.24-.6-.83-1.23-.82-.67.01-.27.38.01.53.34.19.73.9.82 1.13.16.45.68 1.31 2.69.94 0 .67.01 1.3.01 1.49 0 .21-.15.45-.55.38A7.995 7.995 0 0 1 0 8c0-4.42 3.58-8 8-8Z"/>
    </svg>
  </a>
</li>
```

In the header and footer, a link holding an SVG is drawn like mbr's own header buttons: a 36px box in the icon colour that fills on hover. An SVG with no `width` gets 20px, the size of the built-in icons. The header has room for one or two icons on a phone; the page title gives way first.

**A plain text link** needs nothing special:

```html
{# .mbr/_page_footer_links.html #}
<li><a href="https://example.com">example.com</a></li>
<li><a href="https://mastodon.social/@you" rel="me noopener">Mastodon</a></li>
```

**A built-in icon** comes from `icon()` (see [Icons](#icons)):

```html
{# .mbr/_page_header_links.html #}
<li><a href="https://example.com" aria-label="Home page">{{ icon(name="house") | safe }}</a></li>
```

**The sidebar** takes bare links, laid out in a row that wraps. An icon beside text is fine there:

```html
{# .mbr/_browse_footer.html #}
<a href="https://github.com/you/your-repo" rel="noopener"><svg viewBox="0 0 16 16" width="16" height="16" fill="currentColor" aria-hidden="true" focusable="false"><path d="…the GitHub path above…"/></svg>GitHub</a>
<a href="https://example.com">Home page</a>
```

**Links into your own site** must work both from the server and from a static build (`-b`), where pages sit at different depths and the site may be served from a sub-path. Start them with `relative_root`, which a static build sets to the way back to the site root (`""`, `../`, `../../`…) and the server leaves unset:

```html
<li><a href="{{ relative_root | default(value="/") | safe }}about/">About</a></li>
```

That gives `/about/` from the server and `../../about/` on a page two folders deep in a static build. Keep the `| safe`: Tera escapes the `default` value itself, and without `safe` it is escaped a second time into a broken link.

**Styling.** Override the defaults in `.mbr/user.css`. The header links are `body > header nav > ul > li > a`, the footer links `nav.mbr-footer-links a`, and the sidebar links `mbr-browse > a` (drawer) and `mbr-browse-single > a` (docked sidebar) — they are ordinary page elements, so page CSS reaches them even though the sidebar itself is a web component:

```css
/* .mbr/user.css */
nav.mbr-footer-links a { font-size: 0.875rem; }
:is(mbr-browse, mbr-browse-single) > a { color: var(--pico-primary); }
```

The iOS app hides mbr's header for its own bar, so header links do not appear there; footer and sidebar links do.

## Example: Custom Navigation

To add a few links to the header, prefer `_page_header_links.html` ([Adding your own links](#adding-your-own-links)): a copied `_nav.html` no longer picks up changes to mbr's header. To replace the header outright, create `.mbr/_nav.html`:

```html
<nav>
  <ul>
    <li><a href="/">Home</a></li>
    <li><a href="/docs/">Documentation</a></li>
    <li><a href="/blog/">Blog</a></li>
  </ul>

  {{ breadcrumbs | safe }}

  <mbr-search></mbr-search>
</nav>
```

## Example: Remove Breadcrumbs

Add an empty `.mbr/_breadcrumbs.html`.

## Example: Directory Listing

Create `.mbr/section.html`:

```html
<!DOCTYPE html>
<html lang="en">
<head>
  {% include "_head.html" %}
  <title>{{ current_dir_name }} - My Notes</title>
</head>
<body>
  {% include "_nav.html" %}

  <main class="container">
    <h1>{{ current_dir_name }}</h1>

    {% if subdirs %}
    <h2>Folders</h2>
    <ul>
      {% for dir in subdirs %}
      <li><a href="{{ dir.url_path }}">{{ dir.name }}/</a></li>
      {% endfor %}
    </ul>
    {% endif %}

    {% if files %}
    <h2>Files</h2>
    <ul>
      {% for file in files %}
      <li>
        <a href="{{ file.url_path }}">{{ file.title | default(value=file.name) }}</a>
        {% if file.description %}
          <p>{{ file.description }}</p>
        {% endif %}
      </li>
      {% endfor %}
    </ul>
    {% endif %}
  </main>

  {% include "_footer.html" %}
  {% include "_scripts.html" %}
</body>
</html>
```

## Example: Custom Error Page

Create `.mbr/error.html` to customize the 404 and error page experience:

```html
<!DOCTYPE html>
<html lang="en">
<head>
  {% include "_head.html" %}
  <title>{{ error_code }} - {{ error_title }}</title>
</head>
<body>
  {% include "_nav.html" %}

  <main class="container">
    <div class="error-page">
      <h1>{{ error_code }}</h1>
      <h2>{{ error_title }}</h2>

      {% if error_message %}
        <p>{{ error_message }}</p>
      {% endif %}

      {% if requested_url %}
        <p>Looking for: <code>{{ requested_url }}</code></p>
      {% endif %}

      <nav>
        <button onclick="history.back()">Go Back</button>
        {% if relative_root %}
          <a href="{{ relative_root }}">Home</a>
        {% else %}
          <a href="./">Home</a>
        {% endif %}
      </nav>

      <p>Try searching for what you need:</p>
      <mbr-search></mbr-search>
    </div>
  </main>

  {% include "_footer.html" %}
  {% include "_scripts.html" %}
</body>
</html>
```

### Error Pages and Static Builds

When you build a static site with `mbr -b`, a `404.html` file is automatically generated at the root of your output directory. This file is used by:

- **GitHub Pages** - Automatically serves `404.html` for missing pages
- **Netlify** - Recognizes `404.html` as the custom error page
- **Most static hosts** - Follow the same convention

The generated `404.html` uses relative paths so it works correctly regardless of where your site is deployed (root domain or subdirectory).

**Important:** In static mode, the `requested_url` variable is not available since the 404 page is pre-generated. The error page shows a generic message instead.

## Development Workflow

1. Copy the default template from mbr's source
2. Modify your copy in `.mbr/`
3. Reload the page to see changes
4. Use browser DevTools to debug

For rapid iteration, use the `--template-folder` flag:

```bash
mbr -s --template-folder ./my-templates ~/notes
```

This loads templates from an external folder, keeping your repository clean during development.
