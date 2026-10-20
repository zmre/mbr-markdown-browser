/**
 * `.mbr-control-group`: zoom and depth buttons in one rounded,
 * hairline-separated pill (Apple Maps' zoom control), for the mini graph and
 * the genealogy charts. Reads the same tokens as `controls.ts`; kept apart
 * from it because only lazy chunks draw these, and a `css` call is never
 * tree-shaken out of a bundle that imports its module. Pure: no state.
 */
import { css } from 'lit'

export const CONTROL_GROUP_CSS = css`
  .mbr-control-group {
    display: inline-flex;
    flex-direction: column;
    background: var(--pico-background-color, #fff);
    border: 1px solid var(--pico-muted-border-color, #e7eaf0);
    border-radius: var(--mbr-control-radius, 8px);
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.08);
    overflow: hidden;
  }

  .mbr-control-group.row {
    flex-direction: row;
  }

  .mbr-control-group > button {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 32px;
    height: 32px;
    margin: 0;
    padding: 0;
    border: 0;
    border-radius: 0;
    background: transparent;
    box-shadow: none;
    color: var(--mbr-icon-color, var(--pico-muted-color, #5c6b73));
    cursor: pointer;
    transition:
      background-color 120ms ease,
      color 120ms ease;
  }

  .mbr-control-group > button + button {
    border-top: 1px solid var(--pico-muted-border-color, #e7eaf0);
  }

  .mbr-control-group.row > button + button {
    border-top: 0;
    border-left: 1px solid var(--pico-muted-border-color, #e7eaf0);
  }

  @media (hover: hover) {
    .mbr-control-group > button:hover:not(:disabled) {
      background: var(--mbr-control-hover-bg, rgba(127, 127, 127, 0.12));
      color: var(--mbr-icon-color-hover, var(--pico-color, #373c44));
    }
  }

  .mbr-control-group > button:active:not(:disabled) {
    background: var(--mbr-control-press-bg, rgba(127, 127, 127, 0.2));
  }

  .mbr-control-group > button:focus-visible {
    outline: none;
    box-shadow: inset 0 0 0 2px var(--pico-primary, #0172ad);
  }

  .mbr-control-group > button:disabled {
    opacity: 0.4;
    cursor: default;
  }
`
