import { describe, it, expect, afterEach } from 'vitest';
import { openPathPicker, fileFsPath, type SiteMarkdownFile } from './editor-path-picker.js';

/**
 * Tests for the editor's destination-path picker.
 *
 * Regression coverage for the `raw_path` bug: suggestions used to be
 * reconstructed from each file's directory-style `url_path`, which cannot tell
 * `docs/guide.md` from `docs/guide/index.md` and hard-coded the first markdown
 * extension. site.json ships the authoritative repo-relative `raw_path`
 * (verified against a real build: `docs/guide/index.md` -> `/docs/guide/`), so
 * the picker reads it and only falls back to the old derivation when a stale
 * cached site.json omits the field.
 *
 * The picker is plain DOM (no Lit), so these drive it end to end: open it with
 * a stub `fetchSiteFiles`, inspect/act on the modal, and assert on the resolved
 * `PathPickResult`.
 */

/** Lets the `fetchSiteFiles()` promise chain settle and the list re-render. */
const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

interface Harness {
  result: Promise<{ path: string; createDirs: boolean } | null>;
  input: HTMLInputElement;
  status: HTMLElement;
  cancelBtn: HTMLButtonElement;
  confirmBtn: HTMLButtonElement;
  /** `[badge, main, sub]` text of each visible suggestion row. */
  rows: () => Array<{ kind: string; label: string; sub: string }>;
  /** Clicks the first suggestion row whose sub-label matches `sub`. */
  pick: (sub: string) => void;
  /** Types a full path into the input (as the user would). */
  type: (value: string) => void;
  confirmBar: HTMLElement;
}

async function openPicker(
  files: SiteMarkdownFile[],
  opts: { mode?: 'new' | 'move'; currentFsPath?: string; markdownExtensions?: string[] } = {},
): Promise<Harness> {
  const result = openPathPicker({
    mode: opts.mode ?? 'new',
    currentFsPath: opts.currentFsPath ?? 'top.md',
    markdownExtensions: opts.markdownExtensions,
    fetchSiteFiles: () => Promise.resolve(files),
  });
  await flush();

  const backdrops = document.querySelectorAll('.mbr-picker-backdrop');
  const modal = backdrops[backdrops.length - 1] as HTMLElement;
  const input = modal.querySelector('input') as HTMLInputElement;
  const buttons = modal.querySelectorAll('.mbr-picker-footer button');

  const rows = () =>
    Array.from(modal.querySelectorAll('.mbr-picker-item')).map((row) => ({
      kind: row.querySelector('.mbr-picker-badge')?.textContent ?? '',
      label: row.querySelector('.mbr-picker-item-main')?.textContent ?? '',
      sub: row.querySelector('.mbr-picker-item-sub')?.textContent ?? '',
    }));

  return {
    result,
    input,
    status: modal.querySelector('.mbr-picker-status') as HTMLElement,
    cancelBtn: buttons[0] as HTMLButtonElement,
    confirmBtn: buttons[1] as HTMLButtonElement,
    confirmBar: modal.querySelector('.mbr-picker-confirm') as HTMLElement,
    rows,
    pick: (sub: string) => {
      const row = Array.from(modal.querySelectorAll('.mbr-picker-item')).find(
        (r) => r.querySelector('.mbr-picker-item-sub')?.textContent === sub,
      );
      if (!row) throw new Error(`no suggestion with sub-label "${sub}" in ${JSON.stringify(rows())}`);
      (row as HTMLElement).click();
    },
    type: (value: string) => {
      input.value = value;
      input.dispatchEvent(new Event('input'));
    },
  };
}

/** A repo with an index file, a `.markdown` file and a plain page. */
const FILES: SiteMarkdownFile[] = [
  { url_path: '/top/', raw_path: 'top.md' },
  { url_path: '/docs/', raw_path: 'docs/index.md' },
  { url_path: '/docs/guide/', raw_path: 'docs/guide/index.md', frontmatter: { title: 'Guide' } },
  { url_path: '/docs/notes/', raw_path: 'docs/notes.markdown' },
];

afterEach(() => {
  document.body.innerHTML = '';
});

describe('fileFsPath', () => {
  it('uses raw_path verbatim (index leaf and real extension preserved)', () => {
    expect(fileFsPath({ url_path: '/docs/guide/', raw_path: 'docs/guide/index.md' }, ['md'])).toBe(
      'docs/guide/index.md',
    );
    expect(fileFsPath({ url_path: '/docs/notes/', raw_path: 'docs/notes.markdown' }, ['md'])).toBe(
      'docs/notes.markdown',
    );
    expect(fileFsPath({ url_path: '/', raw_path: 'index.md' }, ['md'])).toBe('index.md');
  });

  it('tolerates a leading slash or padding on raw_path', () => {
    expect(fileFsPath({ url_path: '/docs/', raw_path: ' /docs/index.md ' }, ['md'])).toBe(
      'docs/index.md',
    );
  });

  it('falls back to the url-derived path when raw_path is missing', () => {
    expect(fileFsPath({ url_path: '/docs/guide/' }, ['md', 'markdown'])).toBe('docs/guide.md');
    expect(fileFsPath({ url_path: '/docs/guide/', raw_path: '' }, ['markdown'])).toBe(
      'docs/guide.markdown',
    );
    expect(fileFsPath({ url_path: '/' }, ['md'])).toBe('index.md');
  });
});

describe('openPathPicker suggestions', () => {
  it("suggests an index file's real raw_path, not the url-derived sibling", async () => {
    const h = await openPicker(FILES);
    const subs = h.rows()
      .filter((r) => r.kind === 'file')
      .map((r) => r.sub);

    expect(subs).toContain('docs/guide/index.md');
    expect(subs).toContain('docs/index.md');
    // The old derivation invented these paths; neither file exists.
    expect(subs).not.toContain('docs/guide.md');
    expect(subs).not.toContain('docs.md');

    h.cancelBtn.click();
    await expect(h.result).resolves.toBeNull();
  });

  it('keeps a .markdown extension instead of assuming the first configured one', async () => {
    const h = await openPicker(FILES);
    const subs = h.rows().map((r) => r.sub);

    expect(subs).toContain('docs/notes.markdown');
    expect(subs).not.toContain('docs/notes.md');

    h.cancelBtn.click();
    await h.result;
  });

  it("offers an index file's own folder for steering", async () => {
    const h = await openPicker(FILES);
    const folders = h.rows()
      .filter((r) => r.kind === 'folder')
      .map((r) => r.label);

    // `docs/guide` is only knowable from raw_path: `/docs/guide/` alone could
    // have been the file `docs/guide.md`.
    expect(folders).toEqual(['docs/', 'docs/guide/']);

    h.cancelBtn.click();
    await h.result;
  });

  it('falls back to the url derivation for a site.json without raw_path', async () => {
    const legacy: SiteMarkdownFile[] = [{ url_path: '/docs/guide/' }, { url_path: '/top/' }];
    const h = await openPicker(legacy);
    const subs = h.rows()
      .filter((r) => r.kind === 'file')
      .map((r) => r.sub);

    expect(subs).toEqual(['docs/guide.md', 'top.md']);

    h.cancelBtn.click();
    await h.result;
  });

  it('labels a file by its frontmatter title, falling back to the url stem', async () => {
    const h = await openPicker(FILES);
    const bySub = new Map(h.rows().map((r) => [r.sub, r.label]));

    expect(bySub.get('docs/guide/index.md')).toBe('Guide');
    expect(bySub.get('docs/notes.markdown')).toBe('notes');

    h.cancelBtn.click();
    await h.result;
  });
});

describe('openPathPicker validation', () => {
  it('steers into an index file\'s folder without a spurious "file exists" path', async () => {
    const h = await openPicker(FILES);
    h.pick('docs/guide/index.md');

    // The picked value names the file that actually exists, so the collision
    // warning is truthful rather than an artifact of a guessed path.
    expect(h.input.value).toBe('docs/guide/index.md');
    expect(h.status.textContent).toBe('A file already exists at that location.');
    expect(h.confirmBtn.disabled).toBe(true);

    // The realistic workflow: keep the steered folder, rename the leaf.
    h.type('docs/guide/new-note.md');
    expect(h.status.textContent).toBe('');
    expect(h.confirmBtn.disabled).toBe(false);

    h.confirmBtn.click();
    // No "create folder?" detour: docs/guide provably exists.
    expect(h.confirmBar.style.display).toBe('none');
    await expect(h.result).resolves.toEqual({ path: 'docs/guide/new-note.md', createDirs: false });
  });

  it('still prompts to create a folder that no file proves exists', async () => {
    const h = await openPicker(FILES);
    h.type('docs/new-section/page.md');

    expect(h.status.textContent).toBe('New folder “docs/new-section” will be created.');
    h.confirmBtn.click();
    expect(h.confirmBar.style.display).toBe('flex');

    const confirmButtons = Array.from(h.confirmBar.querySelectorAll('button'));
    const yesBtn = confirmButtons[confirmButtons.length - 1] as HTMLButtonElement;
    yesBtn.click();
    await expect(h.result).resolves.toEqual({
      path: 'docs/new-section/page.md',
      createDirs: true,
    });
  });

  it('flags a collision with an existing .markdown file', async () => {
    const h = await openPicker(FILES);
    h.type('docs/notes.markdown');

    expect(h.status.textContent).toBe('A file already exists at that location.');
    expect(h.confirmBtn.disabled).toBe(true);

    h.cancelBtn.click();
    await h.result;
  });

  it('lets an index file be renamed to a free sibling in move mode', async () => {
    const h = await openPicker(FILES, {
      mode: 'move',
      currentFsPath: 'docs/guide/index.md',
    });

    // Prefilled with the current path, which is rejected as a no-op move.
    expect(h.input.value).toBe('docs/guide/index.md');
    expect(h.status.textContent).toBe('Choose a different destination than the current file.');

    h.type('docs/guide/overview.md');
    expect(h.status.textContent).toBe('');
    h.confirmBtn.click();
    await expect(h.result).resolves.toEqual({ path: 'docs/guide/overview.md', createDirs: false });
  });
});
