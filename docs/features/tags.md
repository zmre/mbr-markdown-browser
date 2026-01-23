---
title: Tags
description: Browse and navigate content by tags extracted from frontmatter
---

# Tags

mbr automatically extracts tags from frontmatter and generates browsable tag pages. This enables wiki-style tag navigation without any special syntax in your markdown files.

## Quick Start

1. Add tags to your markdown frontmatter:

```yaml
---
title: Getting Started with Rust
tags: [rust, programming, tutorial]
---
```

2. Navigate to `/tags/` to see all tags
3. Click any tag to see all pages with that tag

## How It Works

### Tag Extraction

mbr extracts tags from configurable frontmatter fields during repository scanning. By default, it looks for a `tags` field, but you can configure your own source or sources.

**Supported formats:**

```yaml
# YAML array
tags: [rust, programming, tutorial]

# YAML list
tags:
  - rust
  - programming
  - tutorial

# Comma-separated string
tags: rust, programming, tutorial
```

### URL Structure

Tag pages follow the pattern `/{source}/{value}/`:

| URL | Description |
|-----|-------------|
| `/tags/` | List of all tags |
| `/tags/rust/` | Pages tagged with "rust" |
| `/authors/` | List of all authors (custom source) |
| `/authors/patrick_walsh/` | Pages with author "Patrick Walsh" |

### Normalization

Tag values are normalized for URLs but preserve display formatting:

| Original | URL Value | Display |
|----------|-----------|---------|
| `Rust` | `rust` | Rust |
| `Joshua Jay` | `joshua_jay` | Joshua Jay |
| `C++` | `c++` | C++ |

The first occurrence of a tag determines its display form.

### File Precedence

Real files always take precedence over generated tag pages. If you have `/tags/index.md`, it will be served instead of the generated tag index.

## Wikilink Syntax

mbr supports wikilink-style tag references that automatically transform into navigable links:

### Double-Bracket Syntax

```markdown
Check out [[Tags:rust]] for Rust-related content.
```

Renders as: Check out [rust](/tags/rust/) for Rust-related content.

### Explicit Link Syntax

```markdown
[Rust programming](Tags:rust)
```

Renders as: [Rust programming](/tags/rust/)

### Syntax Rules

- Source names are case-insensitive: `[[Tags:rust]]` = `[[TAGS:rust]]`
- Unknown sources are left unchanged: `[[Unknown:value]]` remains as-is
- URL-like patterns are not matched: `https://example.com` stays unchanged

## Configuration

### Basic Configuration

In `.mbr/config.toml`:

```toml
# Default configuration (extract from "tags" field)
tag_sources = [
    { field = "tags" }
]

# Generate tag pages in static builds (default: true)
build_tag_pages = true
```

### Multiple Tag Sources

You can configure multiple frontmatter fields as tag sources:

```toml
tag_sources = [
    { field = "tags" },
    { field = "category" },
    { field = "performers", label = "Performer", label_plural = "Performers" }
]
```

### Nested Frontmatter Fields

Use dot-notation to extract tags from nested YAML structures:

```yaml
---
title: Magic Performance
taxonomy:
  tags: [magic, performance]
  performers: [Joshua Jay, Penn and Teller]
---
```

```toml
tag_sources = [
    { field = "taxonomy.tags" },
    { field = "taxonomy.performers", label = "Performer", label_plural = "Performers" }
]
```

### Custom Labels

Each tag source can have custom singular and plural labels:

```toml
tag_sources = [
    { field = "tags" },  # Uses default: "Tag" / "Tags"
    { field = "performers", label = "Performer", label_plural = "Performers" },
    { field = "categories", label = "Category", label_plural = "Categories" }
]
```

If not specified, labels are auto-derived from the field name (title-cased, with 's' removed for singular).

## Static Site Generation

When building a static site with `mbr -b`, tag pages are generated automatically:

```bash
mbr -b ~/notes
```

Output structure:

```
build/
├── tags/
│   ├── index.html          # Tag index
│   ├── rust/
│   │   └── index.html      # Pages tagged "rust"
│   └── programming/
│       └── index.html      # Pages tagged "programming"
├── performers/
│   ├── index.html          # Performer index
│   └── joshua_jay/
│       └── index.html      # Pages with performer "Joshua Jay"
└── .mbr/
    └── site.json           # Includes tag data
```

### Disabling Tag Pages

To skip tag page generation in static builds:

```toml
build_tag_pages = false
```

### Tag Data in site.json

The generated `site.json` includes tag source information:

```json
{
  "tag_sources": {
    "tags": {
      "label": "Tag",
      "label_plural": "Tags",
      "tags": [
        {
          "normalized": "rust",
          "display": "Rust",
          "count": 5,
          "url": "/tags/rust/"
        }
      ]
    }
  }
}
```

## Template Customization

### Tag Page Template (tag.html)

Displays pages with a specific tag. Available context variables:

| Variable | Description |
|----------|-------------|
| `tag_source` | URL identifier (e.g., "tags") |
| `tag_display_value` | Display name (e.g., "Rust") |
| `tag_label` | Singular label (e.g., "Tag") |
| `tag_label_plural` | Plural label (e.g., "Tags") |
| `page_count` | Number of pages |
| `pages` | Array of page objects |

Each page object contains:
- `url_path`: Link to the page
- `title`: Page title
- `description`: Page description (optional)

### Tag Index Template (tag_index.html)

Displays all tags for a source. Available context variables:

| Variable | Description |
|----------|-------------|
| `tag_source` | URL identifier |
| `tag_label` | Singular label |
| `tag_label_plural` | Plural label |
| `tag_count` | Number of unique tags |
| `tags` | Array of tag objects |

Each tag object contains:
- `url_value`: Normalized value for URL
- `display_value`: Display name
- `page_count`: Number of pages

### Custom Templates

Create custom templates in `.mbr/`:

```html
<!-- .mbr/tag.html -->
{% set asset_base = relative_base | default(value="/.mbr/") %}
<!doctype html>
<html lang="en">
<head>
{% include "_head.html" %}
  <title>{{ tag_label }}: {{ tag_display_value }}</title>
</head>
<body>
{% include "_nav.html" %}
  <main class="container">
    <h1>{{ tag_display_value }}</h1>
    <p>{{ page_count }} pages</p>

    {% for page in pages %}
    <article>
      <a href="{{ page.url_path }}">{{ page.title }}</a>
      {% if page.description %}<p>{{ page.description }}</p>{% endif %}
    </article>
    {% endfor %}
  </main>
</body>
</html>
```

## Examples

### Simple Blog Tags

```yaml
---
title: My First Post
tags: [blog, introduction]
---
```

### Nested Taxonomy

```yaml
---
title: Performance Review
taxonomy:
  tags: [magic, review]
  performers: [Joshua Jay]
  venues: [Chicago Magic Lounge]
---
```

```toml
# .mbr/config.toml
tag_sources = [
    { field = "taxonomy.tags" },
    { field = "taxonomy.performers", label = "Performer" },
    { field = "taxonomy.venues", label = "Venue" }
]
```

### Linking to Tags

```markdown
See all [[Tags:magic]] performances.

Related performers: [[taxonomy.performers:Joshua Jay]].
```

## Troubleshooting

### Tags not appearing

1. Verify frontmatter syntax is valid YAML
2. Check that `tag_sources` includes your field name
3. Ensure case matches for nested fields (e.g., `taxonomy.tags` not `Taxonomy.Tags`)

### Wikilinks not transforming

1. Confirm the source name matches a configured `tag_sources` field
2. Check for typos in the source name
3. Verify the wikilink syntax: `[[Source:value]]` not `[[Source:value]` (missing bracket)

### Static build missing tag pages

1. Verify `build_tag_pages = true` in config (default)
2. Check that tags exist in at least one markdown file
3. Look for errors in build output
