/**
 * Unit tests for shared.ts utility functions (keyboard navigation helpers).
 */
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { isNewTabModifier, openInNewTab } from './shared.ts';

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
