/**
 * Shared happy-dom helpers for the properties form's tests: mounting
 * `<mbr-fm-editor>`, finding a row's own value cell, typing, committing a
 * combobox, opening a row's menu.
 */

import { FM_CHANGE_EVENT, type FmChangeDetail, type MbrFmEditor } from './properties-form.js';
import './properties-form.js';
import { openFrontmatter, type FmPath } from './model.js';
import { pathKey } from './form-logic.js';
import type { MbrFmCombobox } from './fm-combobox.js';
import { EditorView } from '@codemirror/view';

export async function mount(text: string, setup: (el: MbrFmEditor) => void = () => {}): Promise<{ el: MbrFmEditor; changes: string[] }> {
  const el = document.createElement('mbr-fm-editor');
  el.model = openFrontmatter(text);
  setup(el);
  const changes: string[] = [];
  el.addEventListener(FM_CHANGE_EVENT, (e) => changes.push((e as CustomEvent<FmChangeDetail>).detail.text));
  document.body.appendChild(el);
  await el.updateComplete;
  return { el, changes };
}

/** Lets the form and any nested comboboxes finish rendering. */
export async function settle(el: MbrFmEditor): Promise<void> {
  for (let i = 0; i < 3; i++) {
    await el.updateComplete;
    await Promise.all([...el.querySelectorAll<MbrFmCombobox>('mbr-fm-combobox')].map((c) => c.updateComplete));
  }
}

export function row(el: MbrFmEditor, path: FmPath): HTMLElement {
  const found = [...el.querySelectorAll<HTMLElement>('.fm-row')].find((r) => r.dataset.path === pathKey(path));
  if (!found) throw new Error(`no row for ${pathKey(path)}`);
  return found;
}

/** The row's own value cell (not a nested row's). */
export function value(el: MbrFmEditor, path: FmPath): HTMLElement {
  const r = row(el, path);
  return [...r.children].find((c) => c.classList.contains('fm-value')) as HTMLElement;
}

export function type(input: HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement, text: string, event = 'input'): void {
  input.value = text;
  input.dispatchEvent(new Event(event, { bubbles: true }));
}

export function commit(combo: MbrFmCombobox, text: string, via = 'enter'): void {
  combo.dispatchEvent(new CustomEvent('fm-combo-commit', { detail: { value: text, via }, bubbles: true }));
}

export async function openMenu(el: MbrFmEditor, path: FmPath): Promise<HTMLElement> {
  const r = row(el, path);
  const btn = [...r.querySelectorAll<HTMLButtonElement>('.fm-menu-btn')].find((b) => b.closest('.fm-row') === r)!;
  btn.click();
  await settle(el);
  return [...r.querySelectorAll<HTMLElement>('.fm-menu')].find((m) => m.closest('.fm-row') === r)!;
}

export function menuItem(menu: HTMLElement, label: string): HTMLButtonElement {
  const item = [...menu.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')].find((b) => b.textContent?.trim() === label);
  if (!item) throw new Error(`no menu item ${label}`);
  return item;
}


/** The CodeMirror view mounted in `container` (`.fm-yaml-text`, `.fm-yaml-slice`). */
export function cmView(container: Element | null): EditorView {
  const dom = container?.querySelector<HTMLElement>('.cm-editor');
  const view = dom ? EditorView.findFromDOM(dom) : null;
  if (!view) throw new Error('no CodeMirror editor mounted');
  return view;
}

/** Replaces a CodeMirror view's text the way typing does (a user input transaction). */
export function cmType(view: EditorView, text: string): void {
  view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: text }, userEvent: 'input.type' });
}
