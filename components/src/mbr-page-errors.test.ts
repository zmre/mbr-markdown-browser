/**
 * Tests for the page-problems summary sentence.
 *
 * `summarizePageErrors` is pure and exported precisely so the wording can be
 * asserted without mounting the element or stubbing `errors.json`.
 */
import { describe, expect, it, beforeEach, afterEach, vi } from 'vitest';
import './mbr-page-errors.js';
import { summarizePageErrors, type MbrPageErrorsElement } from './mbr-page-errors.js';
import {
  MEDIA_ERROR_EVENT,
  PAGE_ERRORS_LOADED_EVENT,
  clearPublishedPageErrors,
  type PageErrorsLoadedEventDetail,
  type UnplayableMediaError,
} from './media-errors.js';

/** Minimal well-formed entries, one per type used below. */
const frontmatterError = {
  type: 'frontmatter_parse_error' as const,
  message: 'String("to"): duplicated key in mapping',
};
const brokenLink = {
  type: 'broken_internal_link' as const,
  target: '/missing/',
  text: 'missing',
};
const cycle = {
  type: 'relationship_cycle' as const,
  members: ['/people/a/', '/people/b/'],
  rel_type: 'child',
};
const ambiguousWikilink = {
  type: 'ambiguous_wikilink' as const,
  raw: '[[John Doe]]',
  resolved_to: '/people/a-john/',
  candidates: ['/people/z-john/'],
};

describe('UNIT summarizePageErrors', () => {
  it('names only the problem types that are present', () => {
    const summary = summarizePageErrors([frontmatterError]);
    expect(summary).toBe('Detected 1 frontmatter parse error.');
  });

  it('never mentions a zero count', () => {
    const summary = summarizePageErrors([cycle]);
    // The regression this guards: every type enumerated, most of them "0 …".
    expect(summary).not.toMatch(/\b0\b/);
    expect(summary).toBe('Detected 1 relationship cycle.');
  });

  it('joins exactly two types with a bare "and"', () => {
    const summary = summarizePageErrors([brokenLink, cycle]);
    expect(summary).toBe('Detected 1 broken link and 1 relationship cycle.');
  });

  it('uses an Oxford comma from three types up', () => {
    const summary = summarizePageErrors([
      brokenLink,
      cycle,
      ambiguousWikilink,
    ]);
    expect(summary).toBe(
      'Detected 1 broken link, 1 relationship cycle, and 1 ambiguous wikilink.'
    );
  });

  it('pluralizes per type independently', () => {
    const summary = summarizePageErrors([brokenLink, brokenLink, cycle]);
    expect(summary).toBe('Detected 2 broken links and 1 relationship cycle.');
  });

  it('reports types in panel order, not arrival order', () => {
    const summary = summarizePageErrors([ambiguousWikilink, brokenLink]);
    expect(summary).toBe('Detected 1 broken link and 1 ambiguous wikilink.');
  });

  it('returns an empty string when there is nothing to report', () => {
    expect(summarizePageErrors([])).toBe('');
  });
});
const GOPRO: UnplayableMediaError = {
  type: 'unplayable_media',
  src: '../Foo%20Bar.mp4',
  kind: 'video',
  reason:
    "This file carries both a 'gpmd' timed-metadata track and a 'tx3g' subtitle track. " +
    'Safari/WebKit sometimes fails to decode that combination, so it is the most likely cause.',
  remedy: 'ffmpeg -i in.mp4 -map 0 -c copy -dn -movflags +faststart out.mp4',
  advisory: true,
}

// Complements the UNIT block above, which covers the link/wikilink/genealogy
// types; these cover the media types. The empty-list case lives up there.
describe('summarizePageErrors, media types', () => {
  it('uses the singular form for a single problem', () => {
    expect(summarizePageErrors([GOPRO])).toBe('Detected 1 unplayable media file.')
  })

  it('omits categories with no problems', () => {
    const summary = summarizePageErrors([
      { type: 'unresolved_wikilink', raw: '[[missing]]' },
      { type: 'unresolved_wikilink', raw: '[[gone]]' },
    ])
    expect(summary).toBe('Detected 2 unresolved wikilinks.')
  })

  it('joins multiple categories with an Oxford comma', () => {
    const summary = summarizePageErrors([
      { type: 'broken_internal_link', target: '/a/', text: 'a' },
      { type: 'unresolved_wikilink', raw: '[[missing]]' },
      GOPRO,
    ])
    expect(summary).toBe(
      'Detected 1 broken link, 1 unresolved wikilink, and 1 unplayable media file.'
    )
  })

  it('counts client-observed playback failures', () => {
    const summary = summarizePageErrors([
      {
        type: 'runtime_media_error',
        src: '/videos/a.mp4',
        kind: 'video',
        code: 3,
        message: '',
      },
    ])
    expect(summary).toMatch(/1 media file that failed to play here/)
  })
})

describe('<mbr-page-errors>', () => {
  let el: MbrPageErrorsElement
  const originalFetch = globalThis.fetch

  /** Mount the element with a canned errors.json payload (null = 404). */
  async function mount(errors: unknown[] | null): Promise<void> {
    globalThis.fetch = vi.fn().mockResolvedValue(
      errors === null
        ? { ok: false, status: 404 }
        : { ok: true, json: () => Promise.resolve({ page_url: '/x/', errors }) }
    ) as unknown as typeof fetch

    el = document.createElement('mbr-page-errors')
    document.body.appendChild(el)
    await new Promise<void>((resolve) => setTimeout(resolve, 0))
    await el.updateComplete
  }

  function fireMediaError(src: string, code = 3, message = 'Media failed to decode') {
    document.dispatchEvent(
      new CustomEvent(MEDIA_ERROR_EVENT, {
        detail: { src, kind: 'video', code, message },
      })
    )
  }

  async function openPanel(): Promise<string> {
    ;(el as unknown as { _isOpen: boolean })._isOpen = true
    await el.updateComplete
    return el.shadowRoot?.textContent?.replace(/\s+/g, ' ').trim() ?? ''
  }

  function count(): number | null {
    const text = el.shadowRoot?.querySelector('.errors-count')?.textContent
    return text == null ? null : Number(text)
  }

  beforeEach(() => {
    clearPublishedPageErrors()
    window.__MBR_CONFIG__ = { serverMode: true, guiMode: false }
  })

  afterEach(() => {
    document.body.innerHTML = ''
    window.__MBR_CONFIG__ = undefined
    globalThis.fetch = originalFetch
    clearPublishedPageErrors()
  })

  it('stays hidden when the page has no problems', async () => {
    await mount([])
    expect(el.shadowRoot?.querySelector('.errors-trigger')).toBeNull()
  })

  // The server predicate is a heuristic with a known false positive (a
  // synthetic file matching it plays fine in Safari), so an advisory hint on
  // its own must not raise the badge or appear in the drawer.
  it('withholds an advisory unplayable_media hint until playback actually fails', async () => {
    await mount([GOPRO])
    expect(el.shadowRoot?.querySelector('.errors-trigger')).toBeNull()
    expect(count()).toBeNull()
    expect(await openPanel()).not.toMatch(/gpmd/)
  })

  it('surfaces the advisory reason and remedy once playback fails for that src', async () => {
    await mount([GOPRO])
    // Same file, different spelling — must still match by canonical src.
    fireMediaError('/Foo Bar.mp4')
    await el.updateComplete

    // Counted once, not twice: the richer server entry replaces the runtime one.
    expect(count()).toBe(1)

    const panel = await openPanel()
    expect(panel).toContain('Unplayable media (1)')
    expect(panel).toMatch(/gpmd/)
    expect(panel).toContain(GOPRO.remedy!)
    expect(panel).toContain('Detected 1 unplayable media file.')
  })

  it('does not surface a hint when a different file fails', async () => {
    await mount([GOPRO])
    fireMediaError('/videos/unrelated.mp4')
    await el.updateComplete

    const panel = await openPanel()
    expect(panel).not.toMatch(/gpmd/)
    // The unrelated failure is still reported on its own terms.
    expect(count()).toBe(1)
    expect(panel).toMatch(/failed to play here/)
  })

  it('honours a non-advisory entry immediately', async () => {
    await mount([{ ...GOPRO, advisory: false }])
    expect(count()).toBe(1)
    expect(await openPanel()).toMatch(/gpmd/)
  })

  it('publishes the loaded payload for media elements', async () => {
    const seen: PageErrorsLoadedEventDetail[] = []
    const listener = (e: CustomEvent<PageErrorsLoadedEventDetail>) => seen.push(e.detail)
    document.addEventListener(PAGE_ERRORS_LOADED_EVENT, listener)
    await mount([GOPRO])
    document.removeEventListener(PAGE_ERRORS_LOADED_EVENT, listener)

    expect(seen).toEqual([{ errors: [GOPRO] }])
  })

  it('folds in a client-side failure the server knew nothing about', async () => {
    await mount([])
    expect(el.shadowRoot?.querySelector('.errors-trigger')).toBeNull()

    fireMediaError('/videos/Other.mp4')
    await el.updateComplete

    expect(count()).toBe(1)
    const panel = await openPanel()
    expect(panel).toContain('Failed to play in this browser (1)')
    expect(panel).toMatch(/could not decode/i)
  })

  it('surfaces client failures even when errors.json is missing', async () => {
    await mount(null)
    fireMediaError('/videos/Other.mp4')
    await el.updateComplete

    expect(count()).toBe(1)
  })

  it('prefers the server diagnosis when both describe the same file', async () => {
    await mount([GOPRO])
    // Same file, different encoding/relativity than the server entry.
    fireMediaError('/Foo Bar.mp4')
    await el.updateComplete

    expect(count()).toBe(1)
    const panel = await openPanel()
    expect(panel).toContain('Unplayable media (1)')
    expect(panel).not.toContain('Failed to play in this browser')
  })

  it('dedupes repeated reports for the same src', async () => {
    await mount([])
    fireMediaError('/videos/Foo Bar.mp4')
    fireMediaError('/videos/Foo%20Bar.mp4')
    await el.updateComplete

    expect(count()).toBe(1)
  })

  it('counts distinct failing files separately', async () => {
    await mount([])
    fireMediaError('/videos/a.mp4')
    fireMediaError('/videos/b.mp4')
    await el.updateComplete

    expect(count()).toBe(2)
  })

  it('ignores reports without a src', async () => {
    await mount([])
    document.dispatchEvent(
      new CustomEvent(MEDIA_ERROR_EVENT, {
        detail: { src: '', kind: 'video', code: 3, message: '' },
      })
    )
    await el.updateComplete

    expect(el.shadowRoot?.querySelector('.errors-trigger')).toBeNull()
  })

  it('stays inert outside server mode (static builds)', async () => {
    window.__MBR_CONFIG__ = undefined
    await mount([GOPRO])

    expect(globalThis.fetch).not.toHaveBeenCalled()

    fireMediaError('/videos/a.mp4')
    await el.updateComplete
    expect(el.shadowRoot?.querySelector('.errors-trigger')).toBeNull()
  })

  it('stops listening once removed', async () => {
    await mount([])
    el.remove()

    fireMediaError('/videos/a.mp4')
    await el.updateComplete
    expect(el.shadowRoot?.querySelector('.errors-trigger')).toBeNull()
  })
})
