/**
 * Unit tests for shared.ts utility functions (keyboard navigation helpers).
 */
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { isNewTabModifier, openInNewTab, getCanonicalPath, getGraphDepth } from './shared.ts';

describe('isNewTabModifier', () => {
  function makeKeyboardEvent(opts: Partial<KeyboardEventInit> = {}): KeyboardEvent {
    return new KeyboardEvent('keydown', { key: 'Enter', ...opts });
  }

  it('returns true when metaKey is pressed (macOS Cmd)', () => {
    expect(isNewTabModifier(makeKeyboardEvent({ metaKey: true }))).toBe(true);
  });

  it('returns true when ctrlKey is pressed', () => {
    expect(isNewTabModifier(makeKeyboardEvent({ ctrlKey: true }))).toBe(true);
  });

  it('returns true when both metaKey and ctrlKey are pressed', () => {
    expect(isNewTabModifier(makeKeyboardEvent({ metaKey: true, ctrlKey: true }))).toBe(true);
  });

  it('returns false when neither modifier is pressed', () => {
    expect(isNewTabModifier(makeKeyboardEvent())).toBe(false);
  });

  it('returns false when only shiftKey is pressed', () => {
    expect(isNewTabModifier(makeKeyboardEvent({ shiftKey: true }))).toBe(false);
  });

  it('returns false when only altKey is pressed', () => {
    expect(isNewTabModifier(makeKeyboardEvent({ altKey: true }))).toBe(false);
  });
});

describe('openInNewTab', () => {
  beforeEach(() => {
    vi.restoreAllMocks();
  });

  it('calls window.open with the URL and _blank target', () => {
    const openSpy = vi.spyOn(window, 'open').mockImplementation(() => null);
    openInNewTab('/docs/guide/');
    expect(openSpy).toHaveBeenCalledWith('/docs/guide/', '_blank');
  });

  it('passes through absolute URLs', () => {
    const openSpy = vi.spyOn(window, 'open').mockImplementation(() => null);
    openInNewTab('https://example.com/page');
    expect(openSpy).toHaveBeenCalledWith('https://example.com/page', '_blank');
  });
});

describe('getCanonicalPath', () => {
  const originalConfig = window.__MBR_CONFIG__;

  function setLocation(pathname: string): void {
    vi.stubGlobal('location', { pathname });
  }

  afterEach(() => {
    vi.unstubAllGlobals();
    window.__MBR_CONFIG__ = originalConfig;
  });

  it('decodes a percent-encoded pathname in server mode to match site.json keys', () => {
    // site.json stores url_path DECODED (literal spaces); the browser pathname
    // is percent-encoded. getCanonicalPath must decode so they match.
    window.__MBR_CONFIG__ = { serverMode: true, guiMode: false };
    setLocation('/Walsh/Patrick%20Joseph%20Walsh%20b.1977-10-01/');
    expect(getCanonicalPath()).toBe('/Walsh/Patrick Joseph Walsh b.1977-10-01/');
  });

  it('returns an already-decoded/plain path unchanged in server mode', () => {
    window.__MBR_CONFIG__ = { serverMode: true, guiMode: false };
    setLocation('/people/george/');
    expect(getCanonicalPath()).toBe('/people/george/');
  });

  it('falls back to the raw string on a malformed escape without throwing', () => {
    window.__MBR_CONFIG__ = { serverMode: true, guiMode: false };
    setLocation('/a%b/');
    expect(getCanonicalPath()).toBe('/a%b/');
  });

  it('decodes %20 segments in static mode too', () => {
    // Deployed under a prefix; depth 2 keeps the last two DECODED segments.
    window.__MBR_CONFIG__ = { serverMode: false, guiMode: false, basePath: '../../' };
    setLocation('/prefix/Walsh/Patrick%20Joseph%20Walsh%20b.1977-10-01/');
    expect(getCanonicalPath()).toBe('/Walsh/Patrick Joseph Walsh b.1977-10-01/');
  });
});

/**
 * Helpers for the module-scope tests below.
 *
 * `shared.ts` runs fetches at import time, so these tests reset the module
 * registry and install a recording fetch before re-importing it.
 */
type SharedModule = typeof import('./shared.ts');

function stubRecordingFetch(site: unknown, urls: string[]): void {
  vi.stubGlobal(
    'fetch',
    vi.fn((url: string) => {
      urls.push(url);
      const body = url.includes('media.json') ? { other_files: [{ url_path: '/img/a.png' }] } : site;
      return Promise.resolve({ ok: true, json: () => Promise.resolve(body) });
    }),
  );
}

function countUrls(urls: string[], needle: string): number {
  return urls.filter(u => u.includes(needle)).length;
}

async function importFreshShared(site: unknown, urls: string[]): Promise<SharedModule> {
  vi.resetModules();
  stubRecordingFetch(site, urls);
  return await import('./shared.ts');
}

describe('getSiteFolderTree', () => {
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.resetModules();
  });

  it('builds the folder tree once and shares it across consumers', async () => {
    const urls: string[] = [];
    const mod = await importFreshShared(
      {
        index_file: 'index.md',
        markdown_files: [
          { url_path: '/docs/a/', raw_path: 'docs/a.md', created: 1, modified: 1, frontmatter: null },
          { url_path: '/docs/b/', raw_path: 'docs/b.md', created: 1, modified: 1, frontmatter: null },
        ],
      },
      urls,
    );

    const [first, second, third] = await Promise.all([
      mod.getSiteFolderTree(),
      mod.getSiteFolderTree(),
      mod.getSiteFolderTree(),
    ]);

    // Same object for every consumer => buildFolderTree ran exactly once.
    expect(first).toBe(second);
    expect(second).toBe(third);
    expect(first.children.get('docs')?.files).toHaveLength(2);
    expect(countUrls(urls, 'site.json')).toBe(1);
  });

  it('honors the configured index_file so folder landing pages attach to the folder', async () => {
    const urls: string[] = [];
    const mod = await importFreshShared(
      {
        index_file: '_index.md',
        markdown_files: [
          {
            url_path: '/docs/',
            raw_path: 'docs/_index.md',
            created: 1,
            modified: 1,
            frontmatter: { title: 'Docs' },
          },
          { url_path: '/docs/guide/', raw_path: 'docs/guide.md', created: 1, modified: 1, frontmatter: null },
        ],
      },
      urls,
    );

    const tree = await mod.getSiteFolderTree();

    // With the wrong index file, '/docs/' would land in the ROOT file list.
    expect(tree.files).toHaveLength(0);
    const docs = tree.children.get('docs');
    expect(docs?.title).toBe('Docs');
    expect(docs?.files.map(f => f.raw_path).sort()).toEqual(['docs/_index.md', 'docs/guide.md']);
  });
});

describe('media navigation loading', () => {
  const emptySite = { index_file: 'index.md', markdown_files: [] };

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.resetModules();
  });

  it('does not fetch media.json at import time', async () => {
    const urls: string[] = [];
    const mod = await importFreshShared(emptySite, urls);
    await mod.siteNav;
    await Promise.resolve();

    expect(countUrls(urls, 'media.json')).toBe(0);
    expect(countUrls(urls, 'site.json')).toBe(1);
  });

  it('fetches media.json exactly once, on the first subscription', async () => {
    const urls: string[] = [];
    const mod = await importFreshShared(emptySite, urls);

    const unsubA = mod.subscribeMediaNav(() => {});
    expect(countUrls(urls, 'media.json')).toBe(1);

    const unsubB = mod.subscribeMediaNav(() => {});
    expect(countUrls(urls, 'media.json')).toBe(1);

    // A direct load() call reuses the same in-flight promise too.
    expect(mod.loadMediaNav()).toBe(mod.loadMediaNav());
    expect(countUrls(urls, 'media.json')).toBe(1);

    unsubA();
    unsubB();
  });

  it('notifies subscribers once media.json resolves', async () => {
    const urls: string[] = [];
    const mod = await importFreshShared(emptySite, urls);

    const states: Array<{ isLoading: boolean; data: any | null }> = [];
    const unsub = mod.subscribeMediaNav(state => {
      states.push({ isLoading: state.isLoading, data: state.data });
    });
    await mod.loadMediaNav();

    expect(states[0].isLoading).toBe(true);
    expect(states[states.length - 1].isLoading).toBe(false);
    expect(states[states.length - 1].data?.other_files).toHaveLength(1);
    expect(mod.getMediaNavState().data?.other_files).toHaveLength(1);
    unsub();
  });
});

describe('getGraphDepth', () => {
  const originalConfig = window.__MBR_CONFIG__;

  afterEach(() => {
    window.__MBR_CONFIG__ = originalConfig;
  });

  function setDepth(graphDepth: unknown): void {
    window.__MBR_CONFIG__ = { serverMode: true, guiMode: false, graphDepth: graphDepth as number };
  }

  it('defaults to 2 when the config is absent', () => {
    window.__MBR_CONFIG__ = undefined;
    expect(getGraphDepth()).toBe(2);
  });

  it('defaults to 2 when graphDepth is missing', () => {
    window.__MBR_CONFIG__ = { serverMode: true, guiMode: false };
    expect(getGraphDepth()).toBe(2);
  });

  it('defaults to 2 for non-numeric values', () => {
    setDepth('3');
    expect(getGraphDepth()).toBe(2);
    setDepth(NaN);
    expect(getGraphDepth()).toBe(2);
  });

  it('passes through in-range values', () => {
    setDepth(1);
    expect(getGraphDepth()).toBe(1);
    setDepth(4);
    expect(getGraphDepth()).toBe(4);
  });

  it('clamps out-of-range values to 1–5', () => {
    setDepth(0);
    expect(getGraphDepth()).toBe(1);
    setDepth(-3);
    expect(getGraphDepth()).toBe(1);
    setDepth(6);
    expect(getGraphDepth()).toBe(5);
    setDepth(99);
    expect(getGraphDepth()).toBe(5);
  });

  it('floors fractional values', () => {
    setDepth(3.9);
    expect(getGraphDepth()).toBe(3);
  });
});
