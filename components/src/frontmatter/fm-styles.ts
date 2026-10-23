/**
 * The properties form's stylesheet. The form renders into the light DOM like
 * the rest of the editor (its fields sit among the editor's own chrome and
 * use the page's Pico variables), so its rules are one `<style>` element in
 * `<head>`, injected once by the first form that connects.
 *
 * Every rule is scoped under `.mbr-fm` and written to outrank Pico's global
 * form rules (`input:not([type=checkbox],…)` and friends would otherwise make
 * every field a full-height, bottom-margined block). The shared icon-button
 * and disclosure rules come from `controls.ts`, appended after Pico so they
 * win its `[type=button]` ties.
 */

import { DISCLOSURE_CSS, ICON_BUTTON_CSS } from '../controls.js';

export const FM_STYLE_ID = 'mbr-fm-editor-styles';

const FORM_CSS = `
.mbr-fm { display: block; font-size: 0.9rem; padding: 0 0.75rem 0.5rem; }
.mbr-fm .fm-tabs { display: flex; gap: 0.25rem; margin: 0 0 0.35rem; }
.mbr-fm button.fm-tab {
  appearance: none; background: none; border: 0; box-shadow: none; margin: 0; width: auto;
  padding: 0.25rem 0.6rem; border-radius: 6px; font: inherit; font-size: 0.8rem; line-height: 1.3;
  color: var(--pico-muted-color, #6b7280); cursor: pointer;
}
.mbr-fm button.fm-tab[aria-selected="true"] {
  color: var(--pico-color, #1a1a1a); background: var(--mbr-control-press-bg, rgba(127,127,127,0.16)); font-weight: 600;
}
.mbr-fm button.fm-tab:focus-visible { outline: 2px solid var(--pico-primary, #0172ad); outline-offset: -2px; }
.mbr-fm .fm-rows { display: flex; flex-direction: column; }
.mbr-fm .fm-row {
  position: relative; display: grid; grid-template-columns: 20px minmax(5rem, 11rem) minmax(0, 1fr) auto;
  align-items: start; gap: 0 0.5rem; padding: 0.1rem 0;
}
.mbr-fm .fm-icon { display: flex; align-items: center; height: 2rem; color: var(--mbr-icon-color, var(--pico-muted-color, #6b7280)); }
.mbr-fm .fm-name {
  display: flex; align-items: center; min-height: 2rem; min-width: 0;
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  color: var(--pico-muted-color, #6b7280);
}
.mbr-fm .fm-name > span { overflow: hidden; text-overflow: ellipsis; }
.mbr-fm .fm-value { min-width: 0; display: flex; flex-direction: column; gap: 0.25rem; justify-content: center; min-height: 2rem; }
.mbr-fm .fm-actions { position: relative; display: flex; align-items: center; height: 2rem; }
.mbr-fm .fm-hint, .mbr-fm .fm-local-hint {
  grid-column: 3 / -1; display: flex; gap: 0.35rem; align-items: center;
  font-size: 0.78rem; color: var(--pico-muted-color, #6b7280); padding: 0 0.5rem 0.2rem;
}
.mbr-fm .fm-hint svg { color: var(--mbr-warning-color, #b7791f); flex: none; }
.mbr-fm .fm-children {
  grid-column: 2 / -1; margin: 0.1rem 0 0.25rem; padding-left: 0.6rem;
  border-left: 1px solid var(--pico-muted-border-color, #e0e0e0);
}
.mbr-fm input.fm-in, .mbr-fm textarea.fm-in, .mbr-fm select.fm-in {
  width: 100%; box-sizing: border-box; margin: 0; height: auto; min-height: 2rem;
  padding: 0.25rem 0.5rem; font: inherit; font-size: 0.9rem; line-height: 1.4;
  border: 1px solid transparent; border-radius: 6px; box-shadow: none;
  background-color: transparent; color: inherit;
}
.mbr-fm textarea.fm-in { resize: vertical; min-height: 3.5rem; }
.mbr-fm select.fm-in { padding-right: 1.75rem; background-position: right 0.4rem center; }
.mbr-fm .fm-in:hover { border-color: var(--pico-muted-border-color, #d0d0d0); }
.mbr-fm input.fm-in:focus, .mbr-fm textarea.fm-in:focus, .mbr-fm select.fm-in:focus {
  outline: none; border-color: var(--pico-primary, #0172ad);
  background-color: var(--pico-form-element-background-color, var(--pico-background-color, #fff));
  box-shadow: 0 0 0 1px var(--pico-primary, #0172ad);
}
.mbr-fm input.fm-in[readonly], .mbr-fm textarea.fm-in[readonly] { opacity: 0.8; }
.mbr-fm input[type="checkbox"].fm-check { margin: 0.45rem 0 0; }
.mbr-fm .fm-inline { display: flex; align-items: center; gap: 0.35rem; min-width: 0; }
.mbr-fm .fm-inline > .fm-in { flex: 1 1 auto; min-width: 0; }
.mbr-fm .fm-offset { flex: none; font-size: 0.78rem; color: var(--pico-muted-color, #6b7280); }
.mbr-fm .fm-partial { display: grid; grid-template-columns: 5.5rem minmax(6rem, 9rem) 5rem; gap: 0.35rem; }
.mbr-fm ol.fm-list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 0.1rem; }
.mbr-fm ol.fm-list > li { list-style: none; margin: 0; padding: 0; display: flex; align-items: center; gap: 0.25rem; }
.mbr-fm ol.fm-list > li > :first-child { flex: 1 1 auto; min-width: 0; }
.mbr-fm .fm-chips { display: flex; flex-wrap: wrap; align-items: center; gap: 0.25rem; }
.mbr-fm .fm-chip {
  display: inline-flex; align-items: center; gap: 0.1rem; max-width: 100%;
  padding: 0 0.1rem 0 0.55rem; border-radius: 999px; font-size: 0.82rem; line-height: 1.6;
  background: var(--mbr-control-hover-bg, rgba(127,127,127,0.14));
}
.mbr-fm .fm-chip > span { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.mbr-fm .fm-chips > mbr-fm-combobox { flex: 1 1 8rem; min-width: 8rem; }
.mbr-fm button.fm-link {
  appearance: none; background: none; border: 0; box-shadow: none; margin: 0; width: auto;
  display: inline-flex; align-items: center; gap: 0.35rem; align-self: flex-start;
  padding: 0.25rem 0.4rem; border-radius: 6px; min-height: 2rem;
  font: inherit; font-size: 0.85rem; color: var(--pico-primary, #0172ad); cursor: pointer;
}
.mbr-fm button.fm-link:hover { background: var(--mbr-control-hover-bg, rgba(127,127,127,0.12)); }
.mbr-fm button.fm-link:focus-visible { outline: 2px solid var(--pico-primary, #0172ad); outline-offset: -2px; }
.mbr-fm button.fm-link.fm-disclosure { color: var(--pico-muted-color, #6b7280); }
.mbr-fm .fm-preview {
  margin: 0; padding: 0.35rem 0.5rem; border-radius: 6px; max-height: 10rem; overflow: auto;
  font-family: var(--pico-font-family-monospace, monospace); font-size: 0.8rem; white-space: pre;
  background: var(--pico-code-background-color, #f6f8fa); color: inherit;
}
/* The YAML editors (CodeMirror, themed in yaml-source.ts); sizes live here for the media query. */
.mbr-fm .fm-cm { min-width: 0; }
.mbr-fm .fm-cm .cm-editor { font-size: 0.85rem; }
.mbr-fm .fm-yaml-text .cm-editor { min-height: 9rem; }
.mbr-fm .fm-yaml-text .cm-scroller { max-height: min(28rem, 60vh); overflow: auto; }
.mbr-fm .fm-yaml-slice .cm-editor { min-height: 3.5rem; }
.mbr-fm .fm-yaml-slice .cm-scroller { max-height: 16rem; overflow: auto; }
.mbr-fm .fm-yaml-slice[data-invalid="true"] .cm-editor { border-color: var(--pico-del-color, #b3261e); }
.mbr-fm .fm-yaml-actions { display: flex; gap: 0.25rem; align-items: center; }
.mbr-fm .fm-errors { margin: 0.25rem 0; padding: 0.4rem 0.6rem; border-radius: 6px; font-size: 0.82rem;
  background: color-mix(in srgb, var(--pico-del-color, #b3261e) 10%, transparent); }
.mbr-fm .fm-errors p { margin: 0; }
.mbr-fm .fm-errors ul { margin: 0.25rem 0 0; padding-left: 1.1rem; }
.mbr-fm .fm-status { font-size: 0.8rem; padding: 0.2rem 0; color: var(--pico-del-color, #b3261e); }
.mbr-fm .fm-status:empty { display: none; }
.mbr-fm .fm-note { font-size: 0.8rem; color: var(--pico-muted-color, #6b7280); padding: 0.2rem 0; }
.mbr-fm .fm-empty { font-size: 0.85rem; color: var(--pico-muted-color, #6b7280); padding: 0.35rem 0; }
.mbr-fm .fm-add { display: flex; align-items: center; gap: 0.35rem; padding-top: 0.2rem; }
.mbr-fm .fm-add > mbr-fm-combobox { flex: 0 1 18rem; }
.mbr-fm .fm-menu {
  position: absolute; top: calc(100% + 2px); right: 0; z-index: 20; min-width: 12rem; padding: 0.3rem;
  background: var(--pico-card-background-color, var(--pico-background-color, #fff));
  color: var(--pico-color, #1a1a1a);
  border: 1px solid var(--pico-muted-border-color, #e0e0e0); border-radius: 10px;
  box-shadow: 0 10px 32px rgba(0,0,0,0.22); display: flex; flex-direction: column;
}
.mbr-fm .fm-menu-label { font-size: 0.72rem; text-transform: uppercase; letter-spacing: 0.03em; padding: 0.35rem 0.6rem 0.1rem; color: var(--pico-muted-color, #6b7280); }
.mbr-fm .fm-menu hr { margin: 0.25rem 0; border: 0; border-top: 1px solid var(--pico-muted-border-color, #e0e0e0); }
.mbr-fm .fm-menu button[role="menuitem"] {
  appearance: none; background: none; border: 0; box-shadow: none; margin: 0; width: 100%;
  min-height: 2.25rem; padding: 0 0.6rem; border-radius: 6px; display: flex; align-items: center; gap: 0.5rem;
  font: inherit; font-size: 0.9rem; color: inherit; text-align: left; cursor: pointer;
}
.mbr-fm .fm-menu button[role="menuitem"]:hover, .mbr-fm .fm-menu button[role="menuitem"]:focus {
  outline: none; background: color-mix(in srgb, currentColor 10%, transparent);
}
.mbr-fm .fm-menu button[role="menuitem"].danger { color: var(--pico-del-color, #b3261e); }
.mbr-fm .fm-combo { position: relative; }
.mbr-fm .fm-combo-list {
  position: absolute; left: 0; right: 0; top: calc(100% + 2px); z-index: 30; max-height: 15rem; overflow: auto;
  margin: 0; padding: 0.25rem; list-style: none;
  background: var(--pico-card-background-color, var(--pico-background-color, #fff)); color: var(--pico-color, #1a1a1a);
  border: 1px solid var(--pico-muted-border-color, #e0e0e0); border-radius: 8px; box-shadow: 0 8px 24px rgba(0,0,0,0.18);
}
.mbr-fm .fm-combo-list[hidden] { display: none; }
.mbr-fm .fm-combo-list > li {
  list-style: none; margin: 0; display: flex; align-items: baseline; gap: 0.5rem;
  padding: 0.3rem 0.5rem; border-radius: 5px; cursor: pointer;
}
.mbr-fm .fm-combo-list > li.active, .mbr-fm .fm-combo-list > li:hover { background: var(--pico-primary-focus, rgba(99,102,241,0.15)); }
.mbr-fm .fm-combo-value { flex: none; max-width: 60%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.mbr-fm .fm-combo-detail { flex: 1; min-width: 0; font-size: 0.78rem; color: var(--pico-muted-color, #6b7280); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.mbr-fm .fm-summary { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 24rem; color: var(--pico-color, #1a1a1a); }
.mbr-fm button.fm-link.fm-me { flex: none; color: var(--pico-muted-color, #6b7280); }
.mbr-fm .fm-inline > mbr-fm-combobox { flex: 1 1 auto; min-width: 0; }
.mbr-fm ol.fm-edges, .mbr-fm ol.fm-labeled { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 0.35rem; }
.mbr-fm ol.fm-edges > li, .mbr-fm ol.fm-labeled > li { list-style: none; margin: 0; padding: 0; }
.mbr-fm .fm-edge { padding: 0.25rem 0.35rem; border-radius: 8px; border: 1px solid var(--pico-muted-border-color, #e0e0e0); }
.mbr-fm .fm-edge-main { display: grid; grid-template-columns: 5.25rem minmax(8rem, 1fr) minmax(5rem, 7rem) auto; gap: 0.25rem; align-items: center; }
.mbr-fm .fm-edge-main > .fm-edge-type { min-width: 0; }
.mbr-fm .fm-edge-main > .fm-edge-type input.fm-in { padding-left: 0.35rem; padding-right: 0.35rem; }
@media (min-width: 601px) {
  .mbr-fm .fm-edge-main.fm-edge-both {
    grid-template-columns: 5.25rem minmax(0, 1fr) auto minmax(0, 1fr) auto;
    grid-template-areas: "type label label label remove" "from-text from to-text to .";
  }
  .mbr-fm .fm-edge-main.fm-edge-both > .fm-edge-type { grid-area: type; }
  .mbr-fm .fm-edge-main.fm-edge-both > .fm-edge-label { grid-area: label; }
  .mbr-fm .fm-edge-main.fm-edge-both > .mbr-icon-btn { grid-area: remove; }
  .mbr-fm .fm-edge-main.fm-edge-both > .fm-edge-from-text { grid-area: from-text; padding-left: 0.7rem; }
  .mbr-fm .fm-edge-main.fm-edge-both > .fm-edge-from { grid-area: from; }
  .mbr-fm .fm-edge-main.fm-edge-both > .fm-edge-to-text { grid-area: to-text; padding: 0 0.15rem; }
  .mbr-fm .fm-edge-main.fm-edge-both > .fm-edge-to { grid-area: to; }
}
.mbr-fm .fm-edge-main > .fm-edge-dir-text { font-size: 0.8rem; color: var(--pico-muted-color, #6b7280); text-align: center; }
.mbr-fm .fm-edge-invert { display: inline-flex; align-items: center; gap: 0.35rem; margin: 0.15rem 0.4rem 0; font-size: 0.78rem; color: var(--pico-muted-color, #6b7280); cursor: pointer; }
.mbr-fm .fm-edge-invert > input[type="checkbox"] { margin: 0; flex: none; width: 0.9rem; height: 0.9rem; border-width: 1px; }
.mbr-fm .fm-edge-sentence { margin: 0.1rem 0.4rem 0; font-size: 0.75rem; font-style: italic; color: var(--pico-muted-color, #6b7280); }
.mbr-fm .fm-edge .fm-hint { padding-left: 0.4rem; }
.mbr-fm .fm-edge > .fm-children { margin-left: 0.4rem; }
.mbr-fm .fm-edge-add > mbr-fm-combobox { display: block; max-width: 12rem; }
.mbr-fm .fm-edge-add input.fm-in { font-size: 0.8rem; min-height: 1.75rem; }
.mbr-fm ol.fm-labeled > li:not(.fm-labeled-raw) { display: grid; grid-template-columns: minmax(5rem, 8rem) minmax(0, 1fr) auto; gap: 0.25rem; align-items: start; }
.mbr-fm .fm-labeled-value { min-width: 0; display: flex; flex-direction: column; }
.mbr-fm .fm-label input.fm-in { color: var(--pico-muted-color, #6b7280); }
.mbr-fm .fm-address { display: grid; grid-template-columns: 1fr 1fr 1fr; gap: 0.25rem; }
.mbr-fm .fm-address > .fm-address-street { grid-column: 1 / -1; min-height: 2.5rem; }
@media (max-width: 600px) {
  .mbr-fm .fm-row { grid-template-columns: 20px minmax(0, 1fr) auto; }
  .mbr-fm .fm-row > .fm-icon { grid-row: 1; grid-column: 1; height: 44px; }
  .mbr-fm .fm-row > .fm-name { grid-row: 1; grid-column: 2; min-height: 44px; }
  .mbr-fm .fm-row > .fm-actions { grid-row: 1; grid-column: 3; height: 44px; }
  .mbr-fm .fm-row > .fm-value { grid-column: 1 / -1; }
  .mbr-fm .fm-row > .fm-hint, .mbr-fm .fm-row > .fm-local-hint, .mbr-fm .fm-row > .fm-children { grid-column: 1 / -1; }
  .mbr-fm input.fm-in, .mbr-fm textarea.fm-in, .mbr-fm select.fm-in { font-size: 16px; min-height: 44px; }
  .mbr-fm .fm-cm .cm-editor { font-size: 16px; }
  .mbr-fm button.fm-link, .mbr-fm button.fm-tab { min-height: 44px; }
  .mbr-fm .fm-menu button[role="menuitem"] { min-height: 44px; }
  .mbr-fm .fm-combo-list > li { min-height: 44px; align-items: center; }
  .mbr-fm .fm-partial { grid-template-columns: 1fr 1.4fr 1fr; }
  .mbr-fm input[type="checkbox"].fm-check { margin: 0.75rem 0; }
  .mbr-fm .fm-edge-main { grid-template-columns: minmax(0, 1fr) auto; }
  .mbr-fm .fm-edge-main > .fm-edge-type input.fm-in { padding-left: 0.5rem; padding-right: 0.5rem; }
  .mbr-fm .fm-edge-main > .fm-wikilink, .mbr-fm .fm-edge-main > .fm-edge-label { grid-column: 1 / -1; }
  .mbr-fm .fm-edge-main > .mbr-icon-btn { grid-row: 1; grid-column: 2; }
  .mbr-fm .fm-edge-main.fm-edge-both { grid-template-columns: 2.5rem minmax(0, 1fr) auto; }
  .mbr-fm .fm-edge-main.fm-edge-both > .fm-edge-type { grid-row: 1; grid-column: 1 / 3; }
  .mbr-fm .fm-edge-main.fm-edge-both > .mbr-icon-btn { grid-column: 3; }
  .mbr-fm .fm-edge-main.fm-edge-both > .fm-edge-dir-text { grid-column: 1; text-align: left; padding-left: 0.5rem; }
  .mbr-fm .fm-edge-main.fm-edge-both > .fm-wikilink { grid-column: 2 / -1; }
  .mbr-fm .fm-edge-invert { min-height: 44px; }
  .mbr-fm .fm-edge-add > mbr-fm-combobox { max-width: none; }
  .mbr-fm .fm-edge-add input.fm-in { font-size: 16px; min-height: 44px; }
  .mbr-fm ol.fm-labeled > li:not(.fm-labeled-raw) { grid-template-columns: minmax(0, 1fr) auto; }
  .mbr-fm ol.fm-labeled > li > .fm-labeled-value { grid-column: 1 / -1; grid-row: 2; }
  .mbr-fm .fm-address { grid-template-columns: 1fr 1fr; }
}
`;

/** The whole stylesheet: the shared controls first, then the form's own rules. */
export function fmEditorCss(): string {
  return [ICON_BUTTON_CSS.cssText, DISCLOSURE_CSS.cssText, FORM_CSS].join('\n');
}

/** Adds the form's `<style>` to `doc`'s head once. */
export function ensureFmStyles(doc: Document = document): void {
  if (doc.getElementById(FM_STYLE_ID)) return;
  const style = doc.createElement('style');
  style.id = FM_STYLE_ID;
  style.textContent = fmEditorCss();
  doc.head.appendChild(style);
}
