/**
 * Test support for the editor round-trip and parity tests: the fixture corpus,
 * and Crepe opened, parsed and serialized exactly as `openEditor` does it.
 *
 * Not a test file itself (no `.test.ts`), and nothing in a bundle imports it.
 */
import { Crepe } from '@milkdown/crepe';
import { remarkCtx } from '@milkdown/kit/core';
import { CREPE_FEATURES, serializeBody, useMarkdownExtensions } from './editor-crepe.js';
import { recombine, splitFrontmatter } from './editor-frontmatter.js';
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
  const crepe = new Crepe({ root, defaultValue: body, features: CREPE_FEATURES });
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
 * optional edit in between: the frontmatter goes through the YAML textarea,
 * the body through Crepe, and `recombine` joins them — `openEditor`'s
 * `currentContent()`.
 */
export async function savedFile(source: string, edit?: (crepe: Crepe) => void): Promise<string> {
  const { frontmatter, body } = splitFrontmatter(source);
  const crepe = await openCrepe(body);
  edit?.(crepe);
  // A textarea's value normalizes line breaks to LF (HTML's "API value"), and
  // the frontmatter is read back from `fmTextarea.value`.
  const yaml = (frontmatter ?? '').replace(/\r\n?/g, '\n');
  return recombine(yaml, serializeBody(crepe));
}

/**
 * The editor's outline of a whole file: the body parsed exactly as Milkdown's
 * parser does (`remark.runSync(remark.parse(body), body)` on the editor's own
 * processor, so every registered remark plugin applies), with line numbers
 * shifted past the frontmatter.
 */
export async function editorOutline(source: string): Promise<OutlineNode[]> {
  const { body } = splitFrontmatter(source);
  const crepe = await openCrepe(body);
  const tree = crepe.editor.action((ctx) => {
    const remark = ctx.get(remarkCtx);
    return remark.runSync(remark.parse(body), body);
  }) as unknown as MdastNode;
  const prefix = source.slice(0, source.length - body.length);
  const lineOffset = prefix.split('\n').length - 1;
  return mdastOutline(tree, body, lineOffset);
}
