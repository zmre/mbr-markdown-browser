/**
 * The editor's source mode (`editor-source.ts`): the whole file in CodeMirror,
 * saved as typed. Driven through the real `openEditor` with `fetch` stubbed,
 * as `editor-roundtrip.test.ts` drives it, so the toggle, the dirty check,
 * Save, Cmd/Ctrl+S and Cancel are the ones a reader uses.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { EditorView } from '@codemirror/view';
import { openEditor } from './editor-crepe.js';
import { EDITOR_MODE_KEY } from './editor-mode-pref.js';
import { sourceEditorExtensions } from './editor-source.js';
import { EditorState } from '@codemirror/state';

const tick = () => new Promise((r) => setTimeout(r, 0));

interface Opened {
  posts: { content: string; base_hash: string }[];
  onSaved: ReturnType<typeof vi.fn>;
  modal: Element;
}

/** Opens the real editor on `raw` with `fetch` stubbed; each POST's body is recorded. */
async function openOn(raw: string): Promise<Opened> {
  const posts: Opened['posts'] = [];
  let hash = 1;
  vi.stubGlobal(
    'fetch',
    vi.fn(async (_url: string, init?: RequestInit) => {
      if (init?.method === 'POST') {
        posts.push(JSON.parse(String(init.body)));
        hash += 1;
        return new Response('', { status: 200, headers: { 'X-MBR-Content-Hash': `h${hash}` } });
      }
      return new Response(raw, { status: 200, headers: { 'X-MBR-Content-Hash': 'h1' } });
    }),
  );
  const onSaved = vi.fn();
  await openEditor({ rawUrl: '/.mbr/raw/n.md', saveUrl: '/.mbr/edit/n.md', filePath: 'n.md', onClose: () => {}, onSaved });
  const modal = document.querySelector('.mbr-editor-modal');
  if (!modal) throw new Error('editor modal not built');
  return { posts, onSaved, modal };
}

const modeButton = (modal: Element) => modal.querySelector<HTMLButtonElement>('.mbr-editor-mode')!;

/** Clicks the mode toggle and waits for the other editor to be up. */
async function toggleMode(modal: Element): Promise<void> {
  modeButton(modal).click();
  await tick();
  await tick();
}

const sourceView = (modal: Element): EditorView | null => {
  const dom = modal.querySelector<HTMLElement>('.mbr-editor-source .cm-editor');
  return dom ? EditorView.findFromDOM(dom) : null;
};

/** Replaces the whole source text, as typing would. */
function typeSource(modal: Element, text: string): void {
  const view = sourceView(modal)!;
  view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: text }, userEvent: 'input.type' });
}

const clickSave = async (modal: Element) => {
  modal.querySelector<HTMLButtonElement>('.mbr-editor-save')!.click();
  await tick();
};

const RAW = '---\ntitle: Note\n---\n\n# Note\n\nSome *text*.\n';

beforeEach(() => localStorage.clear());
afterEach(() => {
  vi.unstubAllGlobals();
  document.querySelectorAll('.mbr-editor-backdrop, .mbr-picker-backdrop').forEach((el) => el.remove());
  localStorage.clear();
});

describe('source mode', () => {
  it('opens rich by default, and the toggle shows the whole file, frontmatter included', async () => {
    const { modal } = await openOn(RAW);
    expect(modeButton(modal).getAttribute('aria-pressed')).toBe('false');
    expect(sourceView(modal)).toBeNull();
    await toggleMode(modal);
    expect(modeButton(modal).getAttribute('aria-pressed')).toBe('true');
    expect(sourceView(modal)!.state.doc.toString()).toBe(RAW);
    expect(modal.querySelector<HTMLElement>('.mbr-editor-crepe')!.hidden).toBe(true);
    expect(modal.querySelector<HTMLElement>('.mbr-editor-fm')!.hidden).toBe(true);
    // The rich editor's tools act on its document; hidden with it.
    expect(modal.querySelector<HTMLElement>('.mbr-editor-footnote')!.hidden).toBe(true);
    expect(modal.querySelector<HTMLElement>('.mbr-editor-media')!.hidden).toBe(true);
  });

  it('a switch alone changes nothing: Save writes nothing', async () => {
    const { modal, posts } = await openOn(RAW);
    await toggleMode(modal);
    await clickSave(modal);
    expect(posts).toHaveLength(0);
    expect(modal.querySelector('.status')?.textContent).toBe('No changes to save.');
    await toggleMode(modal);
    await clickSave(modal);
    expect(posts).toHaveLength(0);
  });

  it('saves exactly what was typed, markdown the rich editor would respell and HTML included', async () => {
    const { modal, posts, onSaved } = await openOn(RAW);
    await toggleMode(modal);
    const typed = '---\ntitle:   "Spaced"   # comment\n---\n+ plus bullet\n+ two\n\nline<br />break  \n&#x20; indented\n\n\n\nend';
    typeSource(modal, typed);
    await clickSave(modal);
    expect(posts).toHaveLength(1);
    expect(posts[0].content).toBe(typed);
    expect(posts[0].base_hash).toBe('h1');
    expect(onSaved).toHaveBeenCalledOnce();
  });

  it('writes a CRLF file back as CRLF', async () => {
    const { modal, posts } = await openOn(RAW.replace(/\n/g, '\r\n'));
    await toggleMode(modal);
    expect(sourceView(modal)!.state.doc.toString()).toBe(RAW);
    typeSource(modal, RAW.replace('Some', 'More'));
    await clickSave(modal);
    expect(posts[0].content).toBe(RAW.replace('Some', 'More').replace(/\n/g, '\r\n'));
  });

  it('Cmd/Ctrl+S saves from the source, and the next save sends the new hash', async () => {
    const { modal, posts } = await openOn(RAW);
    await toggleMode(modal);
    typeSource(modal, RAW + '\nmore\n');
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 's', metaKey: true, bubbles: true, cancelable: true }));
    await tick();
    typeSource(modal, RAW + '\nmore again\n');
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 's', ctrlKey: true, bubbles: true, cancelable: true }));
    await tick();
    expect(posts.map((p) => p.base_hash)).toEqual(['h1', 'h2']);
    expect(posts[1].content).toBe(RAW + '\nmore again\n');
  });

  it('Cancel asks first when the source has unsaved edits', async () => {
    const { modal } = await openOn(RAW);
    await toggleMode(modal);
    typeSource(modal, RAW + 'x');
    modal.querySelector<HTMLButtonElement>('.mbr-editor-cancel')!.click();
    await tick();
    expect(document.querySelector('.mbr-picker-backdrop[aria-label="Unsaved changes"]')).not.toBeNull();
  });

  it('carries unsaved rich edits into the source: a property and a body edit', async () => {
    const { modal } = await openOn(RAW);
    const form = modal.querySelector('mbr-fm-editor') as HTMLElement & { updateComplete: Promise<boolean> };
    await form.updateComplete;
    const row = [...form.querySelectorAll<HTMLElement>('.fm-row')].find((r) => r.dataset.path === JSON.stringify(['title']))!;
    const input = row.querySelector<HTMLInputElement>('.fm-value input')!;
    input.value = 'Retitled';
    input.dispatchEvent(new Event('input', { bubbles: true }));
    await form.updateComplete;
    // A body edit through the footer's own tool: a footnote at the cursor.
    modal.querySelector<HTMLButtonElement>('.mbr-editor-footnote')!.click();
    await toggleMode(modal);
    const text = sourceView(modal)!.state.doc.toString();
    expect(text).toContain('title: Retitled');
    expect(text).toContain('[^1]');
  });

  it('carries unsaved source edits into the rich editor and its properties', async () => {
    const { modal, posts } = await openOn(RAW);
    await toggleMode(modal);
    const edited = RAW.replace('title: Note', 'title: Renamed').replace('Some', 'Changed');
    typeSource(modal, edited);
    await toggleMode(modal);
    expect(sourceView(modal)).toBeNull();
    const form = modal.querySelector('mbr-fm-editor') as HTMLElement & { text: string };
    expect(form.text).toContain('title: Renamed');
    expect(modal.querySelector('.mbr-editor-crepe .ProseMirror')?.textContent).toContain('Changed');
    // Still unsaved, and Save writes it.
    await clickSave(modal);
    expect(posts).toHaveLength(1);
    expect(posts[0].content).toBe(edited);
  });

  it('remembers the mode: the next editor opens in source mode, without Crepe', async () => {
    const { modal } = await openOn(RAW);
    await toggleMode(modal);
    expect(localStorage.getItem(EDITOR_MODE_KEY)).toBe('source');
    document.querySelectorAll('.mbr-editor-backdrop').forEach((el) => el.remove());
    const second = await openOn(RAW);
    expect(sourceView(second.modal)!.state.doc.toString()).toBe(RAW);
    expect(second.modal.querySelector('.mbr-editor-crepe .milkdown')).toBeNull();
    await toggleMode(second.modal);
    expect(localStorage.getItem(EDITOR_MODE_KEY)).toBeNull();
    expect(second.modal.querySelector('.mbr-editor-crepe .ProseMirror')).not.toBeNull();
  });

  it('is in the ⋯ menu too, labelled for the mode it switches to', async () => {
    const { modal } = await openOn(RAW);
    const item = () => modal.querySelector<HTMLButtonElement>('.mbr-editor-menu [role="menuitem"]')!;
    expect(item().textContent).toBe('Markdown source');
    item().click();
    await tick();
    await tick();
    expect(sourceView(modal)).not.toBeNull();
    expect(item().textContent).toBe('Rich text');
  });
});

describe('source editor extensions', () => {
  it('highlight frontmatter and markdown, and keep the text as given', () => {
    const state = EditorState.create({ doc: RAW, extensions: sourceEditorExtensions(() => {}) });
    expect(state.doc.toString()).toBe(RAW);
  });
});
