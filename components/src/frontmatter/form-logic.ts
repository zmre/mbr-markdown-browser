/**
 * The pure half of the properties form (`properties-form.ts`): which widget
 * draws a value, what an empty value of a kind is, which type changes are
 * lossless, how a dotted "+ Add property" key becomes model operations, and
 * how the date/number widgets' inputs become stored values.
 *
 * No DOM, no Lit: everything here is unit-tested directly.
 */

import {
  FrontmatterEditError,
  addItem,
  addKey,
  getIn,
  replaceSubtree,
  setValue,
  sliceAt,
  type FmPath,
  type FmValue,
  type FrontmatterModel,
} from './model.js';
import {
  formatPartialDate,
  isDateTimeString,
  parsePartialDate,
  timestampNow,
  type FieldDef,
  type FieldKind,
  type InferredFieldLike,
} from './fields.js';

/** What draws a value: a field kind, or `tree` (generic nested map/list rows). */
export type WidgetKind = FieldKind | 'tree';

/** A value with nothing in it: null, a blank string, `[]` or `{}`. */
export function isEmptyValue(value: FmValue | undefined): boolean {
  if (value === undefined || value === null) return true;
  if (typeof value === 'string') return value.trim() === '';
  if (Array.isArray(value)) return value.length === 0;
  if (typeof value === 'object') return Object.keys(value).length === 0;
  return false;
}

export function isScalarValue(value: FmValue | undefined): value is string | number | boolean | null {
  return value === null || value === undefined || typeof value !== 'object';
}

const DATE_ONLY = /^\d{4}-\d{2}-\d{2}$/;

/** The kinds whose widget edits one scalar. */
const SCALAR_KINDS: ReadonlySet<WidgetKind> = new Set<WidgetKind>([
  'text',
  'longtext',
  'number',
  'bool',
  'date',
  'datetime',
  'partialDate',
  'enum',
  'wikilink',
  'actor',
]);

export function isScalarKind(kind: WidgetKind): boolean {
  return SCALAR_KINDS.has(kind);
}

/** The kind a bare scalar reads as when its definition does not fit it. */
function scalarKindOf(value: FmValue): WidgetKind {
  if (typeof value === 'boolean') return 'bool';
  if (typeof value === 'number') return 'number';
  if (typeof value === 'string') {
    if (isDateTimeString(value)) return 'datetime';
    if (DATE_ONLY.test(value)) return 'date';
    if (value.includes('\n')) return 'longtext';
  }
  return 'text';
}

/** Whether `kind`'s widget can show `value` without misreading it. */
export function fits(kind: WidgetKind, value: FmValue | undefined): boolean {
  const v = value ?? null;
  if (kind === 'tree') return typeof v === 'object';
  if (kind === 'list') return v === null || (Array.isArray(v) && v.every((i) => isScalarValue(i)));
  // repo.rs reads a tag field as a list or as one comma-separated string.
  if (kind === 'tags') return v === null || (typeof v === 'string' && !v.includes('\n')) || (Array.isArray(v) && v.every((i) => isScalarValue(i)));
  if (kind === 'map' || kind === 'relationships' || kind === 'labeled') return v === null || typeof v === 'object';
  if (v === null) return true;
  if (typeof v === 'object') return false;
  switch (kind) {
    case 'number':
      return typeof v === 'number';
    case 'bool':
      return typeof v === 'boolean';
    case 'date':
      return typeof v === 'string' && (v.trim() === '' || DATE_ONLY.test(v.trim()));
    case 'datetime':
      return typeof v === 'string' && (v.trim() === '' || isDateTimeString(v) || DATE_ONLY.test(v.trim()));
    case 'partialDate':
      return (typeof v === 'string' && v.trim() === '') || parsePartialDate(v) !== null;
    case 'longtext':
      return typeof v === 'string';
    case 'text':
    case 'enum':
    case 'wikilink':
    case 'actor':
      // A multi-line string needs a textarea, whatever the field says.
      return typeof v === 'string' && !v.includes('\n');
    default:
      return false;
  }
}

/**
 * The widget for `value` under definition `def`: the definition's kind while
 * its widget can show the value, else one read off the value (a map or a
 * list of maps is a `tree`). `map`, `relationships` and `labeled` are drawn
 * as trees until a dedicated widget is registered for them.
 */
export function widgetKindFor(def: FieldDef, value: FmValue | undefined): WidgetKind {
  if (fits(def.kind, value)) return def.kind;
  const v = value ?? null;
  if (Array.isArray(v)) return v.every((i) => isScalarValue(i)) ? 'list' : 'tree';
  if (v !== null && typeof v === 'object') return 'tree';
  return scalarKindOf(v);
}

/**
 * What a new property of `kind` starts as. Scalars start empty (`key:`);
 * lists and maps too, so the first item is written in block style (`- x`
 * under the key) rather than as `[]`. Only a checkbox needs a value to show.
 */
export function emptyValueFor(kind: WidgetKind): FmValue {
  return kind === 'bool' ? false : null;
}

/**
 * What a new value for `def` starts as: {@link emptyValueFor} its kind,
 * except that a group starts with its required and seeded parts (a source
 * with its `resource:`, a review with `by:` and `at:`), and such a timestamp
 * starts at `now` in the local offset (a review is usually written when it
 * happens).
 */
export function newValueFor(def: FieldDef | undefined, now: Date = new Date()): FmValue {
  if (!def) return null;
  if (def.kind === 'map' && def.children) {
    const required = def.children.filter((c) => c.required || c.seed);
    if (required.length === 0) return null;
    return Object.fromEntries(required.map((c) => [c.key, c.kind === 'datetime' ? timestampNow(now) : newValueFor({ ...c, required: false, seed: false }, now)]));
  }
  return emptyValueFor(def.kind);
}

/**
 * A one-line summary of a list item that is a group, for its collapsed row:
 * its first required part's value, else the first short text in it
 * (`resource: https://…`, `by: human:pat`).
 */
export function itemSummary(value: FmValue | undefined, def: FieldDef | undefined): string {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return '';
  const text = (v: FmValue | undefined) => (typeof v === 'string' || typeof v === 'number' ? String(v).trim() : '');
  for (const c of def?.children ?? []) {
    if (c.required && text(value[c.key])) return text(value[c.key]);
  }
  for (const v of Object.values(value)) {
    const t = text(v);
    if (t && !t.includes('\n')) return t.length > 60 ? `${t.slice(0, 59)}…` : t;
  }
  return '';
}

/**
 * Appends `item` to a field that may be a list or one item written alone
 * (OKF `verified: {by, at}`). A lone map first becomes a one-item list of
 * its own source, so its bytes (comments, quoting) are kept, then the item
 * is added like any other.
 */
export function addToSingleOrList(model: FrontmatterModel, path: FmPath, item: FmValue): FrontmatterModel {
  const current = getIn(model, path);
  if (current === null || current === undefined || Array.isArray(current)) return addItem(model, path, item);
  const slice = sliceAt(model, path).replace(/\n+$/, '');
  const asItem = slice
    .split('\n')
    .map((line, i) => (i === 0 ? `- ${line}` : line === '' ? '' : `  ${line}`))
    .join('\n');
  return addItem(replaceSubtree(model, path, asItem), path, item);
}

/** Human names for the kinds, for the type menu and the icon's title. */
export const KIND_LABELS: Readonly<Record<WidgetKind, string>> = {
  text: 'Text',
  longtext: 'Long text',
  number: 'Number',
  bool: 'Checkbox',
  date: 'Date',
  datetime: 'Date & time',
  partialDate: 'Partial date',
  list: 'List',
  tags: 'Tags',
  enum: 'Choice',
  wikilink: 'Note link',
  map: 'Group',
  relationships: 'Relationships',
  labeled: 'Labeled values',
  actor: 'Actor',
  tree: 'Group',
};

/** One entry of a row's "Change type" menu. */
export interface Conversion {
  readonly kind: WidgetKind;
  readonly label: string;
  /** The value to write, or `undefined` when the value stays as it is (only the widget changes). */
  readonly write?: FmValue;
}

const CHANGEABLE: readonly WidgetKind[] = ['text', 'longtext', 'number', 'bool', 'date', 'datetime', 'list', 'tree'];

/**
 * The type changes for a value that lose nothing: anything while the value
 * is empty; a scalar into a one-item list and back; text into a number or a
 * checkbox when it already spells one; a number or a checkbox into text;
 * text into a date when it is one; one line of text into long text and back.
 * `current` is left out.
 */
export function typeConversions(value: FmValue | undefined, current: WidgetKind): Conversion[] {
  const v = value ?? null;
  const out: Conversion[] = [];
  const push = (kind: WidgetKind, write?: FmValue) => {
    if (kind !== current && !out.some((c) => c.kind === kind)) out.push({ kind, label: KIND_LABELS[kind], write });
  };
  if (isEmptyValue(v)) {
    // Nothing to lose. A checkbox needs a value; the rest start (or become) empty.
    for (const kind of CHANGEABLE) push(kind, kind === 'bool' ? false : fits(kind, v) ? undefined : null);
    return out;
  }
  if (Array.isArray(v)) {
    if (v.length === 1 && isScalarValue(v[0])) {
      const item = v[0] ?? null;
      push(scalarKindOf(item), item);
    }
    if (v.every((i) => isScalarValue(i)) && current !== 'list') push('list');
    if (current !== 'tree') push('tree');
    return out;
  }
  if (typeof v === 'object') return out;
  if (typeof v === 'number' || typeof v === 'boolean') push('text', String(v));
  if (typeof v === 'string') {
    const t = v.trim();
    if (t !== '' && /^-?\d+(\.\d+)?$/.test(t) && Number.isFinite(Number(t))) push('number', Number(t));
    if (t === 'true' || t === 'false') push('bool', t === 'true');
    if (DATE_ONLY.test(t)) push('date');
    if (isDateTimeString(t)) push('datetime');
    if (v.includes('\n')) push('longtext');
    else {
      push('text');
      push('longtext');
    }
  }
  push('list', [v]);
  return out;
}

/** A stable key for a path (UI state maps: collapsed rows, kind overrides). */
export function pathKey(path: FmPath): string {
  return JSON.stringify(path);
}

/**
 * Adds a top-level property, or a nested one for a dotted catalog key
 * (`taxonomy.performers`): into the existing map at its parent, else as a
 * new map. Throws `FrontmatterEditError` as the model does (`exists`, …).
 */
export function addProperty(model: FrontmatterModel, key: string, value: FmValue): FrontmatterModel {
  if (existingPropertyPath(model, key)) throw new FrontmatterEditError('exists', `${key} already exists`);
  const segs = key.split('.');
  if (segs.length === 1 || segs.some((s) => s === '')) return addKey(model, [], key, value);
  let depth = 0;
  let current = getIn(model, []);
  while (depth < segs.length - 1) {
    const next = current && typeof current === 'object' && !Array.isArray(current) ? current[segs[depth]] : undefined;
    if (next === undefined || next === null || typeof next !== 'object' || Array.isArray(next)) break;
    current = next;
    depth++;
  }
  const parent = segs.slice(0, depth);
  const rest = segs.slice(depth);
  const nested = rest.slice(1).reduceRight<FmValue>((acc, seg) => ({ [seg]: acc }), value);
  const existing = getIn(model, [...parent, rest[0]]);
  if (existing === null && rest.length > 1) return setValue(model, [...parent, rest[0]], nested);
  return addKey(model, parent, rest[0], nested);
}

/** The path a dotted property key names. */
export function propertyPath(key: string): string[] {
  return key.split('.').some((s) => s === '') ? [key] : key.split('.');
}

/**
 * Where the property `key` already is, or `undefined`. A dotted key
 * (`dates.birthday`) counts whether it is written nested or as one flat key
 * (`dates.birthday: …`, which contact.rs reads too).
 */
export function existingPropertyPath(model: FrontmatterModel, key: string): FmPath | undefined {
  const nested = propertyPath(key);
  if (getIn(model, nested) !== undefined) return nested;
  if (nested.length > 1 && getIn(model, [key]) !== undefined) return [key];
  return undefined;
}

/** A number input's value: a number, `null` when cleared, `undefined` while it is not one yet. */
export function parseNumberInput(input: string): number | null | undefined {
  const t = input.trim();
  if (t === '') return null;
  const n = Number(t);
  return Number.isFinite(n) ? n : undefined;
}

/** The three fields of a partial date as typed (`''` for a blank one). */
export interface PartialDateInput {
  readonly year: string;
  readonly month: string;
  readonly day: string;
}

/** The fields shown for a stored partial date (blanks where it has none). */
export function partialDateFields(value: FmValue | undefined): PartialDateInput {
  const p = parsePartialDate(value ?? null);
  const pad = (n: number | undefined, w: number) => (n === undefined ? '' : String(n).padStart(w, '0'));
  return { year: pad(p?.year, 4), month: pad(p?.month, 2), day: pad(p?.day, 2) };
}

export type PartialDateResult =
  | { readonly ok: true; readonly value: FmValue }
  | { readonly ok: false; readonly message: string };

/**
 * The value to store for a partial date's fields: `null` when all are blank,
 * else `formatPartialDate`'s spelling. A year alone stays a number when the
 * stored value was one (`born: 1927`), so a round trip changes nothing.
 */
export function partialDateValue(input: PartialDateInput, previous: FmValue | undefined): PartialDateResult {
  const year = input.year.trim();
  const month = input.month.trim();
  const day = input.day.trim();
  if (!year && !month && !day) return { ok: true, value: null };
  if (year && !/^\d{4}$/.test(year)) return { ok: false, message: 'The year is four digits.' };
  const num = (s: string) => (s ? Number(s) : undefined);
  const formatted = formatPartialDate({ year: num(year), month: num(month), day: num(day) }, previous);
  if (formatted === null) {
    if (day && !month) return { ok: false, message: 'A day needs a month.' };
    return { ok: false, message: 'That is not a date.' };
  }
  if (/^\d{4}$/.test(formatted) && typeof previous === 'number') return { ok: true, value: Number(formatted) };
  return { ok: true, value: formatted };
}

/** One suggestion for a value or key combobox. */
export interface ComboOption {
  readonly value: string;
  /** Shown instead of the value (the value is still what is written). */
  readonly label?: string;
  /** Secondary text: an enum's help, a vault count. */
  readonly detail?: string;
}

/**
 * Value suggestions for a field: its definition's known values (with their
 * help) first, then what the vault uses for the same key, most used first,
 * without repeats.
 */
export function valueSuggestions(
  path: FmPath,
  def: FieldDef,
  inferred: ReadonlyMap<string, InferredFieldLike> | undefined,
): ComboOption[] {
  const out: ComboOption[] = [];
  const seen = new Set<string>();
  for (const opt of def.enumValues ?? []) {
    if (seen.has(opt.value)) continue;
    seen.add(opt.value);
    out.push({ value: opt.value, detail: opt.help });
  }
  // Inferred keys are dotted names; list items are counted under their list's key.
  const dotted = path.filter((s): s is string => typeof s === 'string').join('.');
  for (const v of inferred?.get(dotted)?.values ?? []) {
    if (seen.has(v.value)) continue;
    seen.add(v.value);
    out.push({ value: v.value, detail: `${v.count} note${v.count === 1 ? '' : 's'}` });
  }
  return out;
}
