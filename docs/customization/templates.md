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

### Partial Templates

Partials are prefixed with underscore and included by other templates:

| Partial | Purpose |
|---------|---------|
| `_head.html` | Base `<head>` content |
| `_head_markdown.html` | Extended head for markdown |
| `_nav.html` | Navigation bar |
| `_info_panel.html` | Document info sidebar |
| `_footer.html` | Page footer |
| `_scripts.html` | Base JavaScript includes |
| `_scripts_markdown.html` | Markdown-specific scripts |

## Template Variables

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

  <main class="container">
    <article>
      {% if title %}
        <h1>{{ title }}</h1>
      {% endif %}

      {{ markdown | safe }}
    </article>

    {% include "_info_panel.html" %}
  </main>

  {% include "_footer.html" %}
  {% include "_scripts_markdown.html" %}
</body>
</html>
```

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

## Example: Custom Navigation

Create `.mbr/_nav.html`:

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
