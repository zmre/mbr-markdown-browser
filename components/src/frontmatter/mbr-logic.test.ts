import { describe, expect, it } from 'vitest';
import { getIn, openFrontmatter, toValue, type FmValue } from './model.js';
import { builtinCatalog } from './fields.js';
import { inferCatalog } from './infer.js';
import { addToSingleOrList, itemSummary, newValueFor } from './form-logic.js';
import {
  DEFAULT_RELATIONSHIP_TYPES,
  actorOptions,
  addLabeledEntry,
  addTag,
  createActorMemory,
  edgeDirection,
  edgeOf,
  freshLabel,
  humanActorId,
  labeledEntries,
  labeledShape,
  noteLinkOptions,
  relationshipSentence,
  removeEntry,
  removeTag,
  setEdgeField,
  setEntryLabel,
  tagItems,
  tagSuggestions,
  toggleEdgeDirection,
  type EdgeView,
} from './mbr-logic.js';
import { buildLinkTargets, wikilinkName } from '../link-targets.js';

const edge = (item: FmValue): EdgeView => edgeOf(item)!;
const reg = DEFAULT_RELATIONSHIP_TYPES;

describe('relationships', () => {
  it('reads an edge and its direction', () => {
    const e = edge({ type: 'spouse', to: '[[Mary]]', married: 1948, place: 'Denver' });
    expect(e).toMatchObject({ type: 'spouse', to: '[[Mary]]', hasTo: true, hasFrom: false, attributes: ['married', 'place'] });
    expect(edgeDirection(e)).toBe('to');
    expect(edgeDirection(edge({ type: 'child', from: '[[Sam]]' }))).toBe('from');
    expect(edgeDirection(edge({ type: 'parent', from: '[[A]]', to: '[[B]]' }))).toBe('both');
    expect(edgeOf('just a string')).toBeNull();
  });

  it('phrases an edge the way relationships.rs reads it, with the derived reverse', () => {
    expect(relationshipSentence(edge({ type: 'parent', to: '[[Sam]]' }), reg, 'Jane')).toBe('Sam is Jane’s parent · Jane is Sam’s child');
    expect(relationshipSentence(edge({ type: 'parent', from: '[[Sam]]' }), reg, 'Jane')).toBe('Jane is Sam’s parent · Sam is Jane’s child');
    expect(relationshipSentence(edge({ type: 'reports_to', to: '[[Boss]]' }), reg, 'Jane')).toBe('Jane reports to Boss · Boss manages Jane');
    expect(relationshipSentence(edge({ type: 'spouse', to: '[[Mary]]' }), reg, 'this note')).toBe('This note and Mary are spouses');
    expect(relationshipSentence(edge({ type: 'mentor', to: '[[Yoda]]' }), reg, 'this note')).toBe('Yoda is this note’s mentor');
    expect(relationshipSentence(edge({ type: 'mentor', to: '[[Yoda]]', label: 'Teacher' }), reg, 'Luke')).toBe('Yoda is Luke’s teacher');
    expect(relationshipSentence(edge({ type: 'parent', to: null }), reg, 'Jane')).toBeNull();
    expect(relationshipSentence(edge({ type: null, to: '[[X]]' }), reg, 'Jane')).toBeNull();
  });

  it('toggles direction by renaming only the key token', () => {
    const text = 'relationships:\n  - type: spouse   # wed 1948\n    to: "[[Mary]]"\n';
    const m = openFrontmatter(text);
    const e = edge(getIn(m, ['relationships', 0])!);
    const next = toggleEdgeDirection(m, ['relationships', 0], e);
    expect(next.text).toBe('relationships:\n  - type: spouse   # wed 1948\n    from: "[[Mary]]"\n');
    const back = toggleEdgeDirection(next, ['relationships', 0], edge(getIn(next, ['relationships', 0])!));
    expect(back.text).toBe(text);
  });

  it('writes an endpoint as a quoted wikilink', () => {
    const m = openFrontmatter('relationships:\n  - type: parent\n    to:\n');
    expect(setEdgeField(m, ['relationships', 0], 'to', '[[Sam Doe]]').text).toBe('relationships:\n  - type: parent\n    to: "[[Sam Doe]]"\n');
  });
});

describe('labeled values', () => {
  it('reads all three shapes as contact.rs does', () => {
    expect(labeledEntries({ mobile: '+1 555', work: '+1 556' }, ['phones'], 'phones').map((e) => [e!.form, e!.label, e!.valuePath])).toEqual([
      ['key', 'mobile', ['phones', 'mobile']],
      ['key', 'work', ['phones', 'work']],
    ]);
    const list = labeledEntries(['a@x', { work: 'b@x' }, { label: 'home', value: 'c@x' }, { a: 1, b: 2 }, ['nested']], ['emails'], 'emails');
    expect(list.map((e) => e && [e.form, e.label, e.value])).toEqual([['bare', '', 'a@x'], ['pair', 'work', 'b@x'], ['labelValue', 'home', 'c@x'], null, null]);
    expect(labeledEntries('solo@x', ['emails'], 'emails')).toEqual([{ form: 'bare', label: '', value: 'solo@x', valuePath: ['emails'] }]);
    expect(labeledEntries({ label: 'work', name: 'Bob' }, ['aliases'], 'aliases')[0]).toMatchObject({ form: 'labelValue', label: 'work', value: 'Bob' });
    const addr = labeledEntries([{ label: 'home', city: 'Paris' }], ['addresses'], 'addresses')[0]!;
    expect(addr).toMatchObject({ form: 'address', label: 'home', valuePath: ['addresses', 0] });
    expect(labeledEntries({ city: 'Paris' }, ['addresses'], 'addresses')[0]).toMatchObject({ form: 'bare', valuePath: ['addresses'] });
    expect(labeledShape(null)).toBe('empty');
  });

  it('adds new entries in the shape the note already uses', () => {
    const map = openFrontmatter('phones:\n  mobile: "+1 555"\n');
    expect(addLabeledEntry(map, ['phones'], getIn(map, ['phones']), 'phones', 'work', '+1556').text).toBe('phones:\n  mobile: "+1 555"\n  work: "+1556"\n');

    const pairs = openFrontmatter('emails:\n  - work: a@x\n');
    expect(addLabeledEntry(pairs, ['emails'], getIn(pairs, ['emails']), 'emails', 'home', 'b@x').text).toBe('emails:\n  - work: a@x\n  - home: b@x\n');

    const bare = openFrontmatter('aliases:\n  - Bob\n');
    expect(addLabeledEntry(bare, ['aliases'], getIn(bare, ['aliases']), 'aliases', 'nickname', 'Rob').text).toBe('aliases:\n  - Bob\n  - Rob\n');

    const lv = openFrontmatter('emails:\n  - label: work\n    value: a@x\n');
    expect(toValue(addLabeledEntry(lv, ['emails'], getIn(lv, ['emails']), 'emails', 'home', 'b@x'))).toEqual({
      emails: [{ label: 'work', value: 'a@x' }, { label: 'home', value: 'b@x' }],
    });

    const empty = openFrontmatter('type: person\n');
    expect(addLabeledEntry(empty, ['phones'], undefined, 'phones', 'mobile', null).text).toBe('type: person\nphones:\n  mobile:\n');

    const single = openFrontmatter('emails: a@x\n');
    expect(toValue(addLabeledEntry(single, ['emails'], 'a@x', 'emails', 'work', 'b@x'))).toEqual({ emails: ['a@x', { work: 'b@x' }] });
  });

  it('keeps a lone address\'s comments when a second one turns it into a list', () => {
    const one = openFrontmatter('addresses:\n  street: 1 Main St  # front door\n  city: "Paris"\ntype: person\n');
    const next = addLabeledEntry(one, ['addresses'], getIn(one, ['addresses']), 'addresses', 'work', { city: null });
    expect(next.text).toBe('addresses:\n  - street: 1 Main St  # front door\n    city: "Paris"\n  - city:\ntype: person\n');
    expect(next.normalized).toBe(false);
  });

  it('relabels only the label', () => {
    const map = openFrontmatter('phones:\n  mobile: "+1 555" # cell\n  work: x\n');
    const entry = labeledEntries(getIn(map, ['phones']), ['phones'], 'phones')[0]!;
    expect(setEntryLabel(map, ['phones'], entry, 'home').text).toBe('phones:\n  home: "+1 555" # cell\n  work: x\n');

    const list = openFrontmatter('aliases:\n  - Bob\n  - nickname: Rob\n');
    const [bareEntry, pairEntry] = labeledEntries(getIn(list, ['aliases']), ['aliases'], 'aliases');
    expect(toValue(setEntryLabel(list, ['aliases'], bareEntry!, 'birth_name'))).toEqual({ aliases: [{ birth_name: 'Bob' }, { nickname: 'Rob' }] });
    expect(setEntryLabel(list, ['aliases'], pairEntry!, 'formerly').text).toBe('aliases:\n  - Bob\n  - formerly: Rob\n');
    expect(toValue(setEntryLabel(list, ['aliases'], pairEntry!, ''))).toEqual({ aliases: ['Bob', 'Rob'] });
  });

  it('removes an entry', () => {
    const map = openFrontmatter('phones:\n  mobile: a\n  work: b\n');
    const e = labeledEntries(getIn(map, ['phones']), ['phones'], 'phones')[0]!;
    expect(removeEntry(map, ['phones'], e).text).toBe('phones:\n  work: b\n');
  });

  it('picks a fresh label', () => {
    expect(freshLabel(['home'], ['home', 'work'])).toBe('work');
    expect(freshLabel(['Home', 'work'], ['home', 'work'])).toBe('other2');
  });
});

describe('tags', () => {
  it('reads a list or a comma string', () => {
    expect(tagItems(['a', 'b', ''])).toEqual(['a', 'b']);
    expect(tagItems('rust, web dev ,')).toEqual(['rust', 'web dev']);
    expect(tagItems(null)).toEqual([]);
  });

  it('keeps a comma string a string', () => {
    const m = openFrontmatter('tags: a, b # mine\n');
    const added = addTag(m, ['tags'], 'a, b', 'c');
    expect(added.text).toBe('tags: a, b, c # mine\n');
    expect(removeTag(added, ['tags'], 'a, b, c', 0).text).toBe('tags: b, c # mine\n');
    const tight = openFrontmatter('tags: "a,b"\n');
    expect(addTag(tight, ['tags'], 'a,b', 'c').text).toBe('tags: "a,b,c"\n');
  });

  it('keeps a list a list', () => {
    const m = openFrontmatter('tags: [a, b]\n');
    expect(addTag(m, ['tags'], ['a', 'b'], 'c').text).toBe('tags: [a, b, c]\n');
    expect(removeTag(m, ['tags'], ['a', 'b'], 1).text).toBe('tags: [a]\n');
    expect(addTag(openFrontmatter('tags:\n'), ['tags'], null, 'x').text).toBe('tags:\n  - x\n');
  });

  it('suggests the field’s tags first, then other tag sources’ (dotted ones too)', () => {
    const catalog = builtinCatalog({ tagSources: [{ field: 'tags' }, { field: 'taxonomy.performers' }] });
    const inferred = inferCatalog(
      [
        { frontmatter: { tags: ['rust', 'web'] } },
        { frontmatter: { tags: 'rust, cli' } },
        { frontmatter: { 'taxonomy.performers': ['Miles'] } },
        { frontmatter: { taxonomy: { performers: ['Miles', 'Coltrane'] } } },
      ],
      { tagFields: ['tags', 'taxonomy.performers'] },
    );
    expect(tagSuggestions('tags', catalog, inferred, ['web']).map((o) => o.value)).toEqual(['rust', 'cli', 'Miles', 'Coltrane']);
    expect(tagSuggestions('taxonomy.performers', catalog, inferred).map((o) => o.value).slice(0, 2)).toEqual(['Miles', 'Coltrane']);
  });
});

describe('note links and actors', () => {
  it('builds one target per note, shared with the [[ autocomplete', () => {
    const files = [
      { url_path: '/people/jane/', frontmatter: { title: 'Jane Doe' } },
      { url_path: '/work/jane', frontmatter: { title: 'Jane Doe' } },
      { url_path: '/notes/untitled/' },
    ];
    expect(buildLinkTargets(files).map((t) => [t.wiki, t.path])).toEqual([
      ['Jane Doe', '/people/jane/'],
      ['Jane Doe', '/work/jane/'],
      ['untitled', '/notes/untitled/'],
    ]);
    expect(noteLinkOptions(files).map((o) => [o.value, o.detail])).toEqual([
      ['[[Jane Doe]]', '/people/jane/'],
      ['[[Jane Doe]]', '/work/jane/'],
      ['[[untitled]]', '/notes/untitled/'],
    ]);
    expect(wikilinkName('[[Target|Alias]]')).toBe('Target');
    expect(wikilinkName('plain')).toBeNull();
  });

  it('offers me, the vault’s actors, then the prefixes', () => {
    const inferred = inferCatalog([{ frontmatter: { 'generated.by': 'claude/4' } }, { frontmatter: { 'generated.by': 'human:pat' } }]);
    const memory = createActorMemory();
    expect(actorOptions(inferred, memory).map((o) => o.value)).toEqual(['claude/4', 'human:pat', 'human:', 'process:']);
    memory.id = 'sam';
    expect(actorOptions(inferred, memory)[0]).toEqual({ value: 'human:sam', detail: 'You' });
    expect(humanActorId('human:pat')).toBe('pat');
    expect(humanActorId('process:x')).toBeNull();
  });
});

describe('OKF groups', () => {
  const catalog = builtinCatalog();
  const def = (key: string) => catalog.fields.find((f) => f.key === key)!;

  it('starts new groups with their required parts', () => {
    const now = new Date(2026, 9, 21, 14, 5, 9);
    expect(newValueFor(def('sources').item, now)).toEqual({ resource: null });
    const review = newValueFor(def('verified').item, now) as Record<string, FmValue>;
    expect(review.by).toBeNull();
    expect(review.at).toMatch(/^2026-10-21T14:05:09(Z|[+-]\d{2}:\d{2})$/);
    expect(newValueFor(def('generated'), now)).toMatchObject({ by: null });
    expect(newValueFor(def('title'))).toBeNull();
  });

  it('summarizes a group item by its required part', () => {
    expect(itemSummary({ id: 's1', resource: 'https://x.test' }, def('sources').item)).toBe('https://x.test');
    expect(itemSummary({ by: 'human:pat', at: '2026-01-01T00:00:00Z' }, def('verified').item)).toBe('human:pat');
  });

  it('turns a lone verified map into a list when a second review is added, keeping its bytes', () => {
    const text = 'title: T\nverified:\n  by: human:pat   # me\n  at: 2026-01-02T03:04:05Z\nstatus: stable\n';
    const next = addToSingleOrList(openFrontmatter(text), ['verified'], { by: 'human:sam', at: null });
    expect(next.text).toBe(
      'title: T\nverified:\n  - by: human:pat   # me\n    at: 2026-01-02T03:04:05Z\n  - by: human:sam\n    at:\nstatus: stable\n',
    );
    expect(next.normalized).toBe(false);
    const list = addToSingleOrList(next, ['verified'], { by: 'x', at: null });
    expect((getIn(list, ['verified']) as unknown[]).length).toBe(3);
  });
});
