---
title: Presentation Slides
description: Create slide presentations from markdown
order: 5
---

# Presentation Slides

mbr can transform any markdown document into a presentation using [Reveal.js](https://revealjs.com/).

## Quick Start

Add `style: slides` to your frontmatter:

```yaml
---
title: My Presentation
style: slides
---
```

Use horizontal rules (`---`) to separate slides:

```markdown
---
title: My Presentation
style: slides
---

# Welcome

First slide content

---

## Second Slide

- Point one
- Point two

---

## Conclusion

Final slide
```

## Speaker Notes

Use triple blockquotes (`>>>`) for speaker notes visible only to the presenter (press `s` for speaker view):

```markdown
## Slide Title

Content visible to audience.

>>> These notes are only visible in the speaker view.
>>> Press 'S' to open the speaker notes window.
```

## Slide Attributes

Add attributes to individual slides using the section attributes syntax:

```markdown
--- {#intro .dark-bg data-transition="zoom"}

This slide has:
- ID: intro
- Class: dark-bg
- Transition: zoom

--- {.centered}

This slide has centered class
```

## Keyboard Shortcuts

| Key | Action |
|-----|--------|
| `->` / `Space` | Next slide |
| `<-` | Previous slide |
| `Esc` / `O` | Overview mode |
| `s` | Speaker notes |
| `g` | Speaker notes |
| `f` | Fullscreen |
| `?` | Help overlay |

## Tips

- **Images**: Use standard markdown image syntax
- **Code blocks**: Syntax highlighting works automatically
- **Math**: KaTeX equations render in slides
- **Mermaid**: Diagrams work in presentation mode

## Example

Use a [live example](test-slides/).

Create a test presentation:

```markdown
---
title: Test Presentation
style: slides
---

# Welcome

This is the title slide

>>> These are speaker notes for the title slide.
>>> Press 'S' to open the speaker notes window.

---

## Slide Two

- Point one
- Point two
- Point three

>>> Remember to emphasize point two!

---

## Code Example

\`\`\`rust
fn main() {
    println!("Hello slides!");
}
\`\`\`

--- {#conclusion .highlight}

## Conclusion

Thanks for watching!

>>> Final remarks: thank the audience.
```
