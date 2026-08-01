---
title: CSS Theming
description: Customize colors and styles
---

# CSS Theming

mbr uses [Pico CSS](https://picocss.com/) as its base framework, making customization straightforward through CSS variables.

## Theme Selection

The `theme` configuration option lets you select a pre-built Pico CSS color variant without writing any CSS:

```toml
# .mbr/config.toml
theme = "jade"
```

### Theme Options

| Value | Description |
|-------|-------------|
| `default` or empty | Default Pico classless theme (blue accent) |
| `{color}` | Color variant (e.g., `amber`, `jade`, `violet`) |
| `fluid` | Fluid typography that scales with viewport width |
| `fluid.{color}` | Fluid typography with color variant (e.g., `fluid.purple`) |

### Available Colors

19 color variants are available:

- **Warm**: amber, orange, pumpkin, red, pink, fuchsia
- **Cool**: blue, cyan, indigo, violet, purple
- **Natural**: green, jade, lime, yellow
- **Neutral**: grey, slate, sand, zinc

### Examples

```toml
# Amber accent color
theme = "amber"

# Purple with fluid typography
theme = "fluid.purple"

# Minimal grey theme
theme = "grey"
```

### Fluid Typography

Fluid themes use responsive typography that scales smoothly with viewport width, providing optimal reading experience across devices without hard breakpoints:

```toml
theme = "fluid.jade"
```

> **Note:** Theme selection only changes the base Pico CSS. You can further customize using `user.css` (see below).

### Command Line Override

You can override the theme at runtime using the `--theme` flag:

```bash
# Server mode with amber theme
mbr -s --theme amber .

# Static build with fluid purple theme
mbr -b --theme fluid.purple .

# GUI mode with jade theme
mbr -g --theme jade .
```

Valid `--theme` values: `default`, `fluid`, or any color name: `amber`, `blue`, `cyan`, `fuchsia`, `green`, `grey`, `indigo`, `jade`, `lime`, `orange`, `pink`, `pumpkin`, `purple`, `red`, `sand`, `slate`, `violet`, `yellow`, `zinc`. Prefix with `fluid.` for fluid typography (e.g., `--theme fluid.amber`).

This is useful for testing themes or building a site with a different theme than the config file specifies.

## Quick Customization

For simple style additions, create `.mbr/user.css`:

```css
/* .mbr/user.css */
:root {
  --pico-primary: #8B5CF6;
  --pico-primary-hover: #7C3AED;
}

h1 {
  border-bottom: 2px solid var(--pico-primary);
  padding-bottom: 0.5rem;
}
```

This file is loaded **after** the default theme, so your rules take precedence.

## Full Theme Override

To completely replace the default theme, create `.mbr/theme.css`:

```css
/* .mbr/theme.css */
/* This replaces the default theme entirely */

:root {
  /* Your complete theme definition */
}
```

When `theme.css` exists, mbr uses it instead of the built-in theme.

## Pico CSS Variables

### Primary Colors

```css
:root {
  --pico-primary: #1095c1;           /* Primary color */
  --pico-primary-hover: #0d7a9e;     /* Primary hover state */
  --pico-primary-focus: rgba(16, 149, 193, 0.25);
  --pico-primary-inverse: #fff;       /* Text on primary background */
}
```

### Background Colors

```css
:root {
  --pico-background-color: #fff;      /* Page background */
  --pico-card-background-color: #fff; /* Card backgrounds */
  --pico-card-border-color: #e5e5e5;  /* Card borders */
}
```

### Text Colors

```css
:root {
  --pico-color: #373c44;              /* Main text */
  --pico-muted-color: #646b79;        /* Secondary text */
  --pico-h1-color: #1b1f24;           /* Heading colors */
  --pico-h2-color: #24292e;
}
```

### Typography

```css
:root {
  --pico-font-family: system-ui, -apple-system, sans-serif;
  --pico-font-size: 100%;             /* Base font size */
  --pico-line-height: 1.5;
  --pico-font-weight: 400;
}
```

## mbr-Specific Variables

mbr adds custom CSS variables for its unique features:

### Pull Quotes

```css
:root {
  --mbr-pullquote-font-size: 1.25rem;
  --mbr-pullquote-border-color: var(--pico-primary);
  --mbr-pullquote-border-width: 4px;
}
```

### Marginalia (Sidenotes)

```css
:root {
  --mbr-marginalia-font-size: 0.875rem;
  --mbr-marginalia-color: var(--pico-muted-color);
  --mbr-marginalia-marker: "†";
}
```

### GitHub Alerts

```css
:root {
  --mbr-alert-note-bg: #e7f3ff;
  --mbr-alert-note-border: #58a6ff;
  --mbr-alert-tip-bg: #e6ffed;
  --mbr-alert-tip-border: #3fb950;
  --mbr-alert-important-bg: #f3e8ff;
  --mbr-alert-important-border: #a855f7;
  --mbr-alert-warning-bg: #fff8e6;
  --mbr-alert-warning-border: #d29922;
  --mbr-alert-caution-bg: #ffe7e7;
  --mbr-alert-caution-border: #f85149;
}
```

### Definition Lists (FAQ)

Definition lists render as a [click-to-expand FAQ](../markdown/#definition-lists-faq-style).
These are the defaults — every color resolves through a Pico variable, so light
and dark mode and all the color themes follow automatically. Override the
variables rather than the rules and you keep that for free:

```css
:root {
  /* Question (dt) */
  --mbr-dl-question-color: var(--pico-color);
  --mbr-dl-question-hover-color: var(--pico-primary-hover);
  --mbr-dl-question-weight: 600;

  /* Answer (dd) */
  --mbr-dl-answer-color: var(--pico-muted-color);
  --mbr-dl-answer-indent: calc(var(--pico-spacing) * 1.5);

  /* Separator rule drawn above each question */
  --mbr-dl-separator-color: var(--pico-muted-border-color);

  /* Disclosure marker (the chevron that rotates when open) */
  --mbr-dl-marker-color: var(--pico-primary);
  --mbr-dl-marker-size: 0.62em;
  --mbr-dl-marker-gap: 0.55em;

  /* Vertical breathing room around each question and under each answer */
  --mbr-dl-item-spacing: var(--pico-spacing);

  /* Expand/collapse animation */
  --mbr-dl-transition-duration: 0.25s;
  --mbr-dl-transition-easing: ease;
}
```

Some worked examples:

```css
:root {
  /* Tighter list with a louder question */
  --mbr-dl-item-spacing: 0.5rem;
  --mbr-dl-question-weight: 700;

  /* Accent the marker instead of the text */
  --mbr-dl-marker-color: var(--pico-del-color);
  --mbr-dl-marker-size: 0.8em;

  /* Snap open with no animation */
  --mbr-dl-transition-duration: 0s;
}
```

To opt a repository out of the FAQ treatment entirely and get plain, always-open
definition lists back, override the rules themselves in `.mbr/user.css`. All
three selectors are needed: theme.css collapses answers with one low-specificity
rule plus two higher-specificity ones that re-close answers belonging to a
different question. `user.css` is linked after `theme.css`, so matching each
selector exactly is enough and no `!important` is required:

```css
main dl > dd,
main dl > dt:focus ~ dt ~ dd,
main dl > dd:focus-within ~ dt ~ dd {
  visibility: visible;
  height: auto;
  padding-block: 0 var(--mbr-dl-item-spacing);
}

/* Dropping the marker also means undoing its hanging indent, or the first
   line of every question hangs out into the margin. */
main dl > dt::before {
  content: none;
}

main dl > dt {
  cursor: auto;
  padding-inline-start: 0;
  text-indent: 0;
}
```

The height animation only runs where the browser can interpolate the `auto`
keyword (`interpolate-size: allow-keywords`). Elsewhere — Safari, at the time
of writing — answers simply snap open, and `prefers-reduced-motion: reduce`
drops the transitions everywhere.

## Printing

Printing (and "Save as PDF", which is the same code path) is governed by the
`@media print` block at the end of `theme.css`. Two rules define it.

### Nothing is hidden on paper

Everything mbr collapses, clamps or clips is an affordance for reading on a
screen — it trades content for skimmability on the understanding that you can
always open the thing back up. Paper takes that bargain away, so print undoes
all of it:

| Screen behavior | On paper |
|---|---|
| Sections collapsed by clicking a heading | Expanded; the `+` marker is dropped |
| Definition-list answers (click-to-expand FAQ) | Every answer open, chevrons dropped |
| A closed `<details>` written literally in the markdown | Forced open (see the caveat below) |
| Marginalia (`>>>` sidenotes) shown on hover | Printed inline where they occur |
| Oembed link cards clamped to 200px | Full height, nothing clipped |
| Kanban boards (`85vh`, side-scrolling columns) | Flattened to one column so no card is cut off |
| Theater-mode video figures sized to the viewport | Unconstrained, so the caption survives |
| Heading permalink anchors | Hidden — they only exist to be hovered |
| `<video>` / `media-player` | Replaced by `[Video content - see digital version]` |

The one thing that stays hidden is `.sr-only`, which is a *duplicate* of the
page title emitted for search weighting. Revealing it would print the title
twice.

Note that `[hidden]` content is deliberately left hidden. Unlike a section you
collapsed a moment ago, `hidden` is an explicit statement by the author that
the content does not apply.

#### Caveat: `<details>` needs a recent browser

CSS cannot set the `open` attribute, so forcing a closed `<details>` open on
paper depends on the `::details-content` pseudo-element:

```css
details::details-content {
  content-visibility: visible !important;
  block-size: auto !important;
  overflow: visible !important;
}
```

That requires **Chrome/Edge 131+, Safari 18.4+ or Firefox 143+** (Baseline
"newly available", September 2025); GUI mode uses the system WebKit, so it
tracks whichever Safari is installed. On anything older the rule is dropped and
a closed `<details>` prints closed — and there is no CSS-only workaround, which
is why the CSS Working Group issue asking for one stayed open for eight years.
If you must support older browsers, open the blocks by hand before printing, or
add a `beforeprint` listener in `.mbr/components/` that sets `open`.

The `!important` is aimed at your own `user.css`, which is linked *after*
`theme.css`. If you have copied MDN's accordion animation
(`details::details-content { block-size: 0; overflow: clip }`), it matches the
same selector, and `@media print` adds no specificity — without `!important`
source order would re-collapse the block on paper.

Two things this does not fix: the content is still reported as *collapsed* to
screen readers (the `open` attribute genuinely is absent), and print behavior
specifically is not covered by any cross-browser test suite, so it is worth one
manual print-preview check on your target browser.

### Only `<main>` is printed

Chrome — the header, breadcrumbs, footer and every `<mbr-*>` component — is
hidden by an allowlist rather than a list of things to hide:

```css
body > *:not(main, mbr-genealogy, .mbr-print-keep) {
  display: none !important;
}
```

Which components a page carries depends on the mode (server adds live reload,
GUI adds the find bar, the sidebar layout adds the navigation drawer), so any
hand-maintained list of tag names goes stale the moment a component is added —
and the failure only shows up on paper. Inverting the test makes new components
correct by default.

If your repository puts real content in `.mbr/_footer_custom.html`, mark it so
it survives:

```html
<aside class="mbr-print-keep">
  <p>© 2026 Example Corp. Internal use only.</p>
</aside>
```

The family chart on `type: person` pages (`<mbr-genealogy>`) is generated
content rather than chrome, so it is carved out and prints as-is.

### Paper margins (and the Safari bug behind them)

Margins come from padding on the content box, not from an `@page` margin:

```css
:root {
  --mbr-print-margin-block: 0.5in;  /* top and bottom */
  --mbr-print-margin-inline: 1in;   /* left and right */
}
```

Override those variables to change the margins. **Do not move the values onto
`@page`.** A non-zero `@page` margin makes Safari — and the GUI window, which
is a WKWebView — lay out a large empty band at the top of *every* printed page.
Chrome is unaffected. Zeroing the page box and moving the whitespace into the
content box is what removes it.

The band could also be cleared at print time by changing any setting in
Safari's print dialog (in either direction) or by turning on print-media
emulation in devtools — both force a re-layout. The GUI window exposes no
dialog controls, so there it was unavoidable.

The trade-off of the workaround: padding applies at the *start and end of the
box*, so a document longer than one sheet gets its top margin on page 1 and its
bottom margin on the last page, while continuation pages fall back to the
printer's own unprintable area. Left and right margins are unaffected and apply
to every page. If you print single-page documents, or your printer's hardware
margin is enough, this is invisible; if you need uniform vertical margins on a
long document and only ever print from Chrome, you can put them back:

```css
/* Chrome-only: uniform vertical margins, re-triggers the Safari band */
@media print {
  @page { margin: 0.5in 1in; }
  main.container { padding: 0; }
}
```

### Best results

For best results when printing, use light mode and enable "Print backgrounds"
in the browser's print dialog.

## Dark Mode

mbr respects the system color scheme preference. Add dark mode overrides:

```css
@media (prefers-color-scheme: dark) {
  :root {
    --pico-background-color: #1a1a1a;
    --pico-color: #e5e5e5;
    --pico-muted-color: #9ca3af;
    --pico-primary: #a78bfa;
    --pico-primary-hover: #c4b5fd;
  }
}
```

## Example Themes

### Purple Theme

```css
/* .mbr/user.css - Purple theme */
:root {
  --pico-primary: #8B5CF6;
  --pico-primary-hover: #7C3AED;
  --pico-primary-focus: rgba(139, 92, 246, 0.25);
}

@media (prefers-color-scheme: dark) {
  :root {
    --pico-primary: #A78BFA;
    --pico-primary-hover: #C4B5FD;
  }
}
```

### Warm Theme

```css
/* .mbr/user.css - Warm theme */
:root {
  --pico-primary: #EA580C;
  --pico-primary-hover: #C2410C;
  --pico-background-color: #FFFBEB;
}

h1, h2, h3 {
  color: #78350F;
}
```

### Minimal Theme

```css
/* .mbr/user.css - Minimal theme */
:root {
  --pico-primary: #171717;
  --pico-primary-hover: #404040;
}

article {
  max-width: 65ch;
  margin: 0 auto;
}

h1 { font-size: 1.5rem; }
h2 { font-size: 1.25rem; }
```

## Code Syntax Highlighting

mbr uses [highlight.js](https://highlightjs.org/) for code blocks. To customize:

```css
/* .mbr/user.css - Custom code colors */
.hljs {
  background: #1e1e1e;
  color: #d4d4d4;
}

.hljs-keyword { color: #569cd6; }
.hljs-string { color: #ce9178; }
.hljs-comment { color: #6a9955; }
```

Or use a different highlight.js theme by including it in your `.mbr/` folder.

## Component Styling

Style mbr's web components:

```css
/* Navigation bar */
mbr-nav {
  --mbr-nav-bg: var(--pico-card-background-color);
  --mbr-nav-border: var(--pico-card-border-color);
}

/* Browser panel */
mbr-browse {
  --mbr-browse-width: 300px;
}

/* Search */
mbr-search {
  --mbr-search-bg: var(--pico-background-color);
}
```

## Debugging Styles

Use browser developer tools (F12 or Cmd+Option+I) to:

1. Inspect computed styles
2. Test CSS changes live
3. Find the right CSS variables
4. Check specificity issues

## Best Practices

1. **Use CSS variables** - Easier to maintain and supports dark mode
2. **Start with `user.css`** - Only create `theme.css` for complete overhauls
3. **Test dark mode** - Check your changes in both color schemes
4. **Use semantic colors** - Reference Pico variables instead of hardcoded colors
5. **Keep specificity low** - Avoid `!important` when possible
