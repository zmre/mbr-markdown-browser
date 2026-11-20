---
title: Embedding the Markdown Field
description: Use mbr's markdown editor as a form field in another app
order: 3
---

# Embedding the Markdown Field

`<mbr-markdown-field>` is mbr's in-browser editor packaged as a form field
that another web app can embed. It runs the same editor core as
[in-browser editing](../features/editing.md), so markdown written in mbr's
syntax survives a round trip unchanged: wikilinks, alerts, math, definition
lists, task markers, tables and footnotes. Like the editor, it never writes HTML
the author did not type.

It differs from the editor in what it leaves out. It does no file I/O, has no
save button, no frontmatter form and no live reload, and reads nothing from the
page it sits on. **Your app owns the value:** set `value`, listen for `change`,
and store the markdown wherever you like.

## Loading it

The field ships in the mbr binary as one self-contained ES module:

```
/.mbr/components/mbr-markdown-field.min.js
```

Every mbr server serves it, in server and GUI mode. Static builds leave it out,
since no built page uses it. Importing the module defines the element, and it
does not need mbr's main bundle.

Load it from the same origin as your page. Either serve mbr's router inside your
app (an app that links mbr as a library merges `Server`'s router at its root),
or copy the file into your app's static assets.

```html
<script type="module" src="/.mbr/components/mbr-markdown-field.min.js"></script>

<mbr-markdown-field
  value="Thanks for your business."
  placeholder="Notes above the table"
  catalog-url="/.mbr/site.json"
></mbr-markdown-field>
```

The module is about 1.35 MB minified (about 420 KB gzipped), most of it the
editor engine (ProseMirror, Milkdown and Crepe). Load it lazily, when a form
that needs it opens. It defines the element only if no other copy has, so
loading it twice is harmless.

### In a SvelteKit app

Import the module on the client only, for example in `onMount`, because custom
elements do not exist during server-side rendering:

```svelte
<script>
  import { onMount } from 'svelte';

  let { notes = $bindable(''), onsave } = $props();

  onMount(() => {
    import(/* @vite-ignore */ '/.mbr/components/mbr-markdown-field.min.js');
  });
</script>

<mbr-markdown-field
  value={notes}
  placeholder="Notes above the table"
  catalog-url="/.mbr/site.json"
  onchange={(event) => (notes = event.detail.value)}
  onblur={() => onsave?.(notes)}
></mbr-markdown-field>
```

You can write each `change` back into `value`, as this example does. When the
field receives the value it just sent, it keeps the editor, the caret and the
undo history as they are. A *different* value replaces the document.

## Attributes and properties

| Property | Attribute | Type | Default | Meaning |
|----------|-----------|------|---------|---------|
| `value` | `value` | string | `''` | The markdown. Reading it is always current, even while a `change` is still pending. Setting it replaces the document and fires no `change`. The attribute is read whenever it changes but never written back. |
| `readonly` | `readonly` | boolean | `false` | The reader can select and copy but not edit. Reflected, and mirrored to `aria-readonly`. |
| `placeholder` | `placeholder` | string | `''` | Shown while the field is empty. |
| `catalogUrl` | `catalog-url` | string | `''` | Where the `[[` autocomplete gets its notes (see [Wikilink autocomplete](#wikilink-autocomplete)). Empty means no autocomplete and no network requests. |
| `linkBase` | `link-base` | string | `/` | The URL of the note being edited. The autocomplete's relative-link mode builds its links from here. |
| `uploadImage` | (none) | `(file: File) => Promise<string>` | `null` | Stores a pasted, dropped or picked image and returns the URL to write. Without one, image uploads are refused, so the field never writes a temporary `blob:` URL into your markdown. |
| `ready` | (none) | `Promise<void>`, read-only | | Resolves when the editor for the current value, placeholder and catalog has mounted. |

`focus()` puts the caret in the editor.

A property set before the module loads is kept. Frameworks sometimes set
properties on an element before its class is defined, and the field picks them
up when it connects.

## Events

| Event | Type | When |
|-------|------|------|
| `change` | `CustomEvent<{ value: string }>`, bubbles | The reader edited the document. Events are spaced about 120 ms apart while typing, and a pending one is always sent before `blur`. None is sent when the value is unchanged (an edit undone) or when you set `value`. |
| `blur` | `Event` | Focus left the field. Moving focus into the field's own toolbar, menus or popovers does not count. |

Native `change` events from the editor's internal inputs, such as the image
URL field, are stopped inside the element. Any `change` your listener receives
is the field's.

## What the field writes

- **Untouched means unchanged.** Until the reader edits, `value` returns
  exactly what you set.
- **Edits stay local.** Once the reader edits, blocks they did not touch are
  still written as you set them. This is the editor's minimal-diff save path,
  so one edit does not reformat the rest of the text.
- **Line endings and the end of the text** follow the value you set: CRLF in,
  CRLF out, and a missing final newline stays missing.
- **No HTML.** An empty paragraph is written as nothing, not as `<br />`, as in
  the full editor.
- **The value is body text.** A leading `---` block is a thematic break here,
  not frontmatter. Frontmatter is your app's to manage.

## Wikilink autocomplete

With `catalog-url` set, typing `[[` opens a list of notes, filtered as you
type. Enter inserts `[[Name]]`, and Tab switches to a relative markdown link
built from `link-base`. The URL is fetched with same-origin credentials each
time the list opens, so a note created a moment ago appears. It must return
either of these:

- site.json, as mbr serves it at `/.mbr/site.json` (`{"markdown_files": [...]}`);
- a bare array of the same entries: `[{"url_path": "/customers/acme/", "frontmatter": {"title": "Acme Corp"}}]`.

A note's link name is its `title`, falling back to its file name, as
mbr resolves wikilinks when it renders.

## Theming

The field lives in your page's light DOM. Its stylesheet is injected once into
`<head>`, and every rule is scoped under `mbr-markdown-field`, so nothing leaks
into your page. Set these custom properties on the element, or on any ancestor,
to match your theme:

| Variable | Used for | Default |
|----------|----------|---------|
| `--mbr-field-background` | Field background | `--pico-background-color`, else `#ffffff` |
| `--mbr-field-color` | Text | `--pico-color`, else `#1b1c1d` |
| `--mbr-field-muted-color` | Secondary text, toolbar icons | `--pico-muted-color`, else `#646b79` |
| `--mbr-field-border-color` | Field border, rules, table lines | `--pico-muted-border-color`, else `#d5d9e0` |
| `--mbr-field-primary` | Links, accents, the focus ring | `--pico-primary`, else `#1d5fbf` |
| `--mbr-field-surface` | Toolbar, menus and popovers | `--pico-card-background-color`, else `#ffffff` |
| `--mbr-field-surface-low` | Secondary surfaces | `--pico-card-sectioning-background-color`, else `#f4f5f8` |
| `--mbr-field-hover-background` | Hovered menu items | `--pico-secondary-hover-background`, else `#eceef4` |
| `--mbr-field-selection-background` | Selected text and nodes | `--mbr-field-primary` at 22% |
| `--mbr-field-code-background` | Code background | `--pico-code-background-color`, else `#f1f3f6` |
| `--mbr-field-code-color` | Inline code text | `--pico-code-color`, else `#4a4f5c` |
| `--mbr-field-error-color` | Error hints | `--pico-del-color`, else `#b3261e` |
| `--mbr-field-font-family` | Text font | `--pico-font-family`, else the system UI font |
| `--mbr-field-font-family-monospace` | Code font | `--pico-font-family-monospace`, else the system monospace font |
| `--mbr-field-font-size` | Base text size (headings scale from it) | `1rem` |
| `--mbr-field-placeholder-color` | Placeholder text | `--mbr-field-color` at 45% |
| `--mbr-field-focus-color` | Border and ring while focused | `--mbr-field-primary` |
| `--mbr-field-focus-ring-width` | Width of the focus ring | `2px` |
| `--mbr-field-border-width` | Field border width | `1px` |
| `--mbr-field-radius` | Field corner radius | `0.375rem` |
| `--mbr-field-padding` | Space around the text | `0.625rem 0.875rem` |
| `--mbr-field-min-height` | Minimum height | `6rem` |
| `--mbr-field-readonly-background` | Background while `readonly` | `--mbr-field-background` |

Each variable falls back to the matching [Pico](themes.md) variable, so on a
page styled by mbr's own theme the field matches with no extra CSS. Alerts use
the theme's `--mbr-alert-<kind>-bg`, `--mbr-alert-<kind>-border` and
`--mbr-alert-<kind>-icon-color` (`note`, `tip`, `important`, `warning`,
`caution`) when they are set.

```css
/* A dark host theme */
.invoice-form mbr-markdown-field {
  --mbr-field-background: #1d2026;
  --mbr-field-color: #e6e8ee;
  --mbr-field-muted-color: #9aa1ad;
  --mbr-field-border-color: #343a46;
  --mbr-field-primary: #7aa2f7;
  --mbr-field-surface: #262a33;
  --mbr-field-code-background: #262a33;
  --mbr-field-min-height: 4rem;
}
```

> [!NOTE]
> The full editor's block handle (the `+` and drag grip beside each block)
> needs a gutter about 80px wide, which a form field does not have, so the
> field hides it. Type `/` at the start of an empty line for the same block
> menu: headings, lists, quotes, tables, definition lists and more.
