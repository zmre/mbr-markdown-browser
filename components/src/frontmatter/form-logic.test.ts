import { describe, expect, it } from 'vitest';
import { openFrontmatter } from './model.js';
import { builtinCatalog, resolveField, type FieldDef } from './fields.js';
import { inferCatalog } from './infer.js';
import {
  addProperty,
  existingPropertyPath,
  emptyValueFor,
  fits,
  isEmptyValue,
  parseNumberInput,
  partialDateFields,
  partialDateValue,
  pathKey,
  typeConversions,
  valueSuggestions,
  widgetKindFor,
} from './form-logic.js';

const def = (kind: FieldDef['kind'], extra: Partial<FieldDef> = {}): FieldDef => ({ key: 'k', kind, label: 'K', group: 'vault', ...extra });

describe('widgetKindFor', () => {
  it('uses the definition while its widget fits the value', () => {
    expect(widgetKindFor(def('date'), '2026-10-21')).toBe('date');
    expect(widgetKindFor(def('date'), null)).toBe('date');
    expect(widgetKindFor(def('tags'), ['a', 'b'])).toBe('tags');
    expect(widgetKindFor(def('bool'), true)).toBe('bool');
    expect(widgetKindFor(def('partialDate'), 1927)).toBe('partialDate');
    expect(widgetKindFor(def('partialDate'), '--03-19')).toBe('partialDate');
    expect(widgetKindFor(def('datetime'), '2026-06-30T14:00:00Z')).toBe('datetime');
  });

  it('falls back to what the value is when the definition does not fit', () => {
    expect(widgetKindFor(def('date'), 'next week')).toBe('text');
    expect(widgetKindFor(def('number'), 'many')).toBe('text');
    // repo.rs reads a comma-separated string as tags, so the tag widget shows it.
    expect(widgetKindFor(def('tags'), 'a, b')).toBe('tags');
    expect(widgetKindFor(def('tags'), 'two\nlines')).toBe('longtext');
    expect(widgetKindFor(def('text'), 'two\nlines')).toBe('longtext');
    expect(widgetKindFor(def('text'), 3)).toBe('number');
    expect(widgetKindFor(def('text'), { a: 1 })).toBe('tree');
    expect(widgetKindFor(def('list'), [{ a: 1 }])).toBe('tree');
    expect(widgetKindFor(def('text'), ['a'])).toBe('list');
    expect(widgetKindFor(def('partialDate'), 'circa 1900')).toBe('text');
  });

  it('keeps collection kinds for maps (a tree until a widget is registered)', () => {
    expect(widgetKindFor(def('map'), { a: 1 })).toBe('map');
    expect(widgetKindFor(def('relationships'), [{ type: 'parent' }])).toBe('relationships');
  });
});

describe('fits / isEmptyValue / emptyValueFor', () => {
  it('reads emptiness', () => {
    expect(isEmptyValue(null)).toBe(true);
    expect(isEmptyValue('  ')).toBe(true);
    expect(isEmptyValue([])).toBe(true);
    expect(isEmptyValue({})).toBe(true);
    expect(isEmptyValue(0)).toBe(false);
    expect(isEmptyValue(false)).toBe(false);
  });

  it('starts new properties empty, a checkbox unchecked', () => {
    expect(emptyValueFor('text')).toBeNull();
    expect(emptyValueFor('tags')).toBeNull();
    expect(emptyValueFor('map')).toBeNull();
    expect(emptyValueFor('bool')).toBe(false);
  });

  it('a list widget fits only scalars', () => {
    expect(fits('list', ['a', 1, true, null])).toBe(true);
    expect(fits('list', [['a']])).toBe(false);
  });
});

describe('typeConversions', () => {
  const kinds = (v: Parameters<typeof typeConversions>[0], current: Parameters<typeof typeConversions>[1]) =>
    typeConversions(v, current).map((c) => c.kind);

  it('offers anything for an empty value', () => {
    expect(kinds(null, 'text')).toEqual(expect.arrayContaining(['number', 'bool', 'date', 'list', 'tree']));
    expect(typeConversions(null, 'text').find((c) => c.kind === 'bool')?.write).toBe(false);
    expect(typeConversions(null, 'text').find((c) => c.kind === 'list')?.write).toBeUndefined();
  });

  it('offers only lossless changes for a value', () => {
    expect(kinds('hello', 'text')).toEqual(['longtext', 'list']);
    expect(kinds('42', 'text')).toContain('number');
    expect(typeConversions('42', 'text').find((c) => c.kind === 'number')?.write).toBe(42);
    expect(kinds('4 2', 'text')).not.toContain('number');
    expect(typeConversions(7, 'number')).toEqual([
      { kind: 'text', label: 'Text', write: '7' },
      { kind: 'list', label: 'List', write: [7] },
    ]);
    expect(kinds('a\nb', 'longtext')).not.toContain('text');
    expect(typeConversions(['x'], 'list').find((c) => c.kind === 'text')?.write).toBe('x');
    expect(kinds(['x', 'y'], 'list')).toEqual(['tree']);
    expect(kinds({ a: 1 }, 'tree')).toEqual([]);
  });
});

describe('addProperty', () => {
  it('adds a top-level key', () => {
    expect(addProperty(openFrontmatter('a: 1'), 'b', null).text).toBe('a: 1\nb:');
  });

  it('adds a dotted key into an existing map, or as a new map', () => {
    expect(addProperty(openFrontmatter('taxonomy:\n  genre: x'), 'taxonomy.performers', null).text).toBe(
      'taxonomy:\n  genre: x\n  performers:',
    );
    const fresh = addProperty(openFrontmatter('a: 1'), 'taxonomy.performers', null);
    expect(fresh.text).toBe('a: 1\ntaxonomy:\n  performers:');
  });

  it('refuses an existing key', () => {
    expect(() => addProperty(openFrontmatter('a: 1'), 'a', null)).toThrow(/already exists/);
  });

  it('refuses a dotted key that exists in either spelling', () => {
    // A flat `dates.birthday:` is the same property; adding it nested would duplicate it.
    const flat = openFrontmatter('type: person\ndates.birthday: 1927-03-19\n');
    expect(existingPropertyPath(flat, 'dates.birthday')).toEqual(['dates.birthday']);
    expect(() => addProperty(flat, 'dates.birthday', null)).toThrow(/already exists/);
    const nested = openFrontmatter('dates:\n  birthday: 1927-03-19\n');
    expect(existingPropertyPath(nested, 'dates.birthday')).toEqual(['dates', 'birthday']);
    expect(() => addProperty(nested, 'dates.birthday', null)).toThrow(/already exists/);
    expect(existingPropertyPath(nested, 'dates.death')).toBeUndefined();
  });
});

describe('number and partial-date input', () => {
  it('parses numbers, cleared, and not yet', () => {
    expect(parseNumberInput('3.5')).toBe(3.5);
    expect(parseNumberInput('')).toBeNull();
    expect(parseNumberInput('-')).toBeUndefined();
  });

  it('round-trips every partial-date shape', () => {
    for (const stored of ['1927-03-19', '1927-03', '--03-19', '03-19', 1927, '1927']) {
      const fields = partialDateFields(stored);
      const back = partialDateValue(fields, stored);
      expect(back).toEqual({ ok: true, value: stored });
    }
  });

  it('writes what the fields say and refuses impossible dates', () => {
    expect(partialDateValue({ year: '1950', month: '', day: '' }, null)).toEqual({ ok: true, value: '1950' });
    expect(partialDateValue({ year: '', month: '03', day: '19' }, null)).toEqual({ ok: true, value: '--03-19' });
    expect(partialDateValue({ year: '', month: '', day: '' }, '1950')).toEqual({ ok: true, value: null });
    expect(partialDateValue({ year: '1950', month: '', day: '04' }, null)).toEqual({ ok: false, message: 'A day needs a month.' });
    expect(partialDateValue({ year: '19', month: '', day: '' }, null).ok).toBe(false);
    expect(partialDateValue({ year: '', month: '02', day: '30' }, null).ok).toBe(false);
  });
});

describe('valueSuggestions', () => {
  it('lists known values with help first, then the vault’s by use', () => {
    const catalog = builtinCatalog();
    const inferred = inferCatalog([
      { frontmatter: { type: 'Metric' } },
      { frontmatter: { type: 'Metric' } },
      { frontmatter: { type: 'person' } },
    ]);
    const typeDef = resolveField(['type'], {}, catalog, inferred);
    const out = valueSuggestions(['type'], typeDef, inferred);
    expect(out.slice(0, 4).map((o) => o.value)).toEqual(['slides', 'flashcard', 'person', 'organization']);
    expect(out[0].detail).toMatch(/slideshow/);
    expect(out.find((o) => o.value === 'Metric')).toEqual({ value: 'Metric', detail: '2 notes' });
    expect(out.filter((o) => o.value === 'person')).toHaveLength(1);
  });

  it('keys list items under their list', () => {
    const inferred = inferCatalog([{ frontmatter: { tags: ['a', 'b'] } }, { frontmatter: { tags: ['a'] } }]);
    expect(valueSuggestions(['tags', 0], def('text'), inferred).map((o) => o.value)).toEqual(['a', 'b']);
  });
});

describe('pathKey', () => {
  it('tells numbers and strings apart', () => {
    expect(pathKey(['a', 0])).not.toBe(pathKey(['a', '0']));
  });
});
