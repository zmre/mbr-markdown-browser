import { describe, expect, it } from 'vitest';
import { noteDir } from './editor-upload.js';

describe('noteDir', () => {
  it('returns the parent folder of a file in a subfolder', () => {
    expect(noteDir('notes/foo.md')).toBe('notes');
  });

  it('returns an empty string for a root-level file', () => {
    expect(noteDir('foo.md')).toBe('');
  });

  it('returns the full nested folder path', () => {
    expect(noteDir('a/b/c/foo.md')).toBe('a/b/c');
  });

  it('strips a leading slash so the result stays repo-relative', () => {
    expect(noteDir('/notes/foo.md')).toBe('notes');
    expect(noteDir('/foo.md')).toBe('');
  });

  it('handles an empty path', () => {
    expect(noteDir('')).toBe('');
  });
});
