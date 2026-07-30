/**
 * Tests for the page-problems summary sentence.
 *
 * `summarizePageErrors` is pure and exported precisely so the wording can be
 * asserted without mounting the element or stubbing `errors.json`.
 */
import { describe, expect, it } from 'vitest';
import { summarizePageErrors } from './mbr-page-errors.ts';

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
