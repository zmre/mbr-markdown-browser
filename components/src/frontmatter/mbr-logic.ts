/**
 * The pure half of the mbr/OKF widgets (`mbr-widgets.ts`): reading
 * relationships, labeled contact values, tags and actors out of frontmatter
 * values, phrasing a relationship, building suggestions, and the model
 * operations each widget edits through. Every write is a `model.ts`
 * operation on the smallest node that changes, so the rest of the block
 * keeps its bytes.
 *
 * Mirrors the Rust readers (cited per section) so what the form shows is
 * what the renderer will read. No DOM, no Lit.
 */

import {
  addItem,
  addKey,
  getIn,
  removeItem,
  removeKey,
  renameKey,
  setScalar,
  setValue,
  type FmPath,
  type FmValue,
  type FrontmatterModel,
} from './model.js';
import { humanizeKey, type FieldCatalog } from './fields.js';
import type { InferredField } from './infer.js';
import { vaultActors } from './infer.js';
import { addToSingleOrList, type ComboOption } from './form-logic.js';
import type { RelationTypeConfig } from '../graph/relationship-graph.js';
import { buildLinkTargets, wikilinkName, wikilinkText, type LinkTargetSite } from '../link-targets.js';

type Obj = { [key: string]: FmValue };

const isObj = (v: FmValue | undefined): v is Obj => v !== null && v !== undefined && typeof v === 'object' && !Array.isArray(v);
const scalarText = (v: FmValue | undefined): string => (v === null || v === undefined || typeof v === 'object' ? '' : String(v).trim());

// ============================================================================
// Relationships (relationships.rs `parse_declared_relationships`, config.rs)
// ============================================================================

/**
 * `default_relationship_types()` in config.rs, for a `site.json` without a
 * `relationship_types` (relationship tracking off, or an older server).
 * Keep in step with the Rust table.
 */
export const DEFAULT_RELATIONSHIP_TYPES: readonly RelationTypeConfig[] = [
  { name: 'parent', inverse: 'child', symmetric: false, hierarchy: 'up', category: 'family', label: 'Parent', label_plural: 'Parents' },
  { name: 'child', inverse: 'parent', symmetric: false, hierarchy: 'down', category: 'family', label: 'Child', label_plural: 'Children' },
  { name: 'spouse', inverse: null, symmetric: true, category: 'family', label: 'Spouse', label_plural: 'Spouses' },
  { name: 'sibling', inverse: null, symmetric: true, category: 'family', label: 'Sibling', label_plural: 'Siblings' },
  { name: 'reports_to', inverse: 'manages', symmetric: false, hierarchy: 'up', category: 'work', label: 'Reports to', label_plural: 'Reports to' },
  { name: 'manages', inverse: 'reports_to', symmetric: false, hierarchy: 'down', category: 'work', label: 'Manages', label_plural: 'Manages' },
  { name: 'assistant', inverse: 'assists', symmetric: false, hierarchy: 'down', category: 'work', label: 'Assistant', label_plural: 'Assistants' },
  { name: 'assists', inverse: 'assistant', symmetric: false, hierarchy: 'up', category: 'work', label: 'Assists', label_plural: 'Assists' },
  { name: 'employer', inverse: 'employee', symmetric: false, hierarchy: 'up', category: 'work', label: 'Employer', label_plural: 'Employers' },
  { name: 'employee', inverse: 'employer', symmetric: false, hierarchy: 'down', category: 'work', label: 'Employee', label_plural: 'Employees' },
  { name: 'colleague', inverse: null, symmetric: true, category: 'work', label: 'Colleague', label_plural: 'Colleagues' },
];

/** `type`, `to`, `from` and `label`: everything else on an edge is an attribute (`RESERVED_EDGE_KEYS`). */
export const RESERVED_EDGE_KEYS: readonly string[] = ['type', 'to', 'from', 'label'];

/** One declared edge, as the form reads it. */
export interface EdgeView {
  readonly type: string;
  readonly to: string;
  readonly from: string;
  readonly label: string;
  readonly hasTo: boolean;
  readonly hasFrom: boolean;
  /** Keys other than the reserved four, in authored order. */
  readonly attributes: readonly string[];
}

/** The edge an item of `relationships` declares, or `null` for an item that is not a map. */
export function edgeOf(item: FmValue | undefined): EdgeView | null {
  if (!isObj(item)) return null;
  return {
    type: scalarText(item.type),
    to: scalarText(item.to),
    from: scalarText(item.from),
    label: scalarText(item.label),
    hasTo: 'to' in item,
    hasFrom: 'from' in item,
    attributes: Object.keys(item).filter((k) => !RESERVED_EDGE_KEYS.includes(k)),
  };
}

/**
 * Which endpoint the note picker edits: `to` ("X is my <type>") or `from`
 * ("I am X's <type>"); `both` when the edge names two other notes; `to` for
 * an edge naming neither yet.
 */
export function edgeDirection(edge: EdgeView): 'to' | 'from' | 'both' {
  if (edge.hasTo && edge.hasFrom) return 'both';
  return edge.hasFrom ? 'from' : 'to';
}

/** The registry entry for `type` (case-insensitive, as the registry matches). */
export function relationType(registry: readonly RelationTypeConfig[], type: string): RelationTypeConfig | undefined {
  const t = type.trim().toLowerCase();
  return t ? registry.find((r) => r.name.toLowerCase() === t) : undefined;
}

/** The type picker's suggestions: the registry's types, label and category as detail. */
export function relationTypeOptions(registry: readonly RelationTypeConfig[]): ComboOption[] {
  return registry.map((t) => ({ value: t.name, detail: t.category ? `${t.label} · ${t.category}` : t.label }));
}

const displayEndpoint = (value: string): string => wikilinkName(value) ?? value;

const possessive = (name: string): string => (name === 'this note' ? 'this note’s' : `${name}’s`);

const capitalize = (s: string): string => (s ? s[0].toUpperCase() + s.slice(1) : s);

/**
 * A plain-language reading of an edge, with the reverse edge mbr derives:
 * `type: parent, to: [[Sam]]` on Jane reads "Sam is Jane’s parent · Jane is
 * Sam’s child". A type whose singular and plural labels are the same
 * (`Reports to`, `Manages`) is a verb: "Jane reports to Sam · Sam manages
 * Jane". `me` names the note being edited. An edge naming no other note
 * yet reads `null`.
 */
export function relationshipSentence(edge: EdgeView, registry: readonly RelationTypeConfig[], me: string): string | null {
  const subject = edge.from ? displayEndpoint(edge.from) : me;
  const object = edge.to ? displayEndpoint(edge.to) : me;
  if ((!edge.to && !edge.from) || !edge.type) return null;
  const rt = relationType(registry, edge.type);
  const label = (edge.label || rt?.label || humanizeKey(edge.type)).toLowerCase();
  const isVerb = (t: RelationTypeConfig | undefined) => Boolean(t && t.label === t.label_plural);
  const phrase = (s: string, o: string, t: RelationTypeConfig | undefined, l: string) =>
    isVerb(t) && !(t === rt && edge.label) ? `${capitalize(s)} ${l} ${o}` : `${capitalize(o)} is ${possessive(s)} ${l}`;
  if (rt?.symmetric) return `${capitalize(subject)} and ${object} are ${(rt.label_plural || `${rt.label}s`).toLowerCase()}`;
  const forward = phrase(subject, object, rt, label);
  const inverse = rt?.inverse ? relationType(registry, rt.inverse) : undefined;
  if (!inverse) return forward;
  return `${forward} · ${phrase(object, subject, inverse, inverse.label.toLowerCase())}`;
}

/** The words on an edge's direction toggle (checked = the edge uses `from`). */
export const FLIP_DIRECTION_LABEL = 'Flip relationship direction';

/**
 * Whether an edge offers the direction toggle: not where direction means
 * nothing (a symmetric type) nor on an edge naming two other notes.
 */
export function canFlipDirection(edge: EdgeView, registry: readonly RelationTypeConfig[]): boolean {
  if (edgeDirection(edge) === 'both') return false;
  return !relationType(registry, edge.type)?.symmetric;
}

/** Swaps an edge's `to` for `from` or back (only the key token changes). */
export function toggleEdgeDirection(model: FrontmatterModel, edgePath: FmPath, edge: EdgeView): FrontmatterModel {
  const dir = edgeDirection(edge);
  if (dir === 'both') return model;
  if (dir === 'to' && !edge.hasTo) return addKey(model, edgePath, 'from', null);
  return renameKey(model, [...edgePath, dir], dir === 'to' ? 'from' : 'to');
}

/** Sets an edge's key (`type`, an endpoint, `label`), adding it when absent. */
export function setEdgeField(model: FrontmatterModel, edgePath: FmPath, key: string, value: string): FrontmatterModel {
  return setScalar(model, [...edgePath, key], value === '' ? null : value);
}

/** A new edge: `type` and `to`, empty until chosen. */
export function newEdge(): FmValue {
  return { type: null, to: null };
}

// ============================================================================
// Labeled values (contact.rs `labeled_values`, `labeled_item`)
// ============================================================================

/** How a labeled field is written, which new entries follow. */
export type LabeledShape = 'map' | 'list' | 'single' | 'empty';

/**
 * One entry, with where its label and value live:
 * - `key`: a map entry (`work: x`), the label is the key;
 * - `pair`: a list item `- work: x` (a one-key map), the label is its key;
 * - `labelValue`: `{label: work, value: x}` (or `name:`), in a list or alone;
 * - `address`: a structured address, its `label` (if any) beside the parts;
 * - `bare`: a value with no label (a list item, or the whole field).
 */
export interface LabeledEntry {
  readonly form: 'key' | 'pair' | 'labelValue' | 'address' | 'bare';
  readonly label: string;
  readonly value: FmValue;
  /** Where the value is (for `address`, the map holding the parts). */
  readonly valuePath: FmPath;
  /** The list index, for list items. */
  readonly index?: number;
}

/** `ADDRESS_FIELDS` in contact.rs. */
export const ADDRESS_PARTS: readonly string[] = ['street', 'city', 'region', 'postcode', 'country', 'country_code'];

const isAddressObject = (v: Obj): boolean => Object.keys(v).some((k) => ADDRESS_PARTS.includes(k));

function labelValueKey(v: Obj): 'value' | 'name' | null {
  const keys = Object.keys(v);
  if (keys.length !== 2 || !keys.includes('label')) return null;
  if (keys.includes('value')) return 'value';
  if (keys.includes('name')) return 'name';
  return null;
}

/** The shape a labeled field is written in. */
export function labeledShape(value: FmValue | undefined): LabeledShape {
  if (value === null || value === undefined) return 'empty';
  if (Array.isArray(value)) return 'list';
  if (isObj(value)) return 'map';
  return 'single';
}

/**
 * The entries of a labeled field in authored order, read as contact.rs
 * does: a map is label → value (unless it is one `{label, value}` or, for
 * addresses, one address); a list's items are bare values, one-key
 * `label: value` maps, `{label, value}` maps or addresses; a scalar is one
 * bare value. A list item none of those can read (a nested list, a map of
 * several keys) is `null` in the result, so the caller can show it as YAML.
 */
export function labeledEntries(value: FmValue | undefined, path: FmPath, field: string): (LabeledEntry | null)[] {
  if (value === null || value === undefined) return [];
  if (Array.isArray(value)) {
    return value.map((item, index): LabeledEntry | null => {
      const at = [...path, index];
      if (isObj(item)) {
        const lv = labelValueKey(item);
        if (lv) return { form: 'labelValue', label: scalarText(item.label), value: item[lv], valuePath: [...at, lv], index };
        if (field === 'addresses' && isAddressObject(item)) return { form: 'address', label: scalarText(item.label), value: item, valuePath: at, index };
        const keys = Object.keys(item);
        if (keys.length === 1) return { form: 'pair', label: keys[0], value: item[keys[0]], valuePath: [...at, keys[0]], index };
        return null;
      }
      if (Array.isArray(item)) return null;
      return { form: 'bare', label: '', value: item, valuePath: at, index };
    });
  }
  if (isObj(value)) {
    const lv = labelValueKey(value);
    if (lv) return [{ form: 'labelValue', label: scalarText(value.label), value: value[lv], valuePath: [...path, lv] }];
    if (field === 'addresses' && isAddressObject(value)) return [{ form: 'bare', label: '', value, valuePath: path }];
    return Object.keys(value).map((k) => ({ form: 'key' as const, label: k, value: value[k], valuePath: [...path, k] }));
  }
  return [{ form: 'bare', label: '', value, valuePath: path }];
}

/**
 * Relabels an entry, touching only its label: a map key or a one-key item's
 * key is renamed, a `label:` value is set, and a bare list item becomes a
 * one-key `label: value` item. An empty label removes it where the shape
 * allows (a one-key item becomes bare; a map key cannot be empty, so it stays).
 */
export function setEntryLabel(model: FrontmatterModel, fieldPath: FmPath, entry: LabeledEntry, label: string): FrontmatterModel {
  const next = label.trim();
  if (next === entry.label) return model;
  switch (entry.form) {
    case 'key':
      return next ? renameKey(model, entry.valuePath, next) : model;
    case 'pair':
      return next ? renameKey(model, entry.valuePath, next) : setValue(model, [...fieldPath, entry.index!], entry.value);
    case 'labelValue':
      return setScalar(model, [...entry.valuePath.slice(0, -1), 'label'], next || null);
    case 'address':
      if (next) return setScalar(model, [...entry.valuePath, 'label'], next);
      return 'label' in (entry.value as Obj) ? removeKey(model, [...entry.valuePath, 'label']) : model;
    case 'bare':
      if (!next || entry.index === undefined) return model;
      return setValue(model, entry.valuePath, { [next]: entry.value });
  }
}

/** Removes an entry (a list item, or a map key; a single value clears the field). */
export function removeEntry(model: FrontmatterModel, fieldPath: FmPath, entry: LabeledEntry): FrontmatterModel {
  if (entry.index !== undefined) return removeItem(model, fieldPath, entry.index);
  if (entry.form === 'key') return removeKey(model, entry.valuePath);
  return setValue(model, fieldPath, null);
}

/** The first suggested label not used yet (a map cannot hold one twice), else `label N`. */
export function freshLabel(used: readonly string[], suggested: readonly string[]): string {
  const taken = new Set(used.map((u) => u.toLowerCase()));
  const free = suggested.find((s) => !taken.has(s.toLowerCase()));
  if (free) return free;
  for (let n = 2; ; n++) if (!taken.has(`other${n}`)) return `other${n}`;
}

/**
 * Adds an entry in the shape the field already uses: a map gains a key, a
 * list an item like its others (`label: value`, `{label, value}`, or bare
 * when every item is bare), an empty field becomes a map, and a single value
 * becomes a list holding it and the new entry.
 */
export function addLabeledEntry(model: FrontmatterModel, fieldPath: FmPath, current: FmValue | undefined, field: string, label: string, value: FmValue): FrontmatterModel {
  const shape = labeledShape(current);
  if (shape === 'empty') return getIn(model, fieldPath) === undefined ? setValue(model, fieldPath, { [label]: value }) : addKey(model, fieldPath, label, value);
  if (shape === 'single') return setValue(model, fieldPath, [current as FmValue, { [label]: value }]);
  if (shape === 'map') {
    const entries = labeledEntries(current, fieldPath, field);
    // One `{label, value}` or one address is a single entry, not a map of labels.
    // It becomes a list holding its own source, so its comments and quoting stay.
    if (entries.length === 1 && entries[0]?.form !== 'key') {
      return addToSingleOrList(model, fieldPath, newListItem(entries, field, label, value));
    }
    return addKey(model, fieldPath, label, value);
  }
  return addItem(model, fieldPath, newListItem(labeledEntries(current, fieldPath, field), field, label, value));
}

function newListItem(entries: readonly (LabeledEntry | null)[], field: string, label: string, value: FmValue): FmValue {
  const forms = entries.filter((e): e is LabeledEntry => e !== null).map((e) => e.form);
  if (forms.length > 0 && forms.every((f) => f === 'bare')) return value;
  if (forms.includes('labelValue') && !forms.includes('pair')) return { label, value };
  if (field === 'addresses' && isObj(value)) return { label, ...value };
  return { [label]: value };
}

// ============================================================================
// Tags (repo.rs `extract_tag_values`: a list, or a comma-separated string)
// ============================================================================

/** The tags a value holds: list items, or a comma-separated string split and trimmed. */
export function tagItems(value: FmValue | undefined): string[] {
  if (Array.isArray(value)) return value.map((v) => scalarText(v)).filter((t) => t !== '');
  if (typeof value === 'string') return value.split(',').map((t) => t.trim()).filter((t) => t !== '');
  if (typeof value === 'number' || typeof value === 'boolean') return [String(value)];
  return [];
}

/** The separator a comma string uses (`, ` unless the author wrote bare commas). */
function separatorOf(s: string): string {
  return s.includes(',') && !/,\s/.test(s) ? ',' : ', ';
}

/** Adds a tag, keeping the field's shape: a comma string stays a string. */
export function addTag(model: FrontmatterModel, path: FmPath, value: FmValue | undefined, tag: string): FrontmatterModel {
  if (typeof value === 'string' && value.trim() !== '') {
    const items = tagItems(value);
    return setScalar(model, path, [...items, tag].join(separatorOf(value)));
  }
  if (typeof value === 'number' || typeof value === 'boolean') return setValue(model, path, [value, tag]);
  return addItem(model, path, tag);
}

/** Removes tag `index` (of {@link tagItems}), keeping the field's shape. */
export function removeTag(model: FrontmatterModel, path: FmPath, value: FmValue | undefined, index: number): FrontmatterModel {
  if (Array.isArray(value)) {
    // `tagItems` skips blank items; map back to the item's own index.
    let seen = -1;
    const at = value.findIndex((v) => scalarText(v) !== '' && ++seen === index);
    return at < 0 ? model : removeItem(model, path, at);
  }
  if (typeof value === 'string') {
    const items = tagItems(value);
    items.splice(index, 1);
    return setScalar(model, path, items.length > 0 ? items.join(separatorOf(value)) : null);
  }
  return setScalar(model, path, null);
}

/** The catalog's tag fields (the configured tag sources), dotted. */
export function tagFieldKeys(catalog: FieldCatalog): string[] {
  return catalog.fields.filter((f) => f.kind === 'tags').map((f) => f.key);
}

/**
 * Tag suggestions for `field`: the values the vault uses there, then those
 * of the other tag fields, each split like a comma string, without repeats
 * and without `exclude` (the tags already present).
 */
export function tagSuggestions(field: string, catalog: FieldCatalog, inferred: ReadonlyMap<string, InferredField>, exclude: readonly string[] = []): ComboOption[] {
  const keys = [field, ...tagFieldKeys(catalog).filter((k) => k !== field)];
  const counts = new Map<string, { count: number; own: boolean }>();
  for (const key of keys) {
    for (const v of inferred.get(key)?.values ?? []) {
      for (const tag of tagItems(v.value)) {
        const prev = counts.get(tag);
        if (prev) prev.count += v.count;
        else counts.set(tag, { count: v.count, own: key === field });
      }
    }
  }
  const skip = new Set(exclude);
  return [...counts]
    .filter(([tag]) => !skip.has(tag))
    .sort((a, b) => Number(b[1].own) - Number(a[1].own) || b[1].count - a[1].count || a[0].localeCompare(b[0]))
    .map(([tag, c]) => ({ value: tag, detail: `${c.count} note${c.count === 1 ? '' : 's'}` }));
}

// ============================================================================
// Note links and actors
// ============================================================================

/**
 * `"[[Name]]"` suggestions for every note, the path as detail so two notes
 * of one title can be told apart.
 */
export function noteLinkOptions(files: readonly LinkTargetSite[]): ComboOption[] {
  const seen = new Set<string>();
  const out: ComboOption[] = [];
  for (const t of buildLinkTargets(files)) {
    const key = `${t.wiki}\u0000${t.path}`;
    if (seen.has(key)) continue;
    seen.add(key);
    out.push({ value: wikilinkText(t), label: t.title, detail: t.path });
  }
  return out;
}

/**
 * The reader's own actor id for the "me" shortcut (`human:<id>`), kept in
 * memory only. It is not stored: `localStorage` is reserved by
 * `edit-token.test.ts` to an audited allowlist, and an id typed once per
 * page is cheap.
 */
export interface ActorMemory {
  id: string | null;
}

export function createActorMemory(): ActorMemory {
  return { id: null };
}

/** OKF §7 spellings, offered as starting points. */
const ACTOR_HINTS: readonly ComboOption[] = [
  { value: 'human:', detail: 'A person: human:<id>' },
  { value: 'process:', detail: 'A pipeline or job: process:<id>' },
];

/**
 * Actor suggestions: the reader's own (when set), the vault's actors by
 * use, then the `human:`/`process:` prefixes.
 */
export function actorOptions(inferred: ReadonlyMap<string, InferredField>, memory?: ActorMemory): ComboOption[] {
  const out: ComboOption[] = [];
  const seen = new Set<string>();
  const push = (o: ComboOption) => {
    if (seen.has(o.value)) return;
    seen.add(o.value);
    out.push(o);
  };
  if (memory?.id) push({ value: `human:${memory.id}`, detail: 'You' });
  for (const a of vaultActors(inferred)) push({ value: a.value, detail: `${a.count} note${a.count === 1 ? '' : 's'}` });
  for (const h of ACTOR_HINTS) push(h);
  return out;
}

/** The id in `human:<id>`, or `null`. */
export function humanActorId(value: string): string | null {
  const m = /^human:(\S+)$/.exec(value.trim());
  return m ? m[1] : null;
}
