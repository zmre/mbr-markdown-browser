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
