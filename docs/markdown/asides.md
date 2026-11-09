---
title: Pull Quotes, Marginalia and Footnotes
description: Emphasized quotations, sidenotes in the margin, and footnotes with hover previews
order: 9
---

# Pull Quotes, Marginalia and Footnotes

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

## Footnotes

Add references that link to notes:

```markdown
Here is a statement that needs citation[^1].

[^1]: This is the footnote content.
```

### Live Example

mbr uses pulldown-cmark for markdown parsing[^1], which provides excellent CommonMark compliance and performance[^2].

Footnotes appear at the bottom of the page. On desktop (hover-capable devices),
hovering a footnote reference shows a preview card with the note's content;
clicking still jumps to the definition at the bottom.

[^1]: pulldown-cmark is a Rust library that parses markdown to events, allowing flexible rendering.

[^2]: The library uses SIMD optimizations for faster text processing.
