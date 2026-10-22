import { describe, expect, it } from 'vitest';
import type { RelationTypeConfig } from '../graph/relationship-graph.js';
import {
  addableFields,
  applyDatetimeLocal,
  ATTESTED_COMPUTATION_TYPE,
  builtinCatalog,
  datetimeLocalValue,
  displayPartialDate,
  fieldFor,
  formatOffset,
  formatPartialDate,
  hasUtcOffset,
  humanizeKey,
  isContactType,
  kindOfValue,
  parseDateTime,
  parsePartialDate,
  resolveField,
  validate,
} from './fields.js';
import { inferCatalog } from './infer.js';

const REGISTRY: RelationTypeConfig[] = [
  { name: 'parent', symmetric: false, inverse: 'child', label: 'Parent', label_plural: 'Parents', hierarchy: 'up', category: 'family' },
  { name: 'child', symmetric: false, inverse: 'parent', label: 'Child', label_plural: 'Children', hierarchy: 'down', category: 'family' },
  { name: 'spouse', symmetric: true, inverse: null, label: 'Spouse', label_plural: 'Spouses', category: 'family' },
];

const catalog = builtinCatalog({ relationshipTypes: REGISTRY });

describe('builtinCatalog', () => {
  it('offers type with mbr’s kinds and never a style helper', () => {
    const type = fieldFor(['type'], {}, catalog)!;
    expect(type.kind).toBe('enum');
    expect(type.enumValues!.map((e) => e.value)).toEqual(['slides', 'flashcard', 'person', 'organization']);
    expect(type.enumValues!.every((e) => e.help)).toBe(true);
    expect(fieldFor(['style'], {}, catalog)).toBeUndefined();
  });

  it('uses the configured tag sources, defaulting to tags', () => {
    expect(fieldFor(['tags'], {}, catalog)?.kind).toBe('tags');
    const custom = builtinCatalog({ tagSources: [{ field: 'taxonomy.performers', label: 'Performer', label_plural: 'Performers' }, { field: 'keywords' }] });
    expect(fieldFor(['tags'], {}, custom)).toBeUndefined();
    expect(fieldFor(['taxonomy', 'performers'], {}, custom)).toMatchObject({ kind: 'tags', label: 'Performers' });
    expect(fieldFor(['taxonomy', 'performers', 0], {}, custom)?.label).toBe('Performer');
    expect(fieldFor(['keywords'], {}, custom)?.label).toBe('Keywords');
  });

  it('gates contact fields on type person / organization (exact, trimmed)', () => {
    expect(fieldFor(['phones'], {}, catalog)).toBeUndefined();
    expect(fieldFor(['phones'], { type: 'person' }, catalog)?.kind).toBe('labeled');
    expect(fieldFor(['phones'], { type: ' organization ' }, catalog)?.kind).toBe('labeled');
    expect(fieldFor(['phones'], { type: 'Person' }, catalog)).toBeUndefined();
    expect(fieldFor(['first_name'], { type: 'organization' }, catalog)).toBeUndefined();
    expect(fieldFor(['company'], { type: 'organization' }, catalog)?.kind).toBe('wikilink');
    expect(isContactType({ type: 'person' })).toBe(true);
  });

  it('resolves the three labeled shapes to the item', () => {
    const fm = { type: 'person' };
    expect(fieldFor(['emails', 'work'], fm, catalog)?.label).toBe('Email');
    expect(fieldFor(['emails', 0], fm, catalog)?.label).toBe('Email');
    expect(fieldFor(['emails', 0, 'work'], fm, catalog)?.label).toBe('Email');
    expect(fieldFor(['emails', 0, 'value'], fm, catalog)?.label).toBe('Email');
    expect(fieldFor(['emails', 0, 'label'], fm, catalog)?.label).toBe('Label');
    expect(fieldFor(['addresses', 'home', 'city'], fm, catalog)?.label).toBe('City');
    expect(fieldFor(['addresses', 0, 'city'], fm, catalog)?.label).toBe('City');
    expect(fieldFor(['addresses', 0, 'home', 'postcode'], fm, catalog)?.label).toBe('Postcode');
    expect(fieldFor(['dates', 'birthday'], fm, catalog)?.kind).toBe('partialDate');
    expect(fieldFor(['dates'], fm, catalog)?.labels).toEqual(['birthday', 'death', 'anniversary']);
    expect(fieldFor(['phones'], fm, catalog)?.labels).toContain('mobile');
  });

  it('describes relationships from the registry', () => {
    const rel = fieldFor(['relationships'], {}, catalog)!;
    expect(rel.kind).toBe('relationships');
    const type = fieldFor(['relationships', 0, 'type'], {}, catalog)!;
    expect(type.enumValues!.map((e) => e.value)).toEqual(['parent', 'child', 'spouse']);
    expect(type.enumValues![0].help).toBe('Parent (family)');
    expect(fieldFor(['relationships', 0, 'to'], {}, catalog)?.kind).toBe('wikilink');
    expect(fieldFor(['relationships', 0, 'married'], {}, catalog)).toBeUndefined();
  });

  it('has the OKF fields for every note', () => {
    expect(fieldFor(['status'], {}, catalog)?.enumValues?.map((e) => e.value)).toEqual(['draft', 'stable', 'deprecated']);
    expect(fieldFor(['sources', 0, 'resource'], {}, catalog)).toMatchObject({ kind: 'text', required: true });
    expect(fieldFor(['sources', 0, 'author'], {}, catalog)?.kind).toBe('actor');
    expect(fieldFor(['sources', 0, 'usage_count'], {}, catalog)?.kind).toBe('number');
    expect(fieldFor(['sources', 0, 'last_modified'], {}, catalog)?.kind).toBe('datetime');
    expect(fieldFor(['sources', 0, 'usage_window', 'from'], {}, catalog)?.kind).toBe('datetime');
    expect(fieldFor(['usage_window', 'to'], {}, catalog)?.kind).toBe('datetime');
    expect(fieldFor(['generated', 'by'], {}, catalog)?.kind).toBe('actor');
    expect(fieldFor(['stale_after'], {}, catalog)?.kind).toBe('datetime');
  });

  it('reads verified as a list or a single map', () => {
    expect(fieldFor(['verified'], {}, catalog)?.singleOrList).toBe(true);
    expect(fieldFor(['verified', 0, 'by'], {}, catalog)?.kind).toBe('actor');
    expect(fieldFor(['verified', 'at'], {}, catalog)?.kind).toBe('datetime');
  });

  it('shows the Attested Computation group only for that type', () => {
    expect(fieldFor(['runtime'], {}, catalog)).toBeUndefined();
    const fm = { type: ATTESTED_COMPUTATION_TYPE };
    expect(fieldFor(['runtime'], fm, catalog)?.required).toBe(true);
    expect(fieldFor(['parameters', 0, 'required'], fm, catalog)?.kind).toBe('bool');
    expect(fieldFor(['executor', 'receipt', 0], fm, catalog)?.kind).toBe('text');
    expect(fieldFor(['attester', 'resource'], fm, catalog)?.kind).toBe('text');
  });
});

describe('resolveField', () => {
  const inferred = inferCatalog([{ frontmatter: { rating: 4, mood: 'calm' } }, { frontmatter: { rating: 5, mood: 'calm' } }]);

  it('prefers the built-in, then the vault’s, then the value', () => {
    expect(resolveField(['title'], {}, catalog, inferred).group).toBe('mbr');
    expect(resolveField(['rating'], {}, catalog, inferred)).toMatchObject({ kind: 'number', group: 'vault', label: 'Rating' });
    expect(resolveField(['mood'], {}, catalog, inferred).enumValues).toEqual([{ value: 'calm' }]);
    expect(resolveField(['when'], {}, catalog, inferred, '2026-01-02T03:04:05Z').kind).toBe('datetime');
    expect(resolveField(['x', 0], {}, catalog, inferred, true)).toMatchObject({ kind: 'bool', label: '#1' });
  });

  it('kindOfValue', () => {
    expect(kindOfValue('2026-10-21')).toBe('date');
    expect(kindOfValue('2026-10-21 10:00')).toBe('datetime');
    expect(kindOfValue('[[Note]]')).toBe('wikilink');
    expect(kindOfValue('a\nb')).toBe('longtext');
    expect(kindOfValue([])).toBe('list');
    expect(kindOfValue({})).toBe('map');
    expect(kindOfValue(null)).toBe('text');
  });

  it('humanizeKey', () => {
    expect(humanizeKey('job_title')).toBe('Job title');
    expect(humanizeKey('stale-after')).toBe('Stale after');
  });
});

describe('addableFields', () => {
  const inferred = inferCatalog([
    { frontmatter: { title: 'a', rating: 1, mood: 'x' } },
    { frontmatter: { rating: 2, 'dates.birthday': '1950' } },
  ]);

  it('lists applicable built-ins by group, then the vault’s keys by use, minus present keys', () => {
    const keys = addableFields({ title: 'x' }, catalog, inferred).map((f) => f.key);
    expect(keys).not.toContain('title');
    expect(keys.indexOf('type')).toBeLessThan(keys.indexOf('status'));
    expect(keys).not.toContain('phones');
    expect(keys).not.toContain('runtime');
    expect(keys.slice(-2)).toEqual(['rating', 'mood']);
    expect(keys).not.toContain('dates.birthday');
  });

  it('counts a dotted field written as one flat key as present', () => {
    const custom = builtinCatalog({ tagSources: [{ field: 'taxonomy.performers' }] });
    expect(addableFields({}, custom).map((f) => f.key)).toContain('taxonomy.performers');
    expect(addableFields({ 'taxonomy.performers': ['x'] }, custom).map((f) => f.key)).not.toContain('taxonomy.performers');
    expect(addableFields({ taxonomy: { performers: ['x'] } }, custom).map((f) => f.key)).not.toContain('taxonomy.performers');
  });

  it('adds contact and attested fields when the type calls for them, never deprecated ones', () => {
    const person = addableFields({ type: 'person' }, catalog).map((f) => f.key);
    expect(person.indexOf('phones')).toBeGreaterThan(person.indexOf('relationships'));
    expect(person.indexOf('phones')).toBeLessThan(person.indexOf('status'));
    expect(person).not.toContain('born');
    const attested = addableFields({ type: ATTESTED_COMPUTATION_TYPE }, catalog).map((f) => f.key);
    expect(attested.indexOf('runtime')).toBeLessThan(attested.indexOf('status'));
  });
});

describe('validate', () => {
  it('passes a clean note', () => {
    expect(validate({ title: 'x', status: 'draft', stale_after: '2026-06-30T14:00:00Z' }, catalog)).toEqual([]);
  });

  it('flags a status outside the OKF enum', () => {
    expect(validate({ status: 'final' }, catalog)).toEqual([{ path: ['status'], message: expect.stringContaining('final') }]);
  });

  it('flags a source without resource', () => {
    const hints = validate({ sources: [{ resource: 'https://x' }, { title: 'no resource' }] }, catalog);
    expect(hints).toEqual([{ path: ['sources', 1, 'resource'], message: 'Resource is required here.' }]);
  });

  it('flags timestamps without an offset, nested ones too', () => {
    const hints = validate(
      {
        stale_after: '2026-06-30T14:00:00',
        generated: { by: 'human:me', at: '2026-06-30 14:00' },
        verified: { by: 'human:me', at: '2026-06-30T14:00:00+02:00' },
        sources: [{ resource: 'x', last_modified: '2026-01-01' }],
      },
      catalog,
    );
    expect(hints.map((h) => h.path)).toEqual([['sources', 0, 'last_modified'], ['generated', 'at'], ['stale_after']]);
  });

  it('requires only `by` in generated and verified (OKF v0.2: `at` is optional)', () => {
    expect(validate({ generated: { by: 'human:me' }, verified: { by: 'human:me' } }, catalog)).toEqual([]);
    expect(validate({ verified: [{ by: 'a/1' }, { at: '2026-01-01T00:00:00Z' }] }, catalog)).toEqual([
      { path: ['verified', 1, 'by'], message: 'By is required here.' },
    ]);
    expect(validate({ generated: { at: '2026-01-01T00:00:00Z' } }, catalog)).toEqual([{ path: ['generated', 'by'], message: 'By is required here.' }]);
    expect(fieldFor(['generated', 'at'], {}, catalog)?.required).toBeFalsy();
    expect(fieldFor(['verified', 0, 'at'], {}, catalog)?.required).toBeFalsy();
  });

  it('checks each item of a verified list', () => {
    const hints = validate({ verified: [{ by: 'a/1', at: '2026-01-01T00:00:00Z' }, { by: 'b', at: '2026-01-01T00:00' }] }, catalog);
    expect(hints.map((h) => h.path)).toEqual([['verified', 1, 'at']]);
  });

  it('requires runtime for an Attested Computation only', () => {
    expect(validate({ type: ATTESTED_COMPUTATION_TYPE }, catalog)).toEqual([{ path: ['runtime'], message: expect.stringContaining('Runtime') }]);
    expect(validate({ type: 'Metric' }, catalog)).toEqual([]);
    expect(validate({ type: ATTESTED_COMPUTATION_TYPE, runtime: 'python', parameters: [{ type: 'int' }] }, catalog)).toEqual([
      { path: ['parameters', 0, 'name'], message: 'Name is required here.' },
    ]);
  });

  it('flags relationship types missing from the registry, case-insensitively', () => {
    const hints = validate({ relationships: [{ type: 'Parent', to: '[[A]]' }, { type: 'mentor', to: '[[B]]' }, { to: '[[C]]' }] }, catalog);
    expect(hints).toEqual([
      { path: ['relationships', 1, 'type'], message: expect.stringContaining('mentor') },
      { path: ['relationships', 2, 'type'], message: 'Type is required here.' },
    ]);
    // Without a registry there is nothing to compare against.
    expect(validate({ relationships: [{ type: 'mentor', to: '[[B]]' }] }, builtinCatalog())).toEqual([]);
  });
});

describe('date-times', () => {
  it('recognizes offsets', () => {
    expect(hasUtcOffset('2026-06-30T14:00:00Z')).toBe(true);
    expect(hasUtcOffset('2026-06-30T14:00:00-06:00')).toBe(true);
    expect(hasUtcOffset('2026-06-30 14:00 +0200')).toBe(true);
    expect(hasUtcOffset('2026-06-30T14:00:00')).toBe(false);
    expect(hasUtcOffset('2026-06-30')).toBe(false);
    expect(parseDateTime('2026-06-30T14:05:09.5Z')).toMatchObject({ hour: '14', minute: '05', second: '09', fraction: '.5', offset: 'Z' });
    expect(parseDateTime('nope')).toBeNull();
  });

  it('formats offsets', () => {
    expect(formatOffset(0)).toBe('Z');
    expect(formatOffset(-360)).toBe('-06:00');
    expect(formatOffset(330)).toBe('+05:30');
  });

  it('shows the stored wall time in a datetime-local input', () => {
    expect(datetimeLocalValue('2026-06-30T14:00:00Z')).toBe('2026-06-30T14:00:00');
    expect(datetimeLocalValue('2026-06-30 14:00 -06:00')).toBe('2026-06-30T14:00');
    expect(datetimeLocalValue('2026-06-30')).toBe('2026-06-30T00:00');
    expect(datetimeLocalValue('garbage')).toBe('');
  });

  it('writes new values with seconds and an explicit offset', () => {
    expect(applyDatetimeLocal('2026-06-30T14:00', undefined, 0)).toBe('2026-06-30T14:00:00Z');
    expect(applyDatetimeLocal('2026-06-30T14:00', undefined, -360)).toBe('2026-06-30T14:00:00-06:00');
    expect(applyDatetimeLocal('2026-06-30T14:00', '2026-01-01T00:00:00')).toMatch(/^2026-06-30T14:00:00(Z|[+-]\d{2}:\d{2})$/);
  });

  it('keeps the previous format: separator, seconds and offset', () => {
    expect(applyDatetimeLocal('2026-07-01T09:30', '2026-06-30 14:00 -06:00', 120)).toBe('2026-07-01 09:30-06:00');
    expect(applyDatetimeLocal('2026-07-01T09:30:15', '2026-06-30T14:00:00Z', 120)).toBe('2026-07-01T09:30:15Z');
    expect(applyDatetimeLocal('bad', '2026-06-30T14:00:00Z')).toBeNull();
  });
});

describe('partial dates (contact.rs PartialDate)', () => {
  it('parses every accepted form', () => {
    expect(parsePartialDate('1927-03-19')).toEqual({ year: 1927, month: 3, day: 19 });
    expect(parsePartialDate('1927-03')).toEqual({ year: 1927, month: 3 });
    expect(parsePartialDate('1927')).toEqual({ year: 1927 });
    expect(parsePartialDate(1927)).toEqual({ year: 1927 });
    expect(parsePartialDate('03-19')).toEqual({ month: 3, day: 19 });
    expect(parsePartialDate('--03-19')).toEqual({ month: 3, day: 19 });
    expect(parsePartialDate(' 2000-02-29 ')).toEqual({ year: 2000, month: 2, day: 29 });
    expect(parsePartialDate('--02-29')).toEqual({ month: 2, day: 29 });
  });

  it('rejects what the renderer rejects', () => {
    for (const bad of ['circa 1855', '27', '1927-3-19', '1927-13', '1927-02-30', '--13-01', '3-19', '1927-03-19-01', '', '19270']) {
      expect(parsePartialDate(bad), bad).toBeNull();
    }
    expect(parsePartialDate(null)).toBeNull();
  });

  it('formats as PartialDate::iso, keeping a bare MM-DD', () => {
    expect(formatPartialDate({ year: 1927, month: 3, day: 19 })).toBe('1927-03-19');
    expect(formatPartialDate({ year: 1927, month: 3 })).toBe('1927-03');
    expect(formatPartialDate({ year: 1927 })).toBe('1927');
    expect(formatPartialDate({ month: 3, day: 19 })).toBe('--03-19');
    expect(formatPartialDate({ month: 3, day: 19 }, '01-02')).toBe('03-19');
    expect(formatPartialDate({ year: 1927, day: 19 })).toBeNull();
    expect(formatPartialDate({ day: 19 })).toBeNull();
    expect(formatPartialDate({ month: 3 })).toBeNull();
    expect(formatPartialDate({ month: 2, day: 30 })).toBeNull();
    expect(formatPartialDate({})).toBeNull();
  });

  it('round-trips every valid form', () => {
    for (const s of ['1927-03-19', '1927-03', '1927', '--03-19', '0099-12-31']) {
      expect(formatPartialDate(parsePartialDate(s)!)).toBe(s);
    }
    expect(formatPartialDate(parsePartialDate('03-19')!, '03-19')).toBe('03-19');
  });

  it('displays like the contact card', () => {
    expect(displayPartialDate({ year: 1927, month: 3, day: 19 })).toBe('March 19, 1927');
    expect(displayPartialDate({ year: 1927, month: 3 })).toBe('March 1927');
    expect(displayPartialDate({ year: 1927 })).toBe('1927');
    expect(displayPartialDate({ month: 3, day: 19 })).toBe('March 19');
  });
});
