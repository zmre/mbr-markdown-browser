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
