import { describe, expect, it, vi } from 'vitest';
import { Document, isMap, isScalar, isSeq, parse, stringify } from 'yaml';
import {
  addItem,
  addKey,
  entries,
  FrontmatterEditError,
  getIn,
  jsEqual,
  nodeInfo,
  openFrontmatter,
  removeItem,
  removeKey,
  renameKey,
  replaceSubtree,
  setScalar,
  setValue,
  sliceAt,
  toValue,
  type FrontmatterModel,
} from './model.js';

const SAMPLE = `# Leading comment
title: Hello world  # the title
tags: [a, "b", c]
desc: |
  line one
  line two

nested:
  k: 1
  list:
    - x
    - y: 2
      z: 3
empty:
flow: {a: 1, b: 2}
quoted: 'single'
dq: "double"
# trailing comment`;

const open = (text: string) => openFrontmatter(text);
const text = (m: FrontmatterModel) => m.text;

/** The edit changed only `[start, end)` of the old text. */
function expectOnlySpan(before: string, after: string, start: number, end: number): void {
  expect(after.startsWith(before.slice(0, start))).toBe(true);
  expect(after.endsWith(before.slice(end))).toBe(true);
}

describe('openFrontmatter', () => {
  it('keeps the text verbatim and parses it', () => {
    const m = open(SAMPLE);
    expect(m.text).toBe(SAMPLE);
    expect(m.errors).toEqual([]);
    expect(m.normalized).toBe(false);
    expect(getIn(m, ['nested', 'list', 1, 'z'])).toBe(3);
    expect(getIn(m, ['missing'])).toBeUndefined();
    expect(getIn(m, ['empty'])).toBeNull();
  });

  it('reads dates as strings (YAML 1.2 core, like the renderer)', () => {
    const m = open('date: 2026-10-21\nyes: yes\nn: 1e3');
    expect(toValue(m)).toEqual({ date: '2026-10-21', yes: 'yes', n: 1000 });
  });

  it('treats an empty or comment-only block as an empty map', () => {
    expect(toValue(open(''))).toEqual({});
    expect(toValue(open('# just a comment'))).toEqual({});
  });

  it('reports parse errors with a line', () => {
    const m = open('a: [1\nb: 2');
    expect(m.errors.length).toBeGreaterThan(0);
    expect(m.errors[0].line).toBeGreaterThanOrEqual(1);
    expect(m.errors[0].message).not.toContain('\n');
  });

  it('reports duplicate keys as errors', () => {
    expect(open('a: 1\na: 2').errors.length).toBeGreaterThan(0);
  });
});

describe('refusing broken YAML', () => {
  const broken = open('title: x\ntags: [a, b\nother: 1');
  const ops: [string, () => unknown][] = [
    ['setScalar', () => setScalar(broken, ['title'], 'y')],
    ['setValue', () => setValue(broken, ['title'], ['y'])],
    ['addKey', () => addKey(broken, [], 'k', 1)],
    ['addItem', () => addItem(broken, ['tags'], 'c')],
    ['removeKey', () => removeKey(broken, ['title'])],
    ['removeItem', () => removeItem(broken, ['tags'], 0)],
    ['renameKey', () => renameKey(broken, ['title'], 'name')],
    ['replaceSubtree', () => replaceSubtree(broken, ['title'], 'z')],
  ];
  for (const [name, op] of ops) {
    it(`${name} throws parse-errors`, () => {
      expect(op).toThrow(FrontmatterEditError);
      try {
        op();
      } catch (e) {
        expect((e as FrontmatterEditError).reason).toBe('parse-errors');
      }
    });
  }
});

describe('a value that cannot be read', () => {
  // No parse error, but `toJS` refuses: the aliases expand past `maxAliasCount`.
  const LAUGHS = [
    'a: &a [x, x, x, x, x, x, x, x, x]',
    'b: &b [*a, *a, *a, *a, *a, *a, *a, *a, *a]',
    'c: &c [*b, *b, *b, *b, *b, *b, *b, *b, *b]',
    'd: &d [*c, *c, *c, *c, *c, *c, *c, *c, *c]',
    'e: &e [*d, *d, *d, *d, *d, *d, *d, *d, *d]',
  ].join('\n');

  it('is a model error, and reading it does not throw', () => {
    const m = open(LAUGHS);
    expect(m.errors).toHaveLength(1);
    expect(m.errors[0].code).toBe('BAD_VALUE');
    expect(m.errors[0].message).toMatch(/alias/i);
    expect(m.errors[0].line).toBe(2);
    expect(toValue(m)).toEqual({});
    expect(getIn(m, ['a'])).toBeUndefined();
    expect(() => setScalar(m, ['a', 0], 'y')).toThrow(FrontmatterEditError);
  });
});

describe('reading the value', () => {
  it('runs toJS once per model, however often it is read', () => {
    const spy = vi.spyOn(Document.prototype, 'toJS');
    try {
      const m = open(SAMPLE);
      for (let i = 0; i < 5; i++) {
        toValue(m);
        getIn(m, ['nested', 'list', 1, 'z']);
        entries(m, ['nested']);
      }
      expect(spy).toHaveBeenCalledTimes(1);
      // An edit is a new model: one more.
      const next = setScalar(m, ['title'], 'Hi');
      spy.mockClear();
      getIn(next, ['title']);
      toValue(next);
      expect(spy).toHaveBeenCalledTimes(0);
    } finally {
      spy.mockRestore();
    }
  });

  it('hands out a frozen value, so no reader can change what the next one sees', () => {
    const v = toValue(open('a:\n  - 1\n')) as { a: number[] };
    expect(Object.isFrozen(v)).toBe(true);
    expect(Object.isFrozen(v.a)).toBe(true);
  });
});

describe('setScalar', () => {
  it('keeps a trailing comment through clearing and refilling a value (a form field emptied then typed)', () => {
    const cleared = setScalar(open('title: Old   # c\nn: 1'), ['title'], null);
    expect(text(cleared)).toBe('title:   # c\nn: 1');
    expect(text(setScalar(cleared, ['title'], 'New'))).toBe('title: New   # c\nn: 1');
  });

  it('changes only the scalar, keeping its trailing comment', () => {
    const m = setScalar(open(SAMPLE), ['title'], 'Goodbye');
    expect(text(m)).toBe(SAMPLE.replace('Hello world  #', 'Goodbye  #'));
    expect(m.lastEdit).not.toBeNull();
    expect(m.normalized).toBe(false);
  });

  it('keeps single and double quotes', () => {
    let m = setScalar(open(SAMPLE), ['quoted'], "it's");
    expect(text(m)).toContain(`quoted: 'it''s'`);
    m = setScalar(m, ['dq'], 'say "hi"');
    expect(text(m)).toContain(`dq: "say \\"hi\\""`);
  });

  it('quotes a plain scalar only when plain would change its meaning', () => {
    const cases: [string, string][] = [
      ['plain words', 'plain words'],
      ['a: b', '"a: b"'],
      ['# not a comment', '"# not a comment"'],
      ['123', '"123"'],
      ['true', '"true"'],
      ['null', '"null"'],
      ['', '""'],
      [' padded ', '" padded "'],
      ['[x]', '"[x]"'],
      ['ünïcödé ✓', 'ünïcödé ✓'],
      ['- dash', '"- dash"'],
      ['2026-10-21', '2026-10-21'],
    ];
    for (const [value, expected] of cases) {
      const m = setScalar(open('k: old\nnext: 1'), ['k'], value);
      expect(text(m)).toBe(`k: ${expected}\nnext: 1`);
      expect(getIn(m, ['k'])).toBe(value);
    }
  });

  it('writes numbers, booleans and null', () => {
    expect(text(setScalar(open('n: 1'), ['n'], 42))).toBe('n: 42');
    expect(text(setScalar(open('n: 1'), ['n'], 1.5))).toBe('n: 1.5');
    expect(text(setScalar(open('b: false'), ['b'], true))).toBe('b: true');
    expect(getIn(setScalar(open('b: x'), ['b'], null), ['b'])).toBeNull();
  });

  it('fills an empty value', () => {
    const m = setScalar(open(SAMPLE), ['empty'], 'now');
    expect(text(m)).toContain('\nempty: now\nflow:');
    expect(m.normalized).toBe(false);
  });

  it('edits a flow item in place, quoting for flow context', () => {
    let m = setScalar(open(SAMPLE), ['tags', 0], 'alpha');
    expect(text(m)).toContain('tags: [alpha, "b", c]');
    m = setScalar(m, ['tags', 2], 'x, y');
    expect(text(m)).toContain('tags: [alpha, "b", "x, y"]');
    m = setScalar(m, ['flow', 'b'], 'two');
    expect(text(m)).toContain('flow: {a: 1, b: two}');
  });

  it('rewrites a block scalar as a block while it has line breaks', () => {
    const m = setScalar(open(SAMPLE), ['desc'], 'first\nsecond\n');
    expect(text(m)).toContain('desc: |\n  first\n  second\n\nnested:');
    const flat = setScalar(open(SAMPLE), ['desc'], 'one line');
    expect(text(flat)).toContain('desc: one line\n\nnested:');
  });

  it('turns a multi-line value into a literal block', () => {
    const m = setScalar(open('a: x\nb: 1'), ['a'], 'one\ntwo');
    expect(text(m)).toBe('a: |-\n  one\n  two\nb: 1');
    expect(getIn(m, ['a'])).toBe('one\ntwo');
  });

  it('keeps a tag and an anchor', () => {
    const m = setScalar(open('a: !!str 12\nb: &x foo'), ['a'], '13');
    // The tag stays; the value is quoted because plain `13` alone would be a number.
    expect(text(m)).toBe('a: !!str "13"\nb: &x foo');
    expect(text(setScalar(m, ['b'], 'bar'))).toBe('a: !!str "13"\nb: &x bar');
  });

  it('adds a missing key', () => {
    expect(text(setScalar(open('a: 1'), ['b'], 'x'))).toBe('a: 1\nb: x');
  });

  it('changes nothing else in the sample', () => {
    const before = SAMPLE;
    const m = setScalar(open(before), ['nested', 'list', 1, 'z'], 99);
    const start = before.indexOf('z: 3') + 3;
    expectOnlySpan(before, m.text, start, start + 1);
  });

  /** `value` typed one keystroke at a time, as the form's text fields write it. */
  const typeInto = (m: FrontmatterModel, path: (string | number)[], value: string): FrontmatterModel => {
    for (let i = 1; i <= value.length; i++) m = setScalar(m, path, value.slice(0, i));
    return m;
  };

  it('a plain scalar quoted only while a keystroke needed it goes back to plain', () => {
    // `hello ` (a trailing space) has to be quoted; `hello world` does not.
    expect(text(typeInto(open('title: x\nn: 1'), ['title'], 'hello world'))).toBe('title: hello world\nn: 1');
    expect(text(typeInto(open('title:  # c\nn: 1'), ['title'], 'a: b c'))).toBe('title: "a: b c"  # c\nn: 1');
    expect(text(typeInto(open('title:\nn: 1'), ['title'], '#1 fan'))).toBe('title: "#1 fan"\nn: 1');
    expect(text(typeInto(open('title:\nn: 1'), ['title'], '1 fan'))).toBe('title: 1 fan\nn: 1');
    // A list item and a flow item (the list widget, the enum combobox inside a list).
    expect(text(typeInto(open('l:\n  - x\n'), ['l', 0], 'two words'))).toBe('l:\n  - two words\n');
    expect(text(typeInto(open('l: [x, y]'), ['l', 1], 'two words'))).toBe('l: [x, two words]');
    // Cleared and retyped (the field emptied first writes `key:`).
    const cleared = setScalar(typeInto(open('t: x'), ['t'], 'a '), ['t'], null);
    expect(text(typeInto(cleared, ['t'], 'b c'))).toBe('t: b c');
  });

  it("keeps an author's quotes while typing", () => {
    expect(text(typeInto(open('title: "x"'), ['title'], 'hello world'))).toBe('title: "hello world"');
    expect(text(typeInto(open("title: 'x'"), ['title'], 'hello world'))).toBe("title: 'hello world'");
  });

  it('forgets a forced quote when the value or its parent is replaced', () => {
    const quoted = setScalar(open('a:\n  b: x\n'), ['a', 'b'], 'x ');
    expect(quoted.forcedQuotes.has(JSON.stringify(['a', 'b']))).toBe(true);
    expect(setValue(quoted, ['a'], { b: 'y' }).forcedQuotes.size).toBe(0);
    expect(setScalar(quoted, ['a', 'b'], 'x y').forcedQuotes.size).toBe(0);
    // Paths may move: a removal or rename clears them all.
    expect(removeKey(quoted, ['a', 'b']).forcedQuotes.size).toBe(0);
  });
});

describe('setValue with collections', () => {
  it('turns a scalar into a block list, keeping its comment on the key line', () => {
    const m = setValue(open('a: x  # note\nb: 1'), ['a'], ['p', 'q']);
    expect(text(m)).toBe('a:  # note\n  - p\n  - q\nb: 1');
    expect(getIn(m, ['a'])).toEqual(['p', 'q']);
  });

  it('follows the document’s unindented sequence style', () => {
    const m = setValue(open('tags:\n- a\nb: x'), ['b'], ['p']);
    expect(text(m)).toBe('tags:\n- a\nb:\n- p');
  });

  it('writes a map value', () => {
    const m = setValue(open('a: 1\nb: 2'), ['a'], { x: 1, y: 'two' });
    expect(text(m)).toBe('a:\n  x: 1\n  y: two\nb: 2');
  });

  it('replaces a nested block value', () => {
    const m = setValue(open(SAMPLE), ['nested', 'list'], ['only']);
    expect(text(m)).toContain('  list:\n    - only\nempty:');
  });

  it('writes flow inside flow', () => {
    const m = setValue(open('f: {a: 1, b: 2}'), ['f', 'b'], [1, 2]);
    expect(text(m)).toBe('f: {a: 1, b: [1, 2]}');
  });

  it('replaces the root through the document API (normalized)', () => {
    const m = setValue(open('a: 1'), [], { b: 2 });
    expect(toValue(m)).toEqual({ b: 2 });
    expect(m.normalized).toBe(true);
  });
});

describe('addKey', () => {
  it('appends after the last key, leaving a trailing comment last', () => {
    const m = addKey(open(SAMPLE), [], 'new', 'value');
    expect(text(m)).toBe(SAMPLE.replace('\n# trailing comment', '\nnew: value\n# trailing comment'));
    expect(m.normalized).toBe(false);
  });

  it('copies the sibling indentation in a nested map', () => {
    const m = addKey(open(SAMPLE), ['nested'], 'added', true);
    expect(text(m)).toContain('      z: 3\n  added: true\nempty:');
  });

  it('adds to a map inside a sequence item', () => {
    const m = addKey(open(SAMPLE), ['nested', 'list', 1], 'w', 4);
    expect(text(m)).toContain('    - y: 2\n      z: 3\n      w: 4\nempty:');
  });

  it('adds inside a flow map', () => {
    expect(text(addKey(open('f: {a: 1}'), ['f'], 'b', 'two'))).toBe('f: {a: 1, b: two}');
    expect(text(addKey(open('f: {}'), ['f'], 'b', 2))).toBe('f: {b: 2}');
  });

  it('adds to an empty document and a comment-only one', () => {
    expect(text(addKey(open(''), [], 'title', 'x'))).toBe('title: x');
    expect(text(addKey(open('# c'), [], 'title', 'x'))).toBe('# c\ntitle: x');
    expect(text(addKey(open('# c\n'), [], 'title', 'x'))).toBe('# c\ntitle: x\n');
  });

  it('turns an empty value into a map', () => {
    expect(text(addKey(open('a:\nb: 1'), ['a'], 'k', 'v'))).toBe('a:\n  k: v\nb: 1');
  });

  it('adds collection values in block style', () => {
    expect(text(addKey(open('a: 1'), [], 'tags', ['x', 'y']))).toBe('a: 1\ntags:\n  - x\n  - y');
    expect(text(addKey(open('a: 1'), [], 'tags', []))).toBe('a: 1\ntags: []');
    expect(text(addKey(open('a: 1'), [], 'm', { k: [1] }))).toBe('a: 1\nm:\n  k:\n    - 1');
  });

  it('quotes awkward keys', () => {
    const m = addKey(open('a: 1'), [], 'has: colon', 1);
    expect(text(m)).toBe('a: 1\n"has: colon": 1');
    expect(getIn(m, ['has: colon'])).toBe(1);
  });

  it('refuses an existing key and a non-map', () => {
    expect(() => addKey(open('a: 1'), [], 'a', 2)).toThrow(/already exists/);
    expect(() => addKey(open('a: [1]'), ['a'], 'k', 2)).toThrow(/not a map/);
  });

  it('works after a final block scalar', () => {
    const m = addKey(open('a: |\n  x\n  y\n'), [], 'b', 1);
    expect(text(m)).toBe('a: |\n  x\n  y\nb: 1\n');
  });
});

describe('addItem', () => {
  it('appends to a block sequence with the sibling indentation', () => {
    const m = addItem(open(SAMPLE), ['nested', 'list'], 'new');
    expect(text(m)).toContain('      z: 3\n    - new\nempty:');
  });

  it('appends a map item compactly', () => {
    const m = addItem(open('rel:\n  - type: parent\n    to: "[[A]]"'), ['rel'], { type: 'child', to: '[[B]]' });
    expect(text(m)).toBe('rel:\n  - type: parent\n    to: "[[A]]"\n  - type: child\n    to: "[[B]]"');
  });

  it('appends inside a flow sequence', () => {
    expect(text(addItem(open(SAMPLE), ['tags'], 'd'))).toContain('tags: [a, "b", c, d]');
    expect(text(addItem(open('t: []'), ['t'], 'x'))).toBe('t: [x]');
    expect(text(addItem(open('t: [a]'), ['t'], 'x, y'))).toBe('t: [a, "x, y"]');
  });

  it('turns an empty value into a list', () => {
    expect(text(addItem(open('t:\nb: 1'), ['t'], 'x'))).toBe('t:\n  - x\nb: 1');
  });

  it('refuses a non-list', () => {
    expect(() => addItem(open('a: 1'), ['a'], 'x')).toThrow(/not a list/);
  });
});

describe('removeKey / removeItem', () => {
  it('removes a key line, keeping comments on other lines', () => {
    const m = removeKey(open('a: 1  # mine\n# about b\nb: 2\nc: 3'), ['a']);
    expect(text(m)).toBe('# about b\nb: 2\nc: 3');
  });

  it('removes a key with a nested block value', () => {
    const m = removeKey(open(SAMPLE), ['nested']);
    expect(text(m)).toContain('desc: |\n  line one\n  line two\n\nempty:\n');
    expect(m.normalized).toBe(false);
  });

  it('removes the last key of a text without a final newline', () => {
    expect(text(removeKey(open('a: 1\nb: 2'), ['b']))).toBe('a: 1');
  });

  it('removes the only key of the root', () => {
    expect(toValue(removeKey(open('a: 1'), ['a']))).toEqual({});
  });

  it('leaves an empty collection when the last nested item goes', () => {
    expect(text(removeKey(open('m:\n  k: 1\nb: 2'), ['m', 'k']))).toBe('m: {}\nb: 2');
    expect(text(removeItem(open('l:\n  - 1\nb: 2'), ['l'], 0))).toBe('l: []\nb: 2');
  });

  it('removes block sequence items', () => {
    const m = removeItem(open(SAMPLE), ['nested', 'list'], 0);
    expect(text(m)).toContain('  list:\n    - y: 2\n      z: 3\nempty:');
    const m2 = removeItem(open(SAMPLE), ['nested', 'list'], 1);
    expect(text(m2)).toContain('  list:\n    - x\nempty:');
  });

  it('removes the first key of a map inside a sequence item', () => {
    const m = removeKey(open('l:\n  - a: 1\n    b: 2'), ['l', 0, 'a']);
    expect(text(m)).toBe('l:\n  - b: 2');
  });

  it('removes flow items with their commas', () => {
    expect(text(removeItem(open(SAMPLE), ['tags'], 0))).toContain('tags: ["b", c]');
    expect(text(removeItem(open(SAMPLE), ['tags'], 1))).toContain('tags: [a, c]');
    expect(text(removeItem(open(SAMPLE), ['tags'], 2))).toContain('tags: [a, "b"]');
    expect(text(removeItem(open('t: [a]'), ['t'], 0))).toBe('t: []');
    expect(text(removeKey(open('f: {a: 1, b: 2}'), ['f', 'b']))).toBe('f: {a: 1}');
  });

  it('leaves comment lines after a nested collection to what follows', () => {
    const src = 'm:\n  a: 1\n  z:\n  - x  # mine\n  # about b\n  b: 2\nc: 3';
    expect(text(removeKey(open(src), ['m', 'z']))).toBe('m:\n  a: 1\n  # about b\n  b: 2\nc: 3');
  });

  it('refuses missing paths and wrong kinds', () => {
    expect(() => removeKey(open('a: 1'), ['z'])).toThrow(/no value/);
    expect(() => removeItem(open('a: 1'), ['a'], 0)).toThrow(FrontmatterEditError);
    expect(() => removeKey(open('a: [1]'), ['a', 0])).toThrow(/not a key/);
  });
});

describe('renameKey', () => {
  it('rewrites only the key token', () => {
    const m = renameKey(open(SAMPLE), ['title'], 'name');
    expect(text(m)).toBe(SAMPLE.replace('title: Hello', 'name: Hello'));
    expect(Object.keys(toValue(m) as object)[0]).toBe('name');
  });

  it('keeps quoting style, quoting when needed', () => {
    expect(text(renameKey(open('"a": 1'), ['a'], 'b'))).toBe('"b": 1');
    expect(text(renameKey(open('a: 1'), ['a'], 'x: y'))).toBe('"x: y": 1');
  });

  it('renames nested and flow keys', () => {
    expect(text(renameKey(open(SAMPLE), ['nested', 'k'], 'kk'))).toContain('  kk: 1\n');
    expect(text(renameKey(open(SAMPLE), ['flow', 'b'], 'c'))).toContain('flow: {a: 1, c: 2}');
  });

  it('refuses a clash', () => {
    expect(() => renameKey(open('a: 1\nb: 2'), ['a'], 'b')).toThrow(/already exists/);
  });
});

describe('sliceAt / replaceSubtree', () => {
  const paths: (string | number)[][] = [['title'], ['tags'], ['desc'], ['nested'], ['nested', 'list'], ['nested', 'list', 1], ['flow'], ['quoted']];
  for (const path of paths) {
    it(`round-trips ${path.join('.')} unchanged`, () => {
      const m = open(SAMPLE);
      const slice = sliceAt(m, path);
      const after = replaceSubtree(m, path, slice);
      expect(after.text).toBe(SAMPLE);
      expect(after.normalized).toBe(false);
    });
  }

  it('dedents a nested map, comments included', () => {
    const m = open('a:\n  # about x\n  x: 1  # one\n  y: [1, 2]\nb: 2');
    expect(sliceAt(m, ['a'])).toBe('# about x\nx: 1  # one\ny: [1, 2]');
  });

  it('writes the user’s YAML, re-indented', () => {
    const m = replaceSubtree(open('a:\n  x: 1\nb: 2'), ['a'], '# c\nx: 2\nz: &k [1]\nw: *k');
    expect(text(m)).toBe('a:\n  # c\n  x: 2\n  z: &k [1]\n  w: *k\nb: 2');
    expect(getIn(m, ['a', 'w'])).toEqual([1]);
  });

  it('puts a scalar snippet inline', () => {
    expect(text(replaceSubtree(open('a:\n  x: 1\nb: 2'), ['a'], 'plain'))).toBe('a: plain\nb: 2');
  });

  it('refuses a snippet that does not parse', () => {
    expect(() => replaceSubtree(open('a: 1'), ['a'], '[unclosed')).toThrow(FrontmatterEditError);
  });

  it('edits an anchored value whose aliases follow it', () => {
    const src = 'defaults: &d\n  color: red\n  size: 2\nwidget:\n  <<: *d\n  size: 3\nother: *d\n';
    const next = replaceSubtree(open(src), ['defaults'], '&d\ncolor: red\nsize: 2\nshape: [round]');
    expect(next.text).toBe('defaults: &d\n  color: red\n  size: 2\n  shape: [round]\nwidget:\n  <<: *d\n  size: 3\nother: *d\n');
    expect(next.normalized).toBe(false);
    expect(getIn(next, ['other'])).toEqual({ color: 'red', size: 2, shape: ['round'] });
    // Dropping the anchor its aliases name is still refused.
    expect(() => replaceSubtree(open(src), ['defaults'], 'color: red')).toThrow(FrontmatterEditError);
  });

  it('edits a value whose own alias names an anchor elsewhere', () => {
    const src = 'defaults: &d\n  color: red\nwidget:\n  <<: *d\n  size: 3\n';
    const next = replaceSubtree(open(src), ['widget'], '<<: *d\nsize: 4');
    expect(next.text).toBe('defaults: &d\n  color: red\nwidget:\n  <<: *d\n  size: 4\n');
    expect(next.normalized).toBe(false);
    expect(() => replaceSubtree(open(src), ['widget'], '<<: *missing')).toThrow(FrontmatterEditError);
  });

  it('an alias with no anchor makes the model broken, not a crash', () => {
    const m = open('a: 1\nb: *nope\n');
    expect(m.errors).toEqual([{ code: 'BAD_ALIAS', message: expect.stringContaining('*nope'), line: 2, col: 4 }]);
    expect(() => setScalar(m, ['a'], 2)).toThrow(FrontmatterEditError);
  });

  it('replaces the whole block at the root', () => {
    expect(text(replaceSubtree(open('a: 1'), [], 'b: 2\n# c'))).toBe('b: 2\n# c');
  });
});

describe('nodeInfo / entries', () => {
  it('describes kinds and styles', () => {
    const m = open(SAMPLE);
    expect(nodeInfo(m, ['title'])).toMatchObject({ kind: 'scalar', scalarStyle: 'PLAIN', hasComments: true, needsYaml: false });
    expect(nodeInfo(m, ['tags'])).toMatchObject({ kind: 'seq', flow: true, needsYaml: false });
    expect(nodeInfo(m, ['nested'])).toMatchObject({ kind: 'map', flow: false, needsYaml: false });
    expect(nodeInfo(m, ['desc']).scalarStyle).toBe('BLOCK_LITERAL');
    expect(nodeInfo(m, ['nope']).kind).toBe('missing');
  });

  it('flags YAML features and comments inside collections', () => {
    const m = open('a: &x 1\nb: *x\nc: !custom v\nd:\n  # c\n  k: 1\ne:\n  <<: {k: 1}\nf:\n  k: 1  # inline');
    expect(nodeInfo(m, ['a'])).toMatchObject({ anchor: 'x', needsYaml: true });
    expect(nodeInfo(m, ['b'])).toMatchObject({ kind: 'alias', needsYaml: true });
    expect(nodeInfo(m, ['c'])).toMatchObject({ tag: '!custom', needsYaml: true });
    expect(nodeInfo(m, ['d'])).toMatchObject({ hasComments: true, needsYaml: true });
    expect(nodeInfo(m, ['e'])).toMatchObject({ hasYamlFeatures: true, needsYaml: true });
    // A same-line comment on a scalar inside is kept by every operation.
    expect(nodeInfo(m, ['f'])).toMatchObject({ hasComments: true, needsYaml: false });
  });

  it('only comments the operations cannot keep make a collection YAML-only', () => {
    const yamlOnly = (src: string, path: (string | number)[]) => nodeInfo(open(src), path).needsYaml;
    // Kept: same-line comments after scalars (also empty ones) and after a flow collection.
    expect(yamlOnly('phones:\n  mobile: "+1 555"  # cell\n  work: x\n', ['phones'])).toBe(false);
    expect(yamlOnly('tags:\n  - a  # first\n  - b # second\n', ['tags'])).toBe(false);
    expect(yamlOnly('m:\n  later: # fill in\n  k: 1\n', ['m'])).toBe(false);
    expect(yamlOnly('m:\n  tags: [a, b]  # mine\n', ['m'])).toBe(false);
    expect(yamlOnly('rel:\n  - type: parent  # mum\n    to: "[[A]]"\n', ['rel'])).toBe(false);
    // Not kept: comment lines between or after items, after `key:`, inside a flow collection, a block header.
    expect(yamlOnly('tags:\n  - a\n  # between\n  - b\n', ['tags'])).toBe(true);
    expect(yamlOnly('m:\n  l:\n    - x\n    # trailing\n  k: 1\n', ['m'])).toBe(true);
    expect(yamlOnly('tags: # mine\n  - a\n', ['tags'])).toBe(true);
    expect(yamlOnly('tags: [a, # x\n  b]\n', ['tags'])).toBe(true);
    expect(yamlOnly('m:\n  d: |  # hdr\n    text\n', ['m'])).toBe(true);
  });

  it('keeps same-line comments through edits to the scalar and its siblings', () => {
    const src = 'phones:\n  mobile: "+1 555"  # cell\n  work: x # desk\ntags:\n  - a  # first\n  - b\n';
    const keepsAll = (m: FrontmatterModel, ...comments: string[]) => comments.forEach((c) => expect(m.text).toContain(c));
    const edited = setScalar(open(src), ['phones', 'mobile'], '+1 555 0100');
    expect(edited.text).toBe('phones:\n  mobile: "+1 555 0100"  # cell\n  work: x # desk\ntags:\n  - a  # first\n  - b\n');
    keepsAll(setScalar(open(src), ['phones', 'work'], null), '# desk', '# cell');
    keepsAll(removeKey(open(src), ['phones', 'work']), '# cell', '# first');
    keepsAll(addKey(open(src), ['phones'], 'home', 'y'), '# cell', '# desk');
    keepsAll(renameKey(open(src), ['phones', 'mobile'], 'cell'), '# cell', '# desk');
    keepsAll(removeItem(open(src), ['tags'], 1), '# first', '# cell');
    keepsAll(addItem(open(src), ['tags'], 'c'), '# first');
    expect(addItem(open(src), ['tags'], 'c').text).toBe(`${src}  - c\n`);
    for (const m of [edited, removeKey(open(src), ['phones', 'work']), removeItem(open(src), ['tags'], 1)]) expect(m.normalized).toBe(false);
  });

  it('lists entries of maps and sequences', () => {
    const m = open(SAMPLE);
    expect(entries(m, []).map((e) => e.key)).toEqual(['title', 'tags', 'desc', 'nested', 'empty', 'flow', 'quoted', 'dq']);
    expect(entries(m, ['tags']).map((e) => e.value)).toEqual(['a', 'b', 'c']);
    expect(entries(m, ['nested', 'list', 1]).map((e) => e.path)).toEqual([
      ['nested', 'list', 1, 'y'],
      ['nested', 'list', 1, 'z'],
    ]);
    expect(entries(m, ['title'])).toEqual([]);
  });
});

describe('jsEqual', () => {
  it('compares deeply, with key order', () => {
    expect(jsEqual({ a: [1, { b: 2 }] }, { a: [1, { b: 2 }] })).toBe(true);
    expect(jsEqual({ a: 1, b: 2 }, { b: 2, a: 1 })).toBe(false);
    expect(jsEqual([1], [1, 2])).toBe(false);
    expect(jsEqual(null, {})).toBe(false);
  });
});

// ============================================================================
// Property test: random documents, random operations
// ============================================================================

/** mulberry32: a small seeded PRNG, so failures reproduce. */
function rng(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

type Rand = () => number;
const pick = <T,>(r: Rand, xs: readonly T[]): T => xs[Math.floor(r() * xs.length)];

const WORDS = ['alpha', 'beta', 'Gamma Ray', 'x', 'été', '42', 'true', 'a: b', '#hash', "it's", 'say "hi"', '', ' pad', 'two\nlines', '[x]', '- d', 'null', '2026-10-21', '[[Note]]'];
const KEYS = ['title', 'tags', 'type', 'k', 'with space', 'dates', 'n', 'z9'];

function randScalar(r: Rand): string | number | boolean | null {
  const roll = r();
  if (roll < 0.6) return pick(r, WORDS);
  if (roll < 0.75) return Math.floor(r() * 2000) - 500;
  if (roll < 0.85) return r() < 0.5;
  if (roll < 0.9) return 1.25;
  return null;
}

function randValue(r: Rand, depth: number): unknown {
  const roll = r();
  if (depth <= 0 || roll < 0.55) return randScalar(r);
  const n = Math.floor(r() * 4);
  if (roll < 0.78) return Array.from({ length: n }, () => randValue(r, depth - 1));
  const obj: Record<string, unknown> = {};
  for (let i = 0; i < n; i++) obj[pick(r, KEYS) + (i > 0 && r() < 0.5 ? i : '')] = randValue(r, depth - 1);
  return obj;
}

/** Emits `value` as YAML with randomized styles: quoting, flow vs block, comments, blank lines. */
function emit(r: Rand, value: unknown, indent: number, step: number, seqIndent: boolean): string {
  const pad = ' '.repeat(indent);
  const scalar = (v: unknown, flow: boolean): string => {
    if (typeof v === 'string') {
      const plainOk = !flow && v === v.trim() && v !== '' && !v.includes('\n') && jsEqual(parseSafe(`k: ${v}`)?.k, v);
      const roll = r();
      if (plainOk && roll < 0.6) return v;
      if (!v.includes('\n') && roll < 0.8) return `'${v.replace(/'/g, "''")}'`;
      return JSON.stringify(v);
    }
    if (v === null) return pick(r, ['null', '~']);
    return String(v);
  };
  const flowOf = (v: unknown): string => {
    if (Array.isArray(v)) return `[${v.map(flowOf).join(', ')}]`;
    if (v && typeof v === 'object') return `{${Object.entries(v).map(([k, x]) => `${JSON.stringify(k)}: ${flowOf(x)}`).join(', ')}}`;
    return scalar(v, true);
  };
  const comment = () => (r() < 0.15 ? `  # c${Math.floor(r() * 100)}` : '');
  const lines: string[] = [];
  const keyText = (k: string) => (/^[a-z0-9]+$/i.test(k) && r() < 0.8 ? k : JSON.stringify(k));
  const child = (v: unknown, lead: string, childIndent: number): void => {
    const empty = Array.isArray(v) ? v.length === 0 : v && typeof v === 'object' ? Object.keys(v).length === 0 : false;
    if (v && typeof v === 'object' && !empty && r() < 0.75) {
      lines.push(`${lead}${comment()}`);
      lines.push(emit(r, v, childIndent, step, seqIndent));
    } else if (v && typeof v === 'object') {
      lines.push(`${lead} ${flowOf(v)}${comment()}`);
    } else if (typeof v === 'string' && v.includes('\n') && !/^\s/.test(v) && r() < 0.5) {
      const chomp = v.endsWith('\n') ? '' : '-';
      const body = (chomp ? v : v.slice(0, -1)).split('\n').map((l) => (l ? ' '.repeat(childIndent + (Array.isArray(value) ? 0 : 0)) + l : ''));
      lines.push(`${lead} |${chomp}${comment()}`);
      lines.push(body.join('\n'));
    } else {
      lines.push(`${lead} ${scalar(v, false)}${comment()}`);
    }
  };
  if (Array.isArray(value)) {
    for (const item of value) {
      if (r() < 0.1) lines.push(`${pad}# own line`);
      child(item, `${pad}-`, indent + 2);
    }
  } else if (value && typeof value === 'object') {
    for (const [k, v] of Object.entries(value)) {
      if (r() < 0.1) lines.push('');
      if (r() < 0.1) lines.push(`${pad}# about ${k}`);
      child(v, `${pad}${keyText(k)}:`, Array.isArray(v) && !seqIndent ? indent : indent + step);
    }
  }
  return lines.join('\n');
}

function parseSafe(src: string): Record<string, unknown> | null {
  try {
    return parse(src) as Record<string, unknown>;
  } catch {
    return null;
  }
}

/** Every (path, kind) in a value, for choosing an operation's target. */
function allPaths(v: unknown, path: (string | number)[] = []): { path: (string | number)[]; v: unknown }[] {
  const out = [{ path, v }];
  if (Array.isArray(v)) v.forEach((x, i) => out.push(...allPaths(x, [...path, i])));
  else if (v && typeof v === 'object') Object.entries(v).forEach(([k, x]) => out.push(...allPaths(x, [...path, k])));
  return out;
}

type Rec = { path: (string | number)[]; src: string };

/**
 * Every scalar's source text (keys as `path + '#key'`) and every comment in
 * a model, by path. Comments are a multiset of text: which node yaml hangs a
 * comment on may change with its neighbours, its presence must not.
 */
function inventory(m: FrontmatterModel): { scalars: Rec[]; comments: { path: (string | number)[]; text: string }[] } {
  const scalars: Rec[] = [];
  const found: { path: (string | number)[]; text: string }[] = [];
  const note = (n: unknown, path: (string | number)[]) => {
    const c = n as { comment?: string | null; commentBefore?: string | null };
    for (const t of [c.commentBefore, c.comment]) if (t) for (const line of t.split('\n')) if (line.trim()) found.push({ path, text: line.trim() });
  };
  const walk = (node: unknown, path: (string | number)[]): void => {
    if (!node) return;
    note(node, path);
    if (isMap(node)) {
      for (const pair of node.items) {
        const key = isScalar(pair.key) ? String(pair.key.value) : '?';
        if (isScalar(pair.key)) {
          const r = pair.key.range!;
          scalars.push({ path: [...path, key, '#key'], src: m.text.slice(r[0], r[1]) });
          note(pair.key, [...path, key, '#key']);
        }
        walk(pair.value, [...path, key]);
      }
    } else if (isSeq(node)) {
      node.items.forEach((item, i) => walk(item, [...path, i]));
    } else if (isScalar(node)) {
      const r = node.range!;
      scalars.push({ path, src: m.text.slice(r[0], r[1]) });
    }
  };
  note(m.doc, []);
  walk(m.doc.contents, []);
  return { scalars, comments: found };
}

const startsWith = (path: readonly (string | number)[], prefix: readonly (string | number)[]) =>
  prefix.length <= path.length && prefix.every((s, i) => path[i] === s);

/** Where an old path is after the operation, `null` when it is the edited subtree. */
type PathMap = (path: (string | number)[]) => (string | number)[] | null;

function keyOf(path: (string | number)[]): string {
  return JSON.stringify(path);
}

describe('property: random documents and operations', () => {
  it('every operation means what it says and leaves the rest of the text alone', () => {
    let normalized = 0;
    let ran = 0;
    let ownKept = 0;
    for (let seed = 1; seed <= Number(process.env.FM_SEEDS ?? 1500); seed++) {
      const r = rng(seed);
      const root: Record<string, unknown> = {};
      const nKeys = 1 + Math.floor(r() * 5);
      for (let i = 0; i < nKeys; i++) root[`${pick(r, KEYS)}${i}`] = randValue(r, 3);
      const src = emit(r, root, 0, pick(r, [2, 4]), r() < 0.7) + (r() < 0.5 ? '\n' : '');
      const m = openFrontmatter(src);
      expect(m.errors, `seed ${seed}: generator emitted invalid YAML:\n${src}`).toEqual([]);
      const before = toValue(m);
      expect(before, `seed ${seed}: generator value mismatch:\n${src}`).toEqual(root);

      const targets = allPaths(before);
      const target = pick(r, targets);
      const opRoll = r();
      let result: FrontmatterModel;
      let label: string;
      const tp = target.path;
      let mapPath: PathMap = (p) => (startsWith(p, tp) ? null : p);
      const isMapV = (v: unknown) => Boolean(v) && typeof v === 'object' && !Array.isArray(v);
      try {
        if (opRoll < 0.3 && target.path.length > 0 && (target.v === null || typeof target.v !== 'object')) {
          label = `setScalar ${JSON.stringify(target.path)}`;
          const node = m.doc.getIn(target.path, true);
          // A same-line comment on the scalar itself is kept too (what lets the form edit around it).
          const own =
            isScalar(node) && node.comment && !node.commentBefore && !node.type?.startsWith('BLOCK') && /^[ \t]*#/.test(src.slice(node.range![1]))
              ? node.comment.trim()
              : null;
          result = setScalar(m, target.path, randScalar(r));
          if (own) {
            ownKept++;
            expect(result.text, `seed ${seed} ${label}: lost its own comment\n${src}\n---\n${result.text}`).toContain(own);
          }
        } else if (opRoll < 0.45 && isMapV(target.v)) {
          const key = `${pick(r, KEYS)}_new`;
          label = `addKey ${JSON.stringify(target.path)} ${key}`;
          result = addKey(m, target.path, key, randValue(r, 2) as never);
          mapPath = (p) => (keyOf(p) === keyOf(tp) ? null : p);
        } else if (opRoll < 0.6 && Array.isArray(target.v)) {
          label = `addItem ${JSON.stringify(target.path)}`;
          result = addItem(m, target.path, randValue(r, 2) as never);
          mapPath = (p) => (keyOf(p) === keyOf(tp) ? null : p);
        } else if (opRoll < 0.75 && target.path.length > 0) {
          const last = target.path[target.path.length - 1];
          label = `remove ${JSON.stringify(target.path)}`;
          result = typeof last === 'number' ? removeItem(m, target.path.slice(0, -1), last) : removeKey(m, target.path);
          const parent = tp.slice(0, -1);
          mapPath = (p) => {
            if (startsWith(p, tp)) return null;
            // An emptied nested collection is rewritten as `[]`/`{}`.
            if (parent.length > 0 && keyOf(p) === keyOf(parent)) return null;
            if (typeof last === 'number' && startsWith(p, parent) && p.length > parent.length) {
              const i = p[parent.length] as number;
              if (i > last) return [...parent, i - 1, ...p.slice(parent.length + 1)];
            }
            return p;
          };
        } else if (opRoll < 0.85 && target.path.length > 0 && typeof target.path[target.path.length - 1] === 'string') {
          label = `rename ${JSON.stringify(target.path)}`;
          const renamed = `${String(target.path[target.path.length - 1])}_renamed`;
          result = renameKey(m, target.path, renamed);
          mapPath = (p) => {
            if (keyOf(p) === keyOf([...tp, '#key'])) return null;
            return startsWith(p, tp) ? [...tp.slice(0, -1), renamed, ...p.slice(tp.length)] : p;
          };
        } else if (opRoll < 0.92 && target.path.length > 0) {
          // "Edit as YAML" without changes is a no-op; with a new value it means that value.
          const unchanged = replaceSubtree(m, target.path, sliceAt(m, target.path));
          expect(unchanged.text, `seed ${seed} identity ${JSON.stringify(target.path)}\n${src}`).toBe(src);
          const v = randValue(r, 2);
          label = `replaceSubtree ${JSON.stringify(target.path)}`;
          result = replaceSubtree(m, target.path, stringify(v));
          expect(getIn(result, target.path)).toEqual(v);
        } else if (target.path.length > 0) {
          const v = randValue(r, 2);
          label = `setValue ${JSON.stringify(target.path)}`;
          result = setValue(m, target.path, v as never);
        } else {
          continue;
        }
      } catch (e) {
        throw new Error(`seed ${seed}: threw ${(e as Error).message}\n${src}`);
      }
      ran++;
      expect(result.errors, `seed ${seed} ${label}`).toEqual([]);
      if (result.normalized) {
        normalized++;
        if (process.env.FM_DEBUG) console.log(`NORM seed ${seed} ${label}\n${src}\n===\n${result.text}\n######`);
        continue;
      }
      const edit = result.lastEdit!;
      // The splice is the whole difference...
      expect(src.slice(0, edit.start) + edit.text + src.slice(edit.end), `seed ${seed} ${label}`).toBe(result.text);
      // ...and it is local: every scalar outside the edited value keeps its exact source,
      // and every comment outside it survives.
      const was = inventory(m);
      const now = inventory(result);
      const nowScalars = new Map(now.scalars.map((x) => [keyOf(x.path), x.src]));
      const context = `seed ${seed} ${label}\n${src}\n---\n${result.text}`;
      for (const x of was.scalars) {
        const to = mapPath(x.path);
        // A block scalar's range takes in its final line break, which a removal at the end of the text may take.
        if (to) expect(nowScalars.get(keyOf(to))?.replace(/\n$/, ''), `${context}\nscalar at ${keyOf(x.path)}`).toBe(x.src.replace(/\n$/, ''));
      }
      const nowComments = now.comments.map((c) => c.text);
      for (const c of was.comments) {
        if (c.path.length > 0 && mapPath(c.path) === null) continue;
        const i = nowComments.indexOf(c.text);
        expect(i, `${context}\nlost comment ${c.text}`).toBeGreaterThanOrEqual(0);
        nowComments.splice(i, 1);
      }
    }
    if (process.env.FM_SEEDS) console.log(`ran ${ran} normalized ${normalized}`);
    expect(ran).toBeGreaterThan(1000);
    expect(ownKept).toBeGreaterThan(10);
    // The fallback is a safety net, not the normal path: over these seeds it is never needed.
    // (FM_SEEDS=100000 runs more; FM_DEBUG=1 prints any operation that fell back.)
    expect(normalized).toBe(0);
  });

  it('untouched text is identical', () => {
    for (let seed = 1; seed <= 100; seed++) {
      const r = rng(seed * 7);
      const src = emit(r, { a: randValue(r, 3), b: randValue(r, 3) }, 0, 2, true);
      expect(openFrontmatter(src).text).toBe(src);
    }
  });
});
