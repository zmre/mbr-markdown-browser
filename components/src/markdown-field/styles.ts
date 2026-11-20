/**
 * `<mbr-markdown-field>`'s own stylesheet: the field box and the theme bridge.
 *
 * The editor's CSS (Crepe's, mbr's plugins', the `[[` popup's) was written
 * against Pico's custom properties, because `<mbr-editor>` runs on mbr pages.
 * A host page has no Pico, and must not have to know Crepe's or Pico's names,
 * so theming goes through one documented vocabulary, `--mbr-field-*`
 * (`docs/customization/markdown-field.md`), in two steps:
 *
 * 1. On the element, each `--mbr-field-*` is resolved into a private
 *    `--mbr-field--*` token: the host's value, else the page's Pico variable
 *    (so the field matches an mbr page with no work), else a light default.
 * 2. On the element's wrapper, the Pico variables the editor reads are set
 *    from those tokens, so every rule inside, popovers included (they attach
 *    to the wrapper: `POPOVER_HOST_ATTR`), follows the host's theme.
 *
 * Two steps because a custom property cannot fall back to its own inherited
 * value (`--pico-color: var(--mbr-field-color, var(--pico-color))` is a
 * cycle, so invalid); resolving on the parent and assigning on the child is
 * not.
 *
 * Every selector starts at `mbr-markdown-field`, so nothing here reaches the
 * host's own page.
 */
export const FIELD_CSS = `
mbr-markdown-field {
  display: block;
  --mbr-field--background: var(--mbr-field-background, var(--pico-background-color, #ffffff));
  --mbr-field--color: var(--mbr-field-color, var(--pico-color, #1b1c1d));
  --mbr-field--muted: var(--mbr-field-muted-color, var(--pico-muted-color, #646b79));
  --mbr-field--border: var(--mbr-field-border-color, var(--pico-muted-border-color, #d5d9e0));
  --mbr-field--primary: var(--mbr-field-primary, var(--pico-primary, #1d5fbf));
  --mbr-field--surface: var(--mbr-field-surface, var(--pico-card-background-color, #ffffff));
  --mbr-field--surface-low: var(--mbr-field-surface-low, var(--pico-card-sectioning-background-color, #f4f5f8));
  --mbr-field--hover: var(--mbr-field-hover-background, var(--pico-secondary-hover-background, #eceef4));
  --mbr-field--selected: var(--mbr-field-selection-background, color-mix(in srgb, var(--mbr-field--primary) 22%, transparent));
  --mbr-field--code-background: var(--mbr-field-code-background, var(--pico-code-background-color, #f1f3f6));
  --mbr-field--code-color: var(--mbr-field-code-color, var(--pico-code-color, #4a4f5c));
  --mbr-field--error: var(--mbr-field-error-color, var(--pico-del-color, #b3261e));
  --mbr-field--font: var(--mbr-field-font-family, var(--pico-font-family, system-ui, -apple-system, 'Segoe UI', sans-serif));
  --mbr-field--mono: var(--mbr-field-font-family-monospace, var(--pico-font-family-monospace, ui-monospace, Menlo, Consolas, monospace));
  --mbr-field--focus: var(--mbr-field-focus-color, var(--mbr-field--primary));
}

mbr-markdown-field > .mbr-markdown-field {
  --pico-background-color: var(--mbr-field--background);
  --pico-color: var(--mbr-field--color);
  --pico-muted-color: var(--mbr-field--muted);
  --pico-muted-border-color: var(--mbr-field--border);
  --pico-primary: var(--mbr-field--primary);
  --pico-primary-focus: color-mix(in srgb, var(--mbr-field--primary) 18%, transparent);
  --pico-card-background-color: var(--mbr-field--surface);
  --pico-card-sectioning-background-color: var(--mbr-field--surface-low);
  --pico-secondary-background: var(--mbr-field--surface-low);
  --pico-secondary-inverse: var(--mbr-field--color);
  --pico-secondary-hover-background: var(--mbr-field--hover);
  --pico-contrast-background: var(--mbr-field--color);
  --pico-contrast-inverse: var(--mbr-field--background);
  --pico-code-background-color: var(--mbr-field--code-background);
  --pico-code-color: var(--mbr-field--code-color);
  --pico-del-color: var(--mbr-field--error);
  --pico-font-family: var(--mbr-field--font);
  --pico-font-family-monospace: var(--mbr-field--mono);

  box-sizing: border-box;
  position: relative;
  min-height: var(--mbr-field-min-height, 6rem);
  background: var(--mbr-field--background);
  color: var(--mbr-field--color);
  border: var(--mbr-field-border-width, 1px) solid var(--mbr-field--border);
  border-radius: var(--mbr-field-radius, 0.375rem);
  transition: border-color 0.15s ease, box-shadow 0.15s ease;
}

mbr-markdown-field:focus-within > .mbr-markdown-field {
  border-color: var(--mbr-field--focus);
  box-shadow: 0 0 0 var(--mbr-field-focus-ring-width, 2px) color-mix(in srgb, var(--mbr-field--focus) 25%, transparent);
}

mbr-markdown-field[readonly] > .mbr-markdown-field {
  background: var(--mbr-field-readonly-background, var(--mbr-field--background));
}

mbr-markdown-field .milkdown {
  --crepe-base-font-size: var(--mbr-field-font-size, 1rem);
  --crepe-color-selected: var(--mbr-field--selected);
  background: transparent;
  min-height: inherit;
}

mbr-markdown-field .milkdown .ProseMirror {
  /* Crepe's page-sized 60px 120px would swallow a form field. */
  padding: var(--mbr-field-padding, 0.625rem 0.875rem);
  min-height: var(--mbr-field-min-height, 6rem);
  box-sizing: border-box;
}

mbr-markdown-field .milkdown .ProseMirror > :first-child { margin-top: 0; }
mbr-markdown-field .milkdown .ProseMirror > :last-child { margin-bottom: 0; }

/* The block handle (+ and drag grip) hangs ~80px left of each block, in a
   gutter a form field does not have. The block menu is still one "/" away. */
mbr-markdown-field .milkdown .milkdown-block-handle { display: none; }

/* The placeholder shows only while the whole field is empty (mode: doc). */
mbr-markdown-field .milkdown .crepe-placeholder::before {
  color: var(--mbr-field-placeholder-color, color-mix(in srgb, var(--mbr-field--color), transparent 55%));
}
`;
