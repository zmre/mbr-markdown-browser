import { describe, expect, it } from 'vitest';
import {
  lineEndingOf,
  recombine,
  splitFrontmatter,
  toLf,
  unescapeWikilinks,
  withLineEnding,
} from './editor-frontmatter.js';

describe('splitFrontmatter', () => {
  it('splits a document with frontmatter', () => {
    const raw = '---\ntitle: Hello\ntags: [a, b]\n---\n# Body\n\ntext';
    const { frontmatter, body } = splitFrontmatter(raw);
    expect(frontmatter).toBe('title: Hello\ntags: [a, b]');
    expect(body).toBe('# Body\n\ntext');
  });

  it('returns the whole document as body when there is no frontmatter', () => {
    const raw = '# Just a heading\n\nno frontmatter here';
    const { frontmatter, body } = splitFrontmatter(raw);
    expect(frontmatter).toBeNull();
    expect(body).toBe(raw);
  });

  it('does not mistake a body horizontal rule for frontmatter', () => {
    const raw = 'intro\n\n---\n\nmore';
    const { frontmatter, body } = splitFrontmatter(raw);
    expect(frontmatter).toBeNull();
    expect(body).toBe(raw);
  });

  it('does not split when the opening fence is never closed', () => {
    const raw = '---\ntitle: unterminated\n\nbody continues';
    const { frontmatter, body } = splitFrontmatter(raw);
    expect(frontmatter).toBeNull();
    expect(body).toBe(raw);
  });
});

describe('recombine', () => {
  it('rebuilds a document with frontmatter', () => {
    expect(recombine('title: Hello', '# Body')).toBe('---\ntitle: Hello\n---\n# Body');
  });

  it('omits the fence block when frontmatter is empty or null', () => {
    expect(recombine(null, '# Body')).toBe('# Body');
    expect(recombine('   ', '# Body')).toBe('# Body');
  });
});

describe('unescapeWikilinks', () => {
  it('restores a fully escaped wikilink', () => {
    expect(unescapeWikilinks('\\[\\[John Doe\\]\\]')).toBe('[[John Doe]]');
  });

  it('restores a partially escaped wikilink', () => {
    expect(unescapeWikilinks('\\[\\[John Doe]]')).toBe('[[John Doe]]');
  });

  it('restores an escaped embed', () => {
    expect(unescapeWikilinks('!\\[\\[img.png\\]\\]')).toBe('![[img.png]]');
  });

  it('restores an escaped alias (interior pipe)', () => {
    expect(unescapeWikilinks('\\[\\[Name\\|alias\\]\\]')).toBe('[[Name|alias]]');
  });

  it('restores an escaped anchor (interior hash)', () => {
    expect(unescapeWikilinks('\\[\\[Name\\#Sec\\]\\]')).toBe('[[Name#Sec]]');
  });

  it('restores multiple wikilinks within surrounding prose', () => {
    expect(unescapeWikilinks('See \\[\\[John Doe\\]\\] and \\[\\[Jane\\]\\].')).toBe(
      'See [[John Doe]] and [[Jane]].',
    );
  });

  it('is idempotent on already-unescaped wikilinks', () => {
    expect(unescapeWikilinks('[[John Doe]]')).toBe('[[John Doe]]');
    expect(unescapeWikilinks(unescapeWikilinks('\\[\\[John Doe\\]\\]'))).toBe('[[John Doe]]');
  });

  it('leaves a normal markdown link untouched', () => {
    expect(unescapeWikilinks('[text](url)')).toBe('[text](url)');
  });

  it('leaves a lone escaped bracket pair (not a wikilink) as-is', () => {
    expect(unescapeWikilinks('\\[not a wikilink\\]')).toBe('\\[not a wikilink\\]');
  });
});

describe('round-trip', () => {
  const cases = [
    '---\ntitle: Hello\ntags: [a, b]\n---\n# Body\n\ntext',
    '---\ntitle: x\n---\n\nbody with leading blank line',
    '# no frontmatter\n\njust body',
    'intro\n\n---\n\nrule in body',
  ];

  for (const raw of cases) {
    it(`is idempotent for ${JSON.stringify(raw.slice(0, 24))}...`, () => {
      const { frontmatter, body } = splitFrontmatter(raw);
      expect(recombine(frontmatter, body)).toBe(raw);
    });
  }
});

describe('line endings', () => {
  it('detects CRLF and LF by majority', () => {
    expect(lineEndingOf('a\r\nb\r\n')).toBe('\r\n');
    expect(lineEndingOf('a\nb\n')).toBe('\n');
    expect(lineEndingOf('a\r\nb\r\nc\n')).toBe('\r\n');
    expect(lineEndingOf('a\r\nb\nc\n')).toBe('\n');
    expect(lineEndingOf('no line break')).toBe('\n');
    expect(lineEndingOf('')).toBe('\n');
  });

  it('converts to LF and back', () => {
    expect(toLf('a\r\nb\r\n')).toBe('a\nb\n');
    expect(withLineEnding('a\nb\n', '\r\n')).toBe('a\r\nb\r\n');
    expect(withLineEnding('a\r\nb\n', '\r\n')).toBe('a\r\nb\r\n');
    expect(withLineEnding('a\nb\n', '\n')).toBe('a\nb\n');
  });

  it('round-trips a CRLF document with frontmatter', () => {
    const raw = '---\r\ntitle: T\r\n---\r\n# H\r\n\r\nBody\r\n';
    const { frontmatter, body } = splitFrontmatter(toLf(raw));
    expect(withLineEnding(recombine(frontmatter, body), lineEndingOf(raw))).toBe(raw);
  });
});
