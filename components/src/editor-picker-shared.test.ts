import { describe, expect, it } from 'vitest';
import {
  normalizeUrl,
  parentFolder,
  hasMarkdownExtension,
  deriveExistingFolders,
  fsPathToApproxUrl,
  relativeUrlPath,
  encodeLinkDestination,
} from './editor-picker-shared.js';

describe('normalizeUrl', () => {
  it('adds leading and trailing slashes', () => {
    expect(normalizeUrl('docs/guide')).toBe('/docs/guide/');
    expect(normalizeUrl('/docs/guide')).toBe('/docs/guide/');
    expect(normalizeUrl('docs/guide/')).toBe('/docs/guide/');
  });

  it('collapses duplicate slashes', () => {
    expect(normalizeUrl('/docs//guide/')).toBe('/docs/guide/');
  });
});

describe('parentFolder', () => {
  it('returns the directory of a file path', () => {
    expect(parentFolder('docs/guide.md')).toBe('docs');
    expect(parentFolder('a/b/c/note.md')).toBe('a/b/c');
  });

  it('returns the repo root for a top-level file', () => {
    expect(parentFolder('README.md')).toBe('');
  });
});

describe('hasMarkdownExtension', () => {
  it('accepts .md and .markdown leaves', () => {
    expect(hasMarkdownExtension('docs/guide.md')).toBe(true);
    expect(hasMarkdownExtension('docs/guide.markdown')).toBe(true);
    expect(hasMarkdownExtension('docs/GUIDE.MD')).toBe(true);
  });

  it('rejects non-markdown or extension-less leaves', () => {
    expect(hasMarkdownExtension('docs/photo.png')).toBe(false);
    expect(hasMarkdownExtension('docs/guide')).toBe(false);
    expect(hasMarkdownExtension('docs/.hidden')).toBe(false);
  });

  it('honours a custom extension list', () => {
    expect(hasMarkdownExtension('a/b.mdx', ['mdx'])).toBe(true);
    expect(hasMarkdownExtension('a/b.md', ['mdx'])).toBe(false);
  });
});

describe('deriveExistingFolders', () => {
  it('includes the repo root and every confident ancestor', () => {
    const folders = deriveExistingFolders([
      '/a/b/guide/',
      '/a/intro/',
      '/top/',
    ]);
    expect(folders.has('')).toBe(true);
    expect(folders.has('a')).toBe(true);
    expect(folders.has('a/b')).toBe(true);
    // '/top/' -> file 'top' lives in the repo root, no sub-folder implied.
    expect(folders.has('top')).toBe(false);
  });

  it('handles the home page (root index) without inventing folders', () => {
    const folders = deriveExistingFolders(['/']);
    expect([...folders]).toEqual(['']);
  });
});

describe('fsPathToApproxUrl', () => {
  it('maps a normal markdown file to its directory-style URL', () => {
    expect(fsPathToApproxUrl('docs/guide.md')).toBe('/docs/guide/');
    expect(fsPathToApproxUrl('README.md')).toBe('/README/');
  });

  it('collapses an index file onto its folder URL', () => {
    expect(fsPathToApproxUrl('docs/index.md')).toBe('/docs/');
    expect(fsPathToApproxUrl('index.md')).toBe('/');
  });
});

describe('relativeUrlPath', () => {
  it('walks up then down between sibling trees', () => {
    expect(relativeUrlPath('/a/b/guide/', '/a/c/note/')).toBe('../../c/note/');
  });

  it('descends from a folder index page', () => {
    expect(relativeUrlPath('/a/', '/a/b/note/')).toBe('./b/note/');
  });

  it('ascends from the root', () => {
    expect(relativeUrlPath('/', '/a/')).toBe('./a/');
  });

  it('returns ./ for a self reference', () => {
    expect(relativeUrlPath('/a/b/guide/', '/a/b/guide/')).toBe('./');
  });
});

describe('encodeLinkDestination', () => {
  it('escapes spaces and parentheses but keeps slashes', () => {
    expect(encodeLinkDestination('../my notes/a (b)/')).toBe('../my%20notes/a%20%28b%29/');
  });

  it('escapes a literal percent first', () => {
    expect(encodeLinkDestination('a%b')).toBe('a%25b');
  });
});
