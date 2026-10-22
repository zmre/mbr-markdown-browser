/**
 * Test support for the editor round-trip and parity tests: the fixture corpus,
 * and Crepe opened, parsed and serialized exactly as `openEditor` does it.
 *
 * Not a test file itself (no `.test.ts`), and nothing in a bundle imports it.
 */
import { Crepe } from '@milkdown/crepe';
import { editorViewCtx, remarkCtx } from '@milkdown/kit/core';
import { TextSelection } from '@milkdown/kit/prose/state';
import type { EditorView } from '@milkdown/kit/prose/view';
import { CREPE_FEATURE_CONFIGS, CREPE_FEATURES, serializeBody, useMarkdownExtensions } from './editor-crepe.js';
import { documentToSave, openDocument } from './editor-frontmatter.js';
import { openFrontmatter } from './frontmatter/model.js';
import { mdastOutline, type MdastNode, type OutlineNode } from './editor-outline.js';

/** Set by `vitest.config.ts` from `tests/fixtures/editor-roundtrip/`. */
declare const __MBR_EDITOR_FIXTURES__: Record<string, { source: string; outline: string | null }>;

/** Every fixture, by name (file name without `.md`), with its renderer outline golden. */
export const EDITOR_FIXTURES: Readonly<Record<string, { source: string; outline: string | null }>> =
  __MBR_EDITOR_FIXTURES__;

/** A fixture's source; throws for an unknown name so a rename cannot pass vacuously. */
export function fixture(name: string): string {
  const entry = EDITOR_FIXTURES[name];
  if (!entry) throw new Error(`no editor fixture named ${name}`);
  return entry.source;
}

const open: Crepe[] = [];

/** Crepe on `body`, configured as `openEditor` configures it. */
export async function openCrepe(body: string): Promise<Crepe> {
  const root = document.createElement('div');
  document.body.appendChild(root);
  const crepe = new Crepe({ root, defaultValue: body, features: CREPE_FEATURES, featureConfigs: CREPE_FEATURE_CONFIGS });
  useMarkdownExtensions(crepe);
  await crepe.create();
  open.push(crepe);
  return crepe;
}

/** Destroys every editor `openCrepe` made; call from `afterEach`. */
export function destroyEditors(): void {
  for (const crepe of open.splice(0)) crepe.destroy();
  document.body.innerHTML = '';
}

/**
 * What the editor writes when a whole file is opened and saved, with an
 * optional edit in between: the frontmatter goes through the properties
 * editor's model (whose text, untouched, is the YAML as opened), the body
 * through Crepe, and `documentToSave` joins them — `openEditor`'s
 * `currentContent()`.
 */
export async function savedFile(source: string, edit?: (crepe: Crepe) => void): Promise<string> {
  // `openEditor` opens the file as LF and saves it with its own line ending.
  const doc = openDocument(source);
  const crepe = await openCrepe(doc.body);
  edit?.(crepe);
  return documentToSave(doc, openFrontmatter(doc.frontmatter ?? '').text, serializeBody(crepe));
}

/**
 * The frontmatter as a textarea hands it back (line breaks normalized to LF,
 * HTML's "API value"), for callers that edit it as text.
 */
export { textareaValue } from './editor-frontmatter.js';

/**
 * The editor's outline of a whole file: the body parsed exactly as Milkdown's
 * parser does (`remark.runSync(remark.parse(body), body)` on the editor's own
 * processor, so every registered remark plugin applies), with line numbers
 * shifted past the frontmatter.
 */
export async function editorOutline(source: string): Promise<OutlineNode[]> {
  // Split as the editor opens the file, so the outline and a save agree on where the body starts.
  const { body, bodyLineOffset } = openDocument(source);
  const crepe = await openCrepe(body);
  const tree = crepe.editor.action((ctx) => {
    const remark = ctx.get(remarkCtx);
    return remark.runSync(remark.parse(body), body);
  }) as unknown as MdastNode;
  return mdastOutline(tree, body, bodyLineOffset);
}

// Driving the editor the way a user does: cursor, keys, typing.

/** The editor view of `crepe`. */
export function viewOf(crepe: Crepe): EditorView {
  return crepe.editor.action((ctx) => ctx.get(editorViewCtx));
}

/** Puts the cursor `offset` characters into the first occurrence of `text`. */
export function placeCursor(view: EditorView, text: string, offset = 0): void {
  let found = -1;
  view.state.doc.descendants((node, pos) => {
    if (found >= 0) return false;
    if (node.isText && node.text?.includes(text)) found = pos + (node.text.indexOf(text) ?? 0) + offset;
    return true;
  });
  if (found < 0) throw new Error(`no text ${text}`);
  view.dispatch(view.state.tr.setSelection(TextSelection.create(view.state.doc, found)));
}

/** Places the cursor in the first empty textblock of type `name`. */
export function placeInEmpty(view: EditorView, name: string): void {
  let found = -1;
  view.state.doc.descendants((node, pos) => {
    if (found >= 0) return false;
    if (node.type.name === name && node.content.size === 0) found = pos + 1;
    return true;
  });
  if (found < 0) throw new Error(`no empty ${name}`);
  view.dispatch(view.state.tr.setSelection(TextSelection.create(view.state.doc, found)));
}

/** Presses `key` through the editor's keymaps; returns whether anything handled it. */
export function press(view: EditorView, key: string): boolean {
  const event = new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true });
  return Boolean(view.someProp('handleKeyDown', (handle) => handle(view, event)));
}

/** Types `text` one character at a time through the input rules, as a user would. */
export function typeText(view: EditorView, text: string): void {
  for (const char of text) {
    const { from, to } = view.state.selection;
    const handled = view.someProp('handleTextInput', (handle) => handle(view, from, to, char, () => view.state.tr));
    if (!handled) view.dispatch(view.state.tr.insertText(char, from, to));
  }
}
