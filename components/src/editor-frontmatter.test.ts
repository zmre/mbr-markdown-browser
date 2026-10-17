import { describe, expect, it } from 'vitest';
import {
  withAuthoredBlankLines,
  withAuthoredEnd,
  documentToSave,
  frontmatterCloseLine,
  frontmatterRewrites,
  lineEndingOf,
  openDocument,
  recombine,
  splitFrontmatter,
  toLf,
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

describe('frontmatterCloseLine: the renderer\'s rule (pulldown-cmark)', () => {
  const close = (text: string) => frontmatterCloseLine(text.split('\n'));

  it('finds `---` and `...` closers, with trailing spaces and CR', () => {
    expect(close('---\na: 1\n---\nbody')).toBe(2);
    expect(close('---\na: 1\n...\nbody')).toBe(2);
    expect(close('---  \na: 1\n---  \nbody')).toBe(2);
    expect(close('---\r\na: 1\r\n---\r\nbody')).toBe(2);
    expect(close('---\n# only a comment\n---\n')).toBe(2);
  });

  it('is no frontmatter when the first line is blank or a closer, or nothing closes it', () => {
    expect(close('---\n---\n# H')).toBe(-1);
    expect(close('---\n...\n# H')).toBe(-1);
    expect(close('---\n\na: 1\n---\n')).toBe(-1);
    expect(close('---\n  \na: 1\n---\n')).toBe(-1);
    expect(close('---\na: 1\n')).toBe(-1);
    expect(close('----\na: 1\n----\n')).toBe(-1);
    expect(close('--- x\na: 1\n---\n')).toBe(-1);
    expect(close('# H\n---\na: 1\n---\n')).toBe(-1);
  });

  it('splits on it', () => {
    expect(splitFrontmatter('---\n---\n# H').frontmatter).toBeNull();
    expect(splitFrontmatter('---\ntitle: t\n\n...\n# H')).toEqual({ frontmatter: 'title: t\n', body: '# H' });
  });
});

describe('openDocument / documentToSave', () => {
  /** An untouched save: the textarea as opened, the body as opened. */
  const untouched = (raw: string) => {
    const doc = openDocument(raw);
    return documentToSave(doc, doc.frontmatter ?? '', doc.body);
  };

  const exact = [
    '---\ntitle: t\n\n\n---\n# H\n\nBody\n',
    '---\ntitle: t\n---\n\n\n# H\n',
    '---   \ntitle: t\n...  \n\nBody\n',
    '---\n# comment only\n---\nBody\n',
    '\uFEFF---\r\ntitle: t\r\n\r\n---\r\n\r\nBody\r\n',
    '---\r\ntitle: t\n---\r\n\r\nmixed endings in the frontmatter\r\n',
    '---\ntitle: t\n---',
    '---\ntitle: t\n---\n',
    '---\n---\n\n# H\n',
    '\uFEFF# no frontmatter\n',
    '\n\n# leading blank lines\n',
    '',
  ];
  for (const raw of exact) {
    it(`writes untouched frontmatter back byte for byte: ${JSON.stringify(raw.slice(0, 30))}`, () => {
      expect(untouched(raw)).toBe(raw);
    });
  }

  it('opens the body without the frontmatter, its separator or a byte-order mark', () => {
    const doc = openDocument('\uFEFF---\r\ntitle: t\r\n\r\n---\r\n\r\n\r\n# H\r\n');
    expect(doc).toEqual({
      lineEnding: '\r\n',
      frontmatter: 'title: t\n',
      body: '# H\n',
      bodyLineOffset: 6,
      head: '\uFEFF---\r\ntitle: t\r\n\r\n---\r\n\r\n\r\n',
      separator: '\n\n',
    });
    expect(openDocument('---\n---\n\n# H\n')).toMatchObject({ frontmatter: null, body: '---\n---\n\n# H\n', bodyLineOffset: 0 });
  });

  it('keeps the frontmatter as loaded when only the body is edited', () => {
    const doc = openDocument('---\ntitle: t\n\n...\n\n# H\n');
    expect(documentToSave(doc, 'title: t\n', '# Edited\n')).toBe('---\ntitle: t\n\n...\n\n# Edited\n');
    // A textarea hands CRLF back as LF: still unedited.
    const crlf = openDocument('---\r\ntitle: t\r\n---\r\n# H\r\n');
    expect(documentToSave(crlf, 'title: t', '# H\n')).toBe('---\r\ntitle: t\r\n---\r\n# H\r\n');
  });

  it('normalizes an edited frontmatter, keeping the separator, ending and byte-order mark', () => {
    const doc = openDocument('\uFEFF---  \r\ntitle: t\r\n\r\n...\r\n\r\n# H\r\n');
    expect(documentToSave(doc, 'title: u\n', '# H\n')).toBe('\uFEFF---\r\ntitle: u\r\n---\r\n\r\n# H\r\n');
    // Cleared: no block, and no blank lines left at the top of the file.
    expect(documentToSave(doc, '  ', '# H\n')).toBe('\uFEFF# H\r\n');
    // Added to a file without one.
    expect(documentToSave(openDocument('# H\n'), 'title: t', '# H\n')).toBe('---\ntitle: t\n---\n# H\n');
  });
});

describe('frontmatterRewrites: what editing the frontmatter normalizes', () => {
  const rewrites = (raw: string) => frontmatterRewrites(openDocument(raw));

  it('is empty when an edit writes the block as it was spelled', () => {
    expect(rewrites('# no frontmatter\n')).toEqual([]);
    expect(rewrites('---\ntitle: t\n---\n\n# H\n')).toEqual([]);
    expect(rewrites('---\r\ntitle: t\r\n---\r\n# H\r\n')).toEqual([]);
    expect(rewrites('\uFEFF---\ntitle: t\n---\n# H\n')).toEqual([]);
    expect(rewrites('---\ntitle: t\n---')).toEqual([]);
  });

  it('names fences, blank lines at the end, and mixed line endings', () => {
    expect(rewrites('---\ntitle: t\n\n---\n# H\n')).toEqual(['blank lines at its end are dropped']);
    expect(rewrites('---\ntitle: t\n...\n# H\n')).toEqual(['its fences are written `---`']);
    expect(rewrites('--- \ntitle: t\n---\n# H\n')).toEqual(['its fences are written `---`']);
    expect(rewrites('---\ntitle: t\n\n...\n# H\n')).toEqual([
      'its fences are written `---`',
      'blank lines at its end are dropped',
    ]);
    expect(rewrites('---\r\ntitle: t\n---\r\n# H\r\n')).toEqual(['its line endings are made uniform']);
  });
});

describe('withAuthoredEnd: the body ends as loaded', () => {
  it('drops the printer\'s line break when the note had none', () => {
    expect(withAuthoredEnd('a\n', 'a')).toBe('a');
  });

  it('keeps blank lines (and their spaces) after the last line', () => {
    expect(withAuthoredEnd('a\n', 'a\n\n\n')).toBe('a\n\n\n');
    expect(withAuthoredEnd('a\n', 'a  \n \n')).toBe('a\n \n');
  });

  it('leaves a body that does not end in one line break alone', () => {
    expect(withAuthoredEnd('a', 'a\n')).toBe('a');
    expect(withAuthoredEnd('a\n\n', 'a')).toBe('a\n\n');
    expect(withAuthoredEnd('a\n', '')).toBe('a\n');
  });
});

describe('withAuthoredBlankLines: whitespace-only lines as loaded', () => {
  it('puts the whitespace back between the same neighbours', () => {
    expect(withAuthoredBlankLines('a\n\nb\n\nc\n', 'a\n  \nb\n\t\nc\n')).toBe('a\n  \nb\n\t\nc\n');
  });

  it('matches each line of a run of blank lines by its place in the run', () => {
    expect(withAuthoredBlankLines('a\n\n\nb\n', 'a\n\n  \nb\n')).toBe('a\n\n  \nb\n');
  });

  it('leaves a blank line whose neighbours changed empty', () => {
    expect(withAuthoredBlankLines('a!\n\nb\n', 'a\n  \nb\n')).toBe('a!\n\nb\n');
  });

  it('never touches a non-empty line', () => {
    expect(withAuthoredBlankLines('a\n    x\nb\n', 'a\n  \nb\n')).toBe('a\n    x\nb\n');
  });

  it('keeps a whitespace-only last line and the line break after it, with the end kept', () => {
    // The empty string after the final line break is not a blank line to fill.
    expect(withAuthoredBlankLines('p\n', 'p\n \n')).toBe('p\n');
    expect(withAuthoredEnd(withAuthoredBlankLines('p\n', 'p\n \n'), 'p\n \n')).toBe('p\n \n');
    expect(withAuthoredEnd(withAuthoredBlankLines('p\n', 'p\n\t'), 'p\n\t')).toBe('p\n\t');
  });
});
