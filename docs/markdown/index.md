---
title: Markdown Extensions
description: Extended markdown features in mbr
---

# Markdown Extensions

mbr uses [pulldown-cmark](https://github.com/raphlinus/pulldown-cmark) with all extensions enabled, plus additional features for richer content.

## Standard Extensions

These are pulldown-cmark's built-in extensions:

| Extension | Syntax | Example |
|-----------|--------|---------|
| Tables | GFM tables | `\| Col1 \| Col2 \|` |
| Footnotes | `[^1]` references | `Text[^1]` + `[^1]: Note` |
| Strikethrough | `~~text~~` | ~~deleted text~~ |
| Task lists | `- [ ]` / `- [x]` | Checkboxes in lists |
| Smart punctuation | `"quotes"`, `--` | Curly quotes, em-dashes |
| Heading attributes | `# Title {#id}` | Custom anchor IDs |
| Autolinks | `<https://...>` | Clickable URLs |

## YAML Frontmatter

Add metadata to any markdown file:

```yaml
---
title: My Document
description: A helpful guide
tags: documentation, guide
date: 2025-01-09
author: Your Name
custom_field: Any value
---

# Content starts here
```

### Using Frontmatter

Frontmatter powers:
- **Page titles**: Browser tab and heading
- **Descriptions**: Search results and previews
- **Tags**: Navigation and filtering
- **Custom fields**: Available in templates

### Supported Fields

| Field | Purpose |
|-------|---------|
| `title` | Page title |
| `description` | Meta description |
| `tags` | Comma-separated tags |
| `date` | Publication date |
| `author` | Author name |
| Any field | Available via `frontmatter_json` |

## GitHub-style Alerts

Use callout boxes for important information:

```markdown
> [!NOTE]
> Helpful information that users should know.
```

Available box types: `[!NOTE]`, [!TIP], `[!IMPORTANT]`, `[!WARNING]`, `[!CAUTION]`.
`

### Live Examples

> [!NOTE]
> Helpful information that users should know, like tips for getting the most out of mbr.

> [!TIP]
> Optional advice to help users succeed. Try pressing `-` to open the file browser!

> [!IMPORTANT]
> Key information users need to know. mbr requires no special directory structure.

> [!WARNING]
> Urgent info that needs immediate attention. Back up your files before bulk operations.

> [!CAUTION]
> Advises about risks or negative outcomes. Running with `--template-folder` overrides local `.mbr/` settings.

## Canceled Task Items

In addition to standard checkboxes, mbr supports canceled items:

```markdown
- [ ] Unchecked task
- [x] Completed task
- [-] Canceled task
```

### Live Example

- [ ] Write documentation
- [x] Set up project structure
- [-] Use complex build system (not needed!)

## Pull Quotes

Use double `>>` for emphasized quotations:

```markdown
>> This important quote stands out from the surrounding text.
```

### Live Example

>> The goal of mbr is simple: take any collection of markdown files and make them instantly browsable, searchable, and publishable -- without requiring special syntax or directory structures.

Pull quotes render with larger font size, italic styling, and a distinctive left border.

## Marginalia (Sidenotes)

On wide screens, marginalia appear in the right margin. On narrow screens, they appear as a dagger (†) that reveals content on hover/click.

Use triple `>>>` for margin notes:

```markdown
Main paragraph text that readers focus on.

>>> This aside provides supplementary context.

Continuation of the main content.
```

### Live Example

mbr's marginalia feature is inspired by Tufte CSS and academic publishing traditions where sidenotes provide additional context without interrupting the flow of the main text.

>>> Edward Tufte popularized sidenotes in his books on data visualization. They allow readers to absorb supplementary information at their own pace.


## Mermaid Diagrams

Code blocks with `mermaid` language render as diagrams:

````markdown
```mermaid
graph LR
    A[Start] --> B{Decision}
    B -->|Yes| C[Result 1]
    B -->|No| D[Result 2]
```
````

Renders as:

```mermaid
graph LR
    A[Start] --> B{Decision}
    B -->|Yes| C[Result 1]
    B -->|No| D[Result 2]
```

### Supported Diagram Types

- Flowcharts (`graph` or `flowchart`)
- Sequence diagrams (`sequenceDiagram`)
- Class diagrams (`classDiagram`)
- State diagrams (`stateDiagram`)
- Gantt charts (`gantt`)
- Pie charts (`pie`)
- And more...

See [Mermaid documentation](https://mermaid.js.org/) for full syntax.

## Syntax Highlighting

Code blocks are highlighted using highlight.js:

### Live Examples

```rust
fn main() {
    let message = "Hello from mbr!";
    println!("{}", message);
}
```

```python
def greet(name: str) -> str:
    """Return a friendly greeting."""
    return f"Hello, {name}!"
```

```javascript
const render = async (markdown) => {
  const html = await mbr.parse(markdown);
  document.body.innerHTML = html;
};
```

### Supported Languages

`bash`, `javascript`, `typescript`, `python`, `ruby`, `rust`, `go`, `java`, `json`, `yaml`, `toml`, `html`, `css`, `sql`, `markdown`, and many more.

## Tables

GFM-style tables with alignment:

```markdown
| Left | Center | Right |
|:-----|:------:|------:|
| A    |   B    |     C |
| D    |   E    |     F |
```

| Left | Center | Right |
|:-----|:------:|------:|
| A    |   B    |     C |
| D    |   E    |     F |

## Footnotes

Add references that link to notes:

```markdown
Here is a statement that needs citation[^1].

[^1]: This is the footnote content.
```

### Live Example

mbr uses pulldown-cmark for markdown parsing[^1], which provides excellent CommonMark compliance and performance[^2].

[^1]: pulldown-cmark is a Rust library that parses markdown to events, allowing flexible rendering.

[^2]: The library uses SIMD optimizations for faster text processing.

Footnotes appear at the bottom of the page with back-links.

## Heading Anchors

Headers automatically get anchor IDs:

```markdown
## My Section
```

Links to `#my-section`. Override with explicit IDs:

```markdown
## My Section {#custom-id}
```

## Auto-linking

URLs in angle brackets become clickable:

```markdown
<https://example.com>
<user@example.com>
```

## See Also

- [Media Embedding](media/) - Videos, audio, PDFs, and more
