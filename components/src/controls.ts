/**
 * The shared chrome for mbr's icon buttons, in one place so the header
 * triggers, panel close buttons, toolbars and floating buttons cannot drift
 * apart again (they had five visual languages in one header row).
 *
 * - {@link ICON_BUTTON_CSS} — `.mbr-icon-btn`: a borderless "ghost" button.
 *   Transparent at rest, a soft fill on hover and press, the open state taken
 *   from `aria-expanded`/`aria-pressed` (never a separate class), and nothing
 *   that changes size, so nothing shifts. `.mbr-icon-btn.sm` is the 28px
 *   variant for inline and on-canvas buttons. Labeled ghost buttons
 *   (`.mbr-icon-btn.labeled`) put an 8px gap between icon and text.
 * - {@link KBD_CSS} — `kbd`: the quiet key cap (hairline, faint fill,
 *   body-coloured text, UI font) shared by the shortcut help and the panel
 *   footers, reading the same `--mbr-kbd-*` tokens as theme.css's page `kbd`.
 *   `--mbr-kbd-size` sets the size per context.
 * - {@link DISCLOSURE_CSS} — `.chevron`: a disclosure chevron that rotates a
 *   quarter turn inside any `[aria-expanded="true"]` ancestor.
 * - `control-group.ts`'s `CONTROL_GROUP_CSS` — zoom/depth pills; a separate
 *   module because only lazy chunks use it, and Lit's `css` calls are not
 *   tree-shaken out of the main bundle.
 *
 * Touch (`max-width: 600px` or `pointer: coarse`): a 44pt-tall target, 40
 * wide in a row (contiguous, so the hit areas still meet), which keeps seven
 * header buttons and a title on a 390px phone.
 *
 * Every rule reads the theme.css tokens (`--mbr-icon-color`,
 * `--mbr-control-hover-bg`, …) and restates a fallback, because a repository's
 * own `.mbr/theme.css` may predate them. Pure: no state.
 */
import { css } from 'lit'

export const ICON_BUTTON_CSS = css`
  .mbr-icon-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 0.5rem;
    flex: none;
    box-sizing: border-box;
    width: 36px;
    height: 36px;
    min-width: 0;
    margin: 0;
    padding: 0;
    border: 0;
    border-radius: var(--mbr-control-radius, 8px);
    background: transparent;
    box-shadow: none;
    color: var(--mbr-icon-color, var(--pico-muted-color, #5c6b73));
    font: inherit;
    line-height: 1;
    cursor: pointer;
    transition:
      background-color 120ms ease,
      color 120ms ease;
    -webkit-tap-highlight-color: transparent;
  }

  .mbr-icon-btn.sm {
    width: 28px;
    height: 28px;
    border-radius: calc(var(--mbr-control-radius, 8px) - 2px);
  }

  .mbr-icon-btn.labeled {
    width: auto;
    padding-inline: 0.625rem;
    font-size: 0.875rem;
    font-weight: 500;
    white-space: nowrap;
  }

  @media (hover: hover) {
    .mbr-icon-btn:hover:not(:disabled) {
      background: var(--mbr-control-hover-bg, rgba(127, 127, 127, 0.12));
      color: var(--mbr-icon-color-hover, var(--pico-color, #373c44));
    }
  }

  .mbr-icon-btn:active:not(:disabled) {
    background: var(--mbr-control-press-bg, rgba(127, 127, 127, 0.2));
  }

  .mbr-icon-btn[aria-expanded='true'],
  .mbr-icon-btn[aria-pressed='true'] {
    background: var(--mbr-control-press-bg, rgba(127, 127, 127, 0.2));
    color: var(--mbr-icon-color-active, var(--pico-primary, #0172ad));
  }

  .mbr-icon-btn:focus-visible {
    outline: none;
    box-shadow: var(
      --mbr-focus-ring,
      0 0 0 2px var(--pico-background-color, #fff),
      0 0 0 4px var(--pico-primary, #0172ad)
    );
  }

  .mbr-icon-btn:disabled {
    opacity: 0.4;
    cursor: default;
  }

  .mbr-icon-btn > .mbr-icon {
    flex: none;
  }

  @media (max-width: 600px), (pointer: coarse) {
    .mbr-icon-btn {
      width: 40px;
      height: 44px;
    }

    .mbr-icon-btn.labeled {
      width: auto;
    }

    .mbr-icon-btn.sm {
      width: 36px;
      height: 36px;
    }
  }
`

export const DISCLOSURE_CSS = css`
  .chevron {
    flex: none;
    color: var(--mbr-icon-color, var(--pico-muted-color, #5c6b73));
    transition: transform 150ms ease;
  }

  [aria-expanded='true'] > .chevron,
  [aria-expanded='true'] .chevron.follows,
  .chevron.open {
    transform: rotate(90deg);
  }

  @media (prefers-reduced-motion: reduce) {
    .chevron {
      transition: none;
    }
  }
`

export const KBD_CSS = css`
  kbd {
    display: inline-block;
    padding: 0.0625rem 0.3125rem;
    border: 1px solid var(--mbr-kbd-border, color-mix(in srgb, currentColor 22%, transparent));
    border-radius: 4px;
    background: var(--mbr-kbd-bg, color-mix(in srgb, currentColor 5%, transparent));
    box-shadow: none;
    color: var(--mbr-kbd-color, var(--pico-color, inherit));
    font-family: inherit;
    font-size: var(--mbr-kbd-size, 0.6875rem);
    font-weight: 500;
    line-height: 1.4;
  }
`
