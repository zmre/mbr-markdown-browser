import { describe, expect, it } from 'vitest';
import { yamlLanguage } from '@codemirror/lang-yaml';
import { keyText, yamlContextAt, type YamlContext } from './yaml-context.js';

/** The context at the last `|` in `src` (the marker is removed; an earlier `|` is YAML). */
function at(src: string): YamlContext | null {
  const pos = src.lastIndexOf('|');
  if (pos < 0) throw new Error('no cursor marker');
  const text = src.slice(0, pos) + src.slice(pos + 1);
  return yamlContextAt(text, yamlLanguage.parser.parse(text), pos);
}

/** `from`/`to` shown as the text they span. */
function span(src: string): { prefix: string; replaced: string } | null {
  const ctx = at(src);
  if (!ctx) return null;
  const pos = src.lastIndexOf('|');
  const text = src.slice(0, pos) + src.slice(pos + 1);
  return { prefix: ctx.prefix, replaced: text.slice(ctx.from, ctx.to) };
}

describe('yamlContextAt: block maps', () => {
  it('reads a key being typed at the root', () => {
    expect(at('|')).toMatchObject({ role: 'key', path: [], prefix: '' });
    expect(at('sta|')).toMatchObject({ role: 'key', path: [], prefix: 'sta', hasColon: false, column: 0 });
    expect(at('title: x\nsta|')).toMatchObject({ role: 'key', path: [], prefix: 'sta' });
    expect(at('title: x\n\n|')).toMatchObject({ role: 'key', path: [], prefix: '' });
    expect(at('title: x\n# a comment\nty|')).toMatchObject({ role: 'key', path: [] });
  });

  it('replaces the rest of an existing key and knows a colon follows', () => {
    expect(at('sta|tus: draft\n')).toMatchObject({ role: 'key', hasColon: true });
    expect(span('sta|tus: draft\n')).toEqual({ prefix: 'sta', replaced: 'status' });
  });

  it('reads a value after `key: `', () => {
    expect(at('status: |')).toMatchObject({ role: 'value', path: ['status'], prefix: '', flow: false });
    expect(at('title: x\nstatus: dr|')).toMatchObject({ role: 'value', path: ['status'], prefix: 'dr' });
    expect(span('status: dr|aft  # note\n')).toEqual({ prefix: 'dr', replaced: 'draft' });
    expect(at('type: Attested Comp|')).toMatchObject({ role: 'value', path: ['type'], prefix: 'Attested Comp' });
  });

  it('keeps a value with colons inside it whole', () => {
    expect(at('generated:\n  by: human:p|')).toMatchObject({ role: 'value', path: ['generated', 'by'], prefix: 'human:p' });
  });

  it('reads inside quotes', () => {
    expect(at('title: "Hel|lo"\n')).toMatchObject({ role: 'value', path: ['title'], quote: '"', prefix: 'Hel' });
    expect(span('title: "Hel|lo"\n')).toEqual({ prefix: 'Hel', replaced: 'Hello' });
    expect(at("title: 'it''s |'\n")).toMatchObject({ quote: "'", path: ['title'] });
    expect(at('title: "done" |')).toBeNull();
  });

  it('nothing right after a colon, in comments, in block scalars or after anchors', () => {
    expect(at('status:|')).toBeNull();
    expect(at('status: draft # dr|')).toBeNull();
    expect(at('# ty|')).toBeNull();
    expect(at('description: |\n  some te|xt\n')).toBeNull();
    expect(at('a: &x |')).toBeNull();
    expect(at('a: *|')).toBeNull();
  });

  it('nested keys follow indentation', () => {
    expect(at('generated:\n  |')).toMatchObject({ role: 'key', path: ['generated'], column: 2 });
    expect(at('generated:\n  a|')).toMatchObject({ role: 'key', path: ['generated'], prefix: 'a' });
    expect(at('generated:\n  by: human:me\n  a|')).toMatchObject({ role: 'key', path: ['generated'] });
    expect(at('generated:\n  by: human:me\n|')).toMatchObject({ role: 'key', path: [] });
    expect(at('a:\n  b:\n    c: 1\n    |')).toMatchObject({ role: 'key', path: ['a', 'b'] });
    expect(at('a:\n  b:\n    c: 1\n  |')).toMatchObject({ role: 'key', path: ['a'] });
    expect(at('a:\n  b:\n    c: 1\n  d: |')).toMatchObject({ role: 'value', path: ['a', 'd'] });
    expect(at('a:\n  b:\n    c: 1\nd: |')).toMatchObject({ role: 'value', path: ['d'] });
  });

  it('skips blank and comment lines between a key and its children', () => {
    expect(at('m:\n  x: 1\n\n  # note\n  y|')).toMatchObject({ role: 'key', path: ['m'] });
  });

  it('refuses an indentation nothing above it opens', () => {
    expect(at('title: x\n    |')).toBeNull();
  });

  it('continues after a block scalar', () => {
    expect(at('description: |\n  text\n  more\nsta|')).toMatchObject({ role: 'key', path: [] });
  });

  it('unquotes keys on the path', () => {
    expect(at('"my key":\n  |')).toMatchObject({ path: ['my key'] });
    expect(at("'it''s':\n  x: |")).toMatchObject({ path: ["it's", 'x'] });
    expect(keyText('"a\\tb"')).toBe('a\tb');
    expect(keyText("'a''b'")).toBe("a'b");
    expect(keyText('plain')).toBe('plain');
  });
});

describe('yamlContextAt: block sequences', () => {
  it('numbers items, indented or flush under their key', () => {
    expect(at('tags:\n  - a\n  - |')).toMatchObject({ role: 'item', path: ['tags', 1] });
    expect(at('tags:\n- a\n- b\n- c|')).toMatchObject({ role: 'item', path: ['tags', 2], prefix: 'c' });
    expect(at('tags:\n  - |')).toMatchObject({ role: 'item', path: ['tags', 0] });
    expect(at('tags:\n- |')).toMatchObject({ role: 'item', path: ['tags', 0] });
    expect(at('tags:\n  - a\n  - b\nnext: |')).toMatchObject({ role: 'value', path: ['next'] });
  });

  it('a `- ` item that is a map: its first key, later keys, a new item', () => {
    const rel = 'relationships:\n  - type: parent\n    to: "[[A]]"\n';
    expect(at('relationships:\n  - type: pa|')).toMatchObject({ role: 'value', path: ['relationships', 0, 'type'], prefix: 'pa' });
    expect(at('relationships:\n  - type: parent\n    |')).toMatchObject({ role: 'key', path: ['relationships', 0], column: 4 });
    expect(at('relationships:\n  - type: parent\n    la|')).toMatchObject({ role: 'key', path: ['relationships', 0], prefix: 'la' });
    expect(at(`${rel}  - |`)).toMatchObject({ role: 'item', path: ['relationships', 1] });
    expect(at(`${rel}  - type: |`)).toMatchObject({ role: 'value', path: ['relationships', 1, 'type'] });
    expect(at(`${rel}  - type: x\n    label: |`)).toMatchObject({ role: 'value', path: ['relationships', 1, 'label'] });
    expect(at('sources:\n- resource: x\n  |')).toMatchObject({ role: 'key', path: ['sources', 0] });
  });

  it('nested sequences', () => {
    expect(at('a:\n  - - x\n    - |')).toMatchObject({ role: 'item', path: ['a', 0, 1] });
    expect(at('a:\n  - - |')).toMatchObject({ role: 'item', path: ['a', 0, 0] });
  });

  it('a `-` with no space yet is nothing', () => {
    expect(at('tags:\n  -|')).toBeNull();
  });

  it('labeled entries: a list of one-key maps', () => {
    expect(at('phones:\n  - mobile: "+1"\n  - wo|')).toMatchObject({ role: 'item', path: ['phones', 1], prefix: 'wo' });
    expect(at('phones:\n  - mobile: "+1"\n  - label: |')).toMatchObject({ role: 'value', path: ['phones', 1, 'label'] });
  });
});

describe('yamlContextAt: flow collections', () => {
  it('numbers flow sequence items by commas, closed or not', () => {
    expect(at('tags: [|]')).toMatchObject({ role: 'value', path: ['tags', 0], flow: true, prefix: '' });
    expect(at('tags: [a, b|]')).toMatchObject({ role: 'value', path: ['tags', 1], prefix: 'b' });
    expect(at('tags: [a, b|')).toMatchObject({ role: 'value', path: ['tags', 1], prefix: 'b' });
    expect(at('tags: [a, |')).toMatchObject({ role: 'value', path: ['tags', 1], prefix: '' });
    expect(span('tags: [a, b|ee, c]')).toEqual({ prefix: 'b', replaced: 'bee' });
    expect(at('tags: [a, b]|')).toBeNull();
    expect(at('tags: [a, b]\nx|')).toMatchObject({ role: 'key', path: [] });
  });

  it('flow maps: keys and values', () => {
    expect(at('g: {|}')).toMatchObject({ role: 'key', path: ['g'], flow: true });
    expect(at('g: {by: human:me, a|}')).toMatchObject({ role: 'key', path: ['g'], prefix: 'a' });
    expect(at('g: {by: hu|}')).toMatchObject({ role: 'value', path: ['g', 'by'], prefix: 'hu' });
    expect(at('g: {by: x, at: |')).toMatchObject({ role: 'value', path: ['g', 'at'] });
  });

  it('nests flow inside block and flow inside flow', () => {
    expect(at('rel:\n  - {type: pa|}')).toMatchObject({ role: 'value', path: ['rel', 0, 'type'] });
    expect(at('x: {a: [1, 2, |]}')).toMatchObject({ role: 'value', path: ['x', 'a', 2] });
    expect(at('x: [{a: 1}, {b: |}]')).toMatchObject({ role: 'value', path: ['x', 1, 'b'] });
    expect(at('x: [[1, 2], [3, |]]')).toMatchObject({ role: 'value', path: ['x', 1, 1] });
    expect(at('rel: [{type: parent}, {t|')).toMatchObject({ role: 'key', path: ['rel', 1], prefix: 't' });
  });

  it('a flow collection spanning lines', () => {
    expect(at('tags: [\n  a,\n  b|\n]')).toMatchObject({ role: 'value', path: ['tags', 1], prefix: 'b' });
  });

  it('quoted flow items', () => {
    expect(at('tags: ["a b", "c|"]')).toMatchObject({ path: ['tags', 1], quote: '"', prefix: 'c' });
  });
});

describe('yamlContextAt: wikilinks', () => {
  it('an unquoted `[[` at the start of a value is quoted on insert', () => {
    const ctx = at('company: [[Ac|')!;
    expect(ctx).toMatchObject({ role: 'value', path: ['company'], from: 9, to: 13, quote: null });
    expect(ctx.wikilink).toMatchObject({ from: 9, query: 'Ac', open: '"', close: '"', quote: '"' });
  });

  it('inside quotes the link goes in bare, closing the quote only when it is open', () => {
    expect(at('to: "[[Al|')!.wikilink).toMatchObject({ query: 'Al', open: '', close: '"', quote: '"' });
    expect(at('to: "[[Al|"')!.wikilink).toMatchObject({ open: '', close: '' });
    expect(at("to: '[[Al|'")!.wikilink).toMatchObject({ open: '', close: '', quote: "'" });
  });

  it('replaces through an existing `]]`', () => {
    expect(span('to: "[[Al|ice]]"\n')).toEqual({ prefix: '[[Al', replaced: '[[Alice]]' });
  });

  it('finds the structure where the link starts', () => {
    expect(at('relationships:\n  - type: parent\n    to: [[Al|')).toMatchObject({ role: 'value', path: ['relationships', 0, 'to'] });
    expect(at('aliases:\n  - [[B|')).toMatchObject({ role: 'item', path: ['aliases', 0] });
    expect(at('aliases: [Bob, [[B|')).toMatchObject({ role: 'value', path: ['aliases', 1], flow: true });
    expect(at('rel: [{type: parent, to: "[[A|')).toMatchObject({ path: ['rel', 0, 'to'], quote: '"' });
  });

  it('inside a quoted sentence the link goes in bare', () => {
    expect(at('summary: "met [[Bo|')!.wikilink).toMatchObject({ open: '', close: '"', query: 'Bo' });
    expect(at('summary: met [[Bo|')!.wikilink).toMatchObject({ open: '', close: '' });
  });

  it('not after the link is closed, not in a key, not mid-way through an unquoted flow value', () => {
    expect(at('to: "[[Alice]]|"')?.wikilink ?? null).toBeNull();
    expect(at('[[x|')?.wikilink ?? null).toBeNull();
    expect(at('tags: [a [[b|')?.wikilink ?? null).toBeNull();
  });
});
