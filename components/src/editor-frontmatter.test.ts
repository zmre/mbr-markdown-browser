import { describe, expect, it } from 'vitest';
import { recombine, splitFrontmatter } from './editor-frontmatter.js';

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
