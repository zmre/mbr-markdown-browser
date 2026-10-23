import { describe, expect, it } from 'vitest';
import { inferCatalog, isActor, vaultActors, vaultTypes } from './infer.js';

const FILES = [
  { frontmatter: { title: 'One', type: 'person', tags: ['a', 'b'], rating: 3, draft: true, 'dates.birthday': '1950-01-02' } },
  { frontmatter: { title: 'Two', type: 'person', tags: ['a'], rating: 4, 'generated.by': 'human:pat' } },
  { frontmatter: { title: 'Three', type: 'Metric', tags: [], rating: 'n/a', generated: { by: 'agent/1.2', at: '2026-01-01T00:00:00Z' } } },
  { frontmatter: { title: 'Four', summary: 'x'.repeat(200), owner: 'process:nightly' } },
  { frontmatter: null },
  {},
];

describe('inferCatalog', () => {
  const inferred = inferCatalog(FILES);

  it('counts keys, most used first', () => {
    expect([...inferred.keys()].slice(0, 3)).toEqual(['title', 'rating', 'tags']);
    expect(inferred.get('title')!.count).toBe(4);
    expect(inferred.get('type')!.count).toBe(3);
  });

  it('takes the dominant kind', () => {
    expect(inferred.get('rating')).toMatchObject({ kind: 'number', kinds: { number: 2, text: 1 } });
    expect(inferred.get('draft')!.kind).toBe('bool');
    expect(inferred.get('tags')!.kind).toBe('tags');
    expect(inferred.get('dates.birthday')!.kind).toBe('date');
    expect(inferred.get('generated.at')!.kind).toBe('datetime');
  });

  it('counts list items one by one and skips long values', () => {
    expect(inferred.get('tags')!.values).toEqual([
      { value: 'a', count: 2 },
      { value: 'b', count: 1 },
    ]);
    expect(inferred.get('summary')!.values).toEqual([]);
    expect(inferred.get('rating')!.values.map((v) => v.value)).toEqual(['3', '4', 'n/a']);
  });

  it('walks nested objects into dotted keys, like site.json’s flattened ones', () => {
    expect(inferred.get('generated')!.kind).toBe('map');
    expect(inferred.get('generated.by')!.values.map((v) => v.value)).toEqual(['agent/1.2', 'human:pat']);
  });

  it('caps values per key and treats configured tag fields as tags', () => {
    const many = Array.from({ length: 30 }, (_, i) => ({ frontmatter: { k: `v${i}`, cast: ['x'] } }));
    const out = inferCatalog(many, { topValues: 5, tagFields: ['cast'] });
    expect(out.get('k')!.values).toHaveLength(5);
    expect(out.get('cast')!.kind).toBe('tags');
  });

  it('is empty for no files', () => {
    expect(inferCatalog([]).size).toBe(0);
  });
});

describe('vault helpers', () => {
  const inferred = inferCatalog(FILES);

  it('lists the vault’s types', () => {
    expect(vaultTypes(inferred)).toEqual([
      { value: 'person', count: 2 },
      { value: 'Metric', count: 1 },
    ]);
  });

  it('collects actors from actor keys and human:/process: values anywhere', () => {
    expect(vaultActors(inferred).map((a) => a.value)).toEqual(['agent/1.2', 'human:pat', 'process:nightly']);
  });

  it('recognizes OKF actor spellings', () => {
    expect(isActor('human:ahormati')).toBe(true);
    expect(isActor('process:finance-nightly')).toBe(true);
    expect(isActor('example_agent/gemini-2.5-pro')).toBe(true);
    expect(isActor('just text')).toBe(false);
    expect(isActor('https://x/y')).toBe(false);
  });
});
