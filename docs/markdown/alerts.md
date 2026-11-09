---
title: Alerts (Callouts)
description: GitHub-style alert boxes and the remark-hint shorthand
order: 3
---

# Alerts (Callouts)

Use callout boxes for important information:

```markdown
> [!NOTE]
> Helpful information that users should know.
```

Available box types: `[!NOTE]`, `[!TIP]`, `[!IMPORTANT]`, `[!WARNING]`, `[!CAUTION]`.

## Live Examples

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

## remark-hint Shorthand

mbr also recognizes the [remark-hint](https://github.com/sergioramos/remark-hint) shorthand. A paragraph that begins with one of these markers is rendered as the matching GitHub-style alert, with the marker stripped from the displayed text:

| Marker | Renders as |
|--------|-----------|
| `!> ` | Tip |
| `?> ` | Warning |
| `x> ` | Caution |

```markdown
!> A helpful tip.
?> Something to watch out for.
x> A risky operation.
```

This shorthand is always on (no configuration required). The marker must appear at the very start of a paragraph and be followed by a space; otherwise the text is left untouched.
