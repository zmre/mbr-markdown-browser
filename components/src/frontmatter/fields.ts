/**
 * What the frontmatter editor knows about fields: a catalog of built-in
 * definitions (mbr's own vocabulary and Google's OKF v0.2), resolution of
 * the definition for a path, soft validation hints, and the date helpers the
 * widgets write through.
 *
 * Pure: no DOM, no fetches. Everything vault-specific (tag sources, the
 * relationship registry, inferred values) is passed in.
 *
 * Sources for the built-ins, so they can be checked when the Rust side moves:
 * - `type`, `style` and the body class: `crates/mbr-core/src/templates.rs`
 *   (`body_class_list`), `docs/markdown/styles.md`.
 * - Contacts: `crates/mbr-core/src/contact.rs` (`Contact::from_map`,
 *   `labeled_values`, `PartialDate`, `ADDRESS_FIELDS`, `WELL_KNOWN_DATES`) and
 *   `docs/markdown/contacts.md` (suggested labels).
 * - Relationships: `crates/mbr-core/src/relationships.rs`
 *   (`parse_declared_relationships`: `type`/`to`/`from`/`label` reserved, any
 *   other key an edge attribute; `company_employer_relationship`) and
 *   `docs/markdown/relationships.md`.
 * - Tags: `crates/mbr-core/src/config.rs` (`TagSource`, `default_tag_sources`).
 * - `order`/`pinned`: `crates/mbr-core/src/sorting.rs` (sort fields).
 * - OKF v0.2: https://github.com/GoogleCloudPlatform/knowledge-catalog/blob/main/okf/SPEC.md
 *   (§5.1 provenance, §5.2–5.3 trust, §5.4–5.5 lifecycle, §7 actors, §10.2
 *   Attested Computation). OKF consumers must keep unknown keys, which the
 *   model does by construction.
 */

import type { RelationTypeConfig } from '../graph/relationship-graph.js';

// ============================================================================
// Types
// ============================================================================

export type FieldKind =
  | 'text'
  | 'longtext'
  | 'number'
  | 'bool'
  | 'date'
  | 'datetime'
  | 'partialDate'
  | 'list'
  | 'tags'
  | 'enum'
  | 'wikilink'
  | 'map'
  | 'relationships'
  | 'labeled'
  | 'actor';

export interface EnumOption {
  readonly value: string;
  readonly help?: string;
}

/** Which family a built-in belongs to, for ordering "+ Add property". */
export type FieldGroup = 'mbr' | 'contact' | 'okf' | 'attested' | 'vault';

/** Frontmatter as the catalog reads it (the model's JS value). */
export type Frontmatter = Readonly<Record<string, unknown>>;

export interface FieldDef {
  /** The key; a dotted key (`taxonomy.performers`) names a nested path. */
  readonly key: string;
  readonly kind: FieldKind;
  readonly label: string;
  readonly help?: string;
  readonly group: FieldGroup;
  /** Suggested values; for `enum` the known ones (free text stays allowed). */
  readonly enumValues?: readonly EnumOption[];
  /** Whether this field applies to a note (e.g. contact fields on `type: person`). */
  readonly when?: (fm: Frontmatter) => boolean;
  /** For `list`, `tags`, `labeled` and `relationships`: one item. */
  readonly item?: FieldDef;
  /** For `map` (and map items): the known keys. Unknown keys stay editable. */
  readonly children?: readonly FieldDef[];
  /** Validation: a value is expected here (soft, never blocking). */
  readonly required?: boolean;
  /**
   * Written into a new parent item although not required (a datetime starts
   * at now): OKF's `generated.at` / `verified[].at` are optional, but a new
   * review is made now.
   */
  readonly seed?: boolean;
  /** For `labeled`: suggested labels. */
  readonly labels?: readonly string[];
  /** A list that may also be written as a single item (OKF `verified`). */
  readonly singleOrList?: boolean;
  /** Superseded spelling; still read, not offered for new notes. */
  readonly deprecated?: string;
}

/** A tag source as `window.__MBR_CONFIG__.tagSources` carries it. */
export interface TagSourceLike {
  readonly field: string;
  readonly label?: string | null;
  readonly label_plural?: string | null;
}

export interface CatalogOptions {
  /** Configured tag fields; `tags` when absent or empty (`default_tag_sources`). */
  readonly tagSources?: readonly TagSourceLike[];
  /** `site.json`'s `relationship_types`. */
  readonly relationshipTypes?: readonly RelationTypeConfig[];
}

export interface FieldCatalog {
  readonly fields: readonly FieldDef[];
  readonly relationshipTypes: readonly RelationTypeConfig[];
}

// ============================================================================
// Built-in definitions
// ============================================================================

const typeOf = (fm: Frontmatter): string => (typeof fm.type === 'string' ? fm.type.trim() : '');

/** `type: person` or `organization`: exact (trimmed) match, as `ContactKind::from_type`. */
export function isContactType(fm: Frontmatter): boolean {
  const t = typeOf(fm);
  return t === 'person' || t === 'organization';
}

const isPerson = (fm: Frontmatter): boolean => typeOf(fm) === 'person';

/** OKF §10.2: the Attested Computation fields apply to that type only. */
export const ATTESTED_COMPUTATION_TYPE = 'Attested Computation';
const isAttested = (fm: Frontmatter): boolean => typeOf(fm) === ATTESTED_COMPUTATION_TYPE;

/** The `type` values mbr gives meaning to, offered before the vault's own. */
export const BUILTIN_TYPES: readonly EnumOption[] = [
  { value: 'slides', help: 'Presents the note as a slideshow (▶ or p).' },
  { value: 'flashcard', help: 'A study deck: each definition-list term is a card.' },
  { value: 'person', help: 'A contact: card at the top, relationship charts below.' },
  { value: 'organization', help: 'A company or group: contact card and org chart.' },
];

/** OKF §5.4: `status`; absent means `stable`. */
export const OKF_STATUSES: readonly EnumOption[] = [
  { value: 'draft', help: 'Work in progress.' },
  { value: 'stable', help: 'Current and reliable (the default when absent).' },
  { value: 'deprecated', help: 'Kept for reference; do not rely on it.' },
];

/** Suggested labels per labeled field (`docs/markdown/contacts.md`). */
export const CONTACT_LABELS: Readonly<Record<string, readonly string[]>> = {
  emails: ['home', 'work', 'other', 'school'],
  phones: ['mobile', 'main', 'home', 'work', 'home_fax', 'work_fax', 'pager', 'other'],
  urls: ['homepage', 'blog', 'work', 'profile'],
  social: ['linkedin', 'mastodon', 'github', 'bluesky', 'twitter', 'instagram', 'facebook'],
  im: ['signal', 'whatsapp', 'matrix', 'telegram', 'slack', 'discord'],
  addresses: ['home', 'work', 'other'],
  // `WELL_KNOWN_DATES` in contact.rs, in display order.
  dates: ['birthday', 'death', 'anniversary'],
  // `alias_phrases` in contact.rs phrases these specially.
  aliases: ['maiden_name', 'nickname', 'formerly', 'birth_name'],
};

/** `ADDRESS_FIELDS` in contact.rs: the parts of a structured address. */
const ADDRESS_CHILDREN: readonly FieldDef[] = [
  { key: 'street', kind: 'longtext', label: 'Street', group: 'contact', help: 'May span several lines.' },
  { key: 'city', kind: 'text', label: 'City', group: 'contact' },
  { key: 'region', kind: 'text', label: 'Region', group: 'contact' },
  { key: 'postcode', kind: 'text', label: 'Postcode', group: 'contact', help: 'Quote it ("75010") so YAML keeps it as text.' },
  { key: 'country', kind: 'text', label: 'Country', group: 'contact' },
  { key: 'country_code', kind: 'text', label: 'Country code', group: 'contact' },
];

const text = (key: string, label: string, group: FieldGroup, extra: Partial<FieldDef> = {}): FieldDef => ({ key, kind: 'text', label, group, ...extra });

const datetime = (key: string, label: string, group: FieldGroup, extra: Partial<FieldDef> = {}): FieldDef => ({
  key,
  kind: 'datetime',
  label,
  group,
  help: 'ISO 8601 with an explicit offset, e.g. 2026-06-30T14:00:00Z.',
  ...extra,
});

const actor = (key: string, label: string, extra: Partial<FieldDef> = {}): FieldDef => ({
  key,
  kind: 'actor',
  label,
  group: 'okf',
  help: 'human:<id>, process:<id>, or <agent>/<version> (OKF §7).',
  ...extra,
});

const usageWindow = (group: FieldGroup): FieldDef => ({
  key: 'usage_window',
  kind: 'map',
  label: 'Usage window',
  group,
  help: 'The period usage counts cover.',
  children: [datetime('from', 'From', group), datetime('to', 'To', group)],
});

/** A labeled multi-value contact field (`labeled_values` in contact.rs: map, list or single value). */
function labeled(key: string, label: string, item: FieldDef, help: string): FieldDef {
  return { key, kind: 'labeled', label, group: 'contact', when: isContactType, labels: CONTACT_LABELS[key], item, help };
}

function relationshipsField(types: readonly RelationTypeConfig[]): FieldDef {
  const typeOptions: EnumOption[] = types.map((t) => ({ value: t.name, help: t.label + (t.category ? ` (${t.category})` : '') }));
  return {
    key: 'relationships',
    kind: 'relationships',
    label: 'Relationships',
    group: 'mbr',
    help: '`type` plus `to: [[Note]]` reads "Note is my <type>"; `from` reads "I am Note’s <type>". Other keys are edge attributes.',
    item: {
      key: '',
      kind: 'map',
      label: 'Relationship',
      group: 'mbr',
      children: [
        { key: 'type', kind: 'enum', label: 'Type', group: 'mbr', required: true, enumValues: typeOptions },
        { key: 'to', kind: 'wikilink', label: 'To', group: 'mbr', help: 'The object: "Note is my <type>".' },
        { key: 'from', kind: 'wikilink', label: 'From', group: 'mbr', help: 'The subject: "I am Note’s <type>".' },
        text('label', 'Label', 'mbr', { help: 'Shown instead of the type’s own label.' }),
      ],
    },
  };
}

function tagFields(sources: readonly TagSourceLike[] | undefined): FieldDef[] {
  const list = sources && sources.length > 0 ? sources : [{ field: 'tags' }];
  return list.map((s) => ({
    key: s.field,
    kind: 'tags' as const,
    label: s.label_plural ?? (s.field === 'tags' ? 'Tags' : humanizeKey(s.field.split('.').pop() ?? s.field)),
    group: 'mbr' as const,
    help: s.field === 'tags' ? 'Tags link to tag pages (also OKF’s `tags`).' : `A configured tag source (${s.field}).`,
    item: text('', s.label ?? 'Tag', 'mbr'),
  }));
}

/** `snake_case` → "Snake case", for keys with no built-in label. */
export function humanizeKey(key: string): string {
  const spaced = key.replace(/[_-]+/g, ' ').trim();
  return spaced ? spaced[0].toUpperCase() + spaced.slice(1) : key;
}

/** The built-in catalog for a vault. Order is the "+ Add property" order within a group. */
export function builtinCatalog(opts: CatalogOptions = {}): FieldCatalog {
  const relationshipTypes = opts.relationshipTypes ?? [];
  const sourceItem: FieldDef = {
    key: '',
    kind: 'map',
    label: 'Source',
    group: 'okf',
    children: [
      text('resource', 'Resource', 'okf', { required: true, help: 'URL, path, or scope descriptor.' }),
      text('id', 'ID', 'okf', { help: 'Stable key for footnote attribution.' }),
      text('title', 'Title', 'okf'),
      actor('author', 'Author'),
      { key: 'usage_count', kind: 'number', label: 'Usage count', group: 'okf', help: 'Exercises over the usage window.' },
      datetime('last_modified', 'Last modified', 'okf'),
      usageWindow('okf'),
    ],
  };
  const verification: FieldDef = {
    key: '',
    kind: 'map',
    label: 'Verification',
    group: 'okf',
    // OKF v0.2 §5.3 marks neither part required; a review names its reviewer, so `by` is hinted.
    children: [actor('by', 'By', { required: true }), datetime('at', 'At', 'okf', { seed: true })],
  };

  const fields: FieldDef[] = [
    // --- mbr ---------------------------------------------------------------
    text('title', 'Title', 'mbr', { help: 'The page title (also OKF’s `title`).' }),
    { key: 'description', kind: 'longtext', label: 'Description', group: 'mbr', help: 'One-sentence summary (OKF).' },
    { key: 'summary', kind: 'longtext', label: 'Summary', group: 'mbr' },
    {
      key: 'type',
      kind: 'enum',
      label: 'Type',
      group: 'mbr',
      enumValues: BUILTIN_TYPES,
      help: 'What kind of note this is. Any value is allowed (OKF types are open-ended); it also becomes a class on the page body.',
    },
    ...tagFields(opts.tagSources),
    {
      // `parse_aliases` reads the labeled shapes: names, `label: name` items, or a map.
      key: 'aliases',
      kind: 'labeled',
      label: 'Aliases',
      group: 'mbr',
      help: 'Other names; links and relationships naming one resolve to this note. A label (maiden_name, nickname) is optional.',
      labels: CONTACT_LABELS.aliases,
      item: text('', 'Alias', 'mbr'),
    },
    { key: 'date', kind: 'date', label: 'Date', group: 'mbr' },
    { key: 'order', kind: 'number', label: 'Order', group: 'mbr', help: 'Sort position when the listing sorts by `order`.' },
    { key: 'pinned', kind: 'bool', label: 'Pinned', group: 'mbr', help: 'Sorts first when the listing sorts by `pinned`.' },
    relationshipsField(relationshipTypes),

    // --- contacts (contact.rs `Contact::from_map`) ---------------------------
    text('prefix', 'Prefix', 'contact', { when: isPerson }),
    text('first_name', 'First name', 'contact', { when: isPerson }),
    text('middle_name', 'Middle name', 'contact', { when: isPerson }),
    text('last_name', 'Last name', 'contact', { when: isPerson }),
    text('suffix', 'Suffix', 'contact', { when: isPerson }),
    {
      key: 'company',
      kind: 'wikilink',
      label: 'Company',
      group: 'contact',
      when: isContactType,
      help: 'Plain text, or [[Organization]]: a wikilink on a person also makes an `employer` relationship.',
    },
    text('department', 'Department', 'contact', { when: isContactType }),
    text('job_title', 'Job title', 'contact', { when: isContactType }),
    text('image', 'Image', 'contact', { when: isContactType, help: 'Portrait or logo; a relative path resolves from the note’s folder.' }),
    text('gender', 'Gender', 'contact', { when: isPerson, help: 'Free text; tints the avatar and the charts.' }),
    labeled('emails', 'Emails', text('', 'Email', 'contact'), 'Kept out of site.json.'),
    labeled('phones', 'Phones', text('', 'Phone', 'contact'), 'Quote numbers starting with + so YAML keeps them as text.'),
    labeled('urls', 'URLs', text('', 'URL', 'contact'), 'Links only when http(s).'),
    labeled('social', 'Social', text('', 'Profile', 'contact'), 'Label with the service name.'),
    labeled('im', 'Messaging', text('', 'Handle', 'contact'), 'Label with the service name.'),
    labeled(
      'addresses',
      'Addresses',
      { key: '', kind: 'map', label: 'Address', group: 'contact', children: ADDRESS_CHILDREN },
      'Each a map of parts, or a preformatted string.',
    ),
    {
      key: 'dates',
      kind: 'labeled',
      label: 'Dates',
      group: 'contact',
      when: isContactType,
      labels: CONTACT_LABELS.dates,
      item: { key: '', kind: 'partialDate', label: 'Date', group: 'contact' },
      help: 'Label → date. Partial dates are fine: 1927-03-19, 1927-03, 1927, --03-19.',
    },
    text('born_place', 'Birthplace', 'contact', { when: isPerson }),
    text('died_place', 'Place of death', 'contact', { when: isPerson }),
    { key: 'born', kind: 'partialDate', label: 'Born', group: 'contact', when: isPerson, deprecated: 'dates.birthday' },
    { key: 'died', kind: 'partialDate', label: 'Died', group: 'contact', when: isPerson, deprecated: 'dates.death' },

    // --- OKF v0.2 -------------------------------------------------------------
    {
      key: 'status',
      kind: 'enum',
      label: 'Status',
      group: 'okf',
      enumValues: OKF_STATUSES,
      help: 'OKF lifecycle; a note without one counts as stable.',
    },
    text('resource', 'Resource', 'okf', { help: 'What this note describes: a URL, a /path in the bundle, or a relative path.' }),
    { key: 'sources', kind: 'list', label: 'Sources', group: 'okf', help: 'Where this knowledge came from (OKF provenance).', item: sourceItem },
    usageWindow('okf'),
    {
      key: 'generated',
      kind: 'map',
      label: 'Generated',
      group: 'okf',
      help: 'Who or what wrote this note, and when.',
      // OKF v0.2 §5.2: `by` is required within `generated`, `at` is optional.
      children: [actor('by', 'By', { required: true }), datetime('at', 'At', 'okf', { seed: true })],
    },
    {
      key: 'verified',
      kind: 'list',
      label: 'Verified',
      group: 'okf',
      singleOrList: true,
      help: 'Reviews; a `human:` reviewer makes the note human-reviewed.',
      item: verification,
    },
    datetime('stale_after', 'Stale after', 'okf', { help: 'The note is stale from this instant on. ISO 8601 with an offset.' }),

    // --- OKF Attested Computation (§10.2) ------------------------------------
    text('runtime', 'Runtime', 'attested', {
      kind: 'enum',
      when: isAttested,
      required: true,
      enumValues: ['bigquery', 'postgres', 'dbt', 'python', 'Looker'].map((value) => ({ value })),
      help: 'Where the computation runs.',
    }),
    {
      key: 'parameters',
      kind: 'list',
      label: 'Parameters',
      group: 'attested',
      when: isAttested,
      item: {
        key: '',
        kind: 'map',
        label: 'Parameter',
        group: 'attested',
        children: [
          text('name', 'Name', 'attested', { required: true }),
          text('type', 'Type', 'attested'),
          { key: 'required', kind: 'bool', label: 'Required', group: 'attested' },
        ],
      },
    },
    text('computation', 'Computation', 'attested', { when: isAttested, help: 'Path to the computation; absent means the note’s # Computation fence.' }),
    {
      key: 'executor',
      kind: 'map',
      label: 'Executor',
      group: 'attested',
      when: isAttested,
      children: [
        text('resource', 'Resource', 'attested'),
        { key: 'receipt', kind: 'list', label: 'Receipt', group: 'attested', help: 'Field names the receipt records.', item: text('', 'Field', 'attested') },
      ],
    },
    {
      key: 'attester',
      kind: 'map',
      label: 'Attester',
      group: 'attested',
      when: isAttested,
      children: [text('resource', 'Resource', 'attested', { help: 'Path to the deterministic attesting code.' })],
    },
  ];
  return { fields, relationshipTypes };
}

// ============================================================================
// Resolution
// ============================================================================

type Path = readonly (string | number)[];

function applies(def: FieldDef, fm: Frontmatter): boolean {
  return !def.when || def.when(fm);
}

/** The definition for one step below `def` (not for labeled fields: see {@link fieldFor}). */
function childDef(def: FieldDef, seg: string | number): FieldDef | undefined {
  if (typeof seg === 'number') return def.item;
  // `verified: {by, at}` written as a single map reads as its item.
  if (def.singleOrList && def.item?.children) return def.item.children.find((c) => c.key === seg);
  return def.children?.find((c) => c.key === seg);
}

/**
 * Below a labeled field the path names an entry in one of its three shapes
 * (`labeled_values` in contact.rs): `{work: x}` (label), `[x, {work: y}]`
 * (index, then the one-key map's label) or `[{label: work, value: x}]`.
 * Those steps are skipped; what is left resolves against the item.
 */
function labeledRest(def: FieldDef, rest: readonly (string | number)[]): FieldDef | undefined {
  const item = def.item;
  if (!item) return undefined;
  const isChild = (seg: string | number | undefined) => typeof seg === 'string' && Boolean(item.children?.some((c) => c.key === seg));
  let i = 0;
  if (typeof rest[i] === 'number') {
    i++;
    if (rest[i] === 'label') return { key: 'label', kind: 'text', label: 'Label', group: def.group };
    if (rest[i] === 'value') i++;
    else if (typeof rest[i] === 'string' && !isChild(rest[i])) i++;
  } else if (typeof rest[i] === 'string') {
    i++;
  }
  let current: FieldDef | undefined = item;
  for (; i < rest.length && current; i++) current = childDef(current, rest[i]);
  return current;
}

/**
 * The built-in definition for the value at `path`, when there is one that
 * applies to this note. A dotted catalog key (`taxonomy.performers`) matches
 * the nested path it names.
 */
export function fieldFor(path: Path, frontmatter: Frontmatter, catalog: FieldCatalog): FieldDef | undefined {
  if (path.length === 0) return undefined;
  for (const def of catalog.fields) {
    if (!applies(def, frontmatter)) continue;
    const segs = def.key.split('.');
    if (segs.length > path.length || !segs.every((s, i) => path[i] === s)) continue;
    const rest = path.slice(segs.length);
    if (rest.length === 0) return def;
    let current: FieldDef | undefined;
    if (def.kind === 'labeled') current = labeledRest(def, rest);
    else {
      current = def;
      for (const seg of rest) {
        current = current && childDef(current, seg);
        if (!current) break;
      }
    }
    if (current) return current;
  }
  return undefined;
}

/** The kind a bare value suggests, when nothing else is known. */
export function kindOfValue(value: unknown): FieldKind {
  if (typeof value === 'boolean') return 'bool';
  if (typeof value === 'number') return 'number';
  if (Array.isArray(value)) return 'list';
  if (value && typeof value === 'object') return 'map';
  if (typeof value === 'string') {
    if (isDateTimeString(value)) return 'datetime';
    if (/^\d{4}-\d{2}-\d{2}$/.test(value)) return 'date';
    if (/^\[\[[^\]]+\]\]$/.test(value.trim())) return 'wikilink';
    if (value.includes('\n')) return 'longtext';
  }
  return 'text';
}

/** What `infer.ts` knows about a key (the shape the resolver needs). */
export interface InferredFieldLike {
  readonly key: string;
  readonly kind: FieldKind;
  readonly values: readonly { readonly value: string; readonly count: number }[];
}

/**
 * The definition the form uses for `path`: the built-in one, else the
 * vault's inferred one (top-level keys), else one made from the value.
 */
export function resolveField(
  path: Path,
  frontmatter: Frontmatter,
  catalog: FieldCatalog,
  inferred?: ReadonlyMap<string, InferredFieldLike>,
  value?: unknown,
): FieldDef {
  const builtin = fieldFor(path, frontmatter, catalog);
  if (builtin) return builtin;
  const last = path[path.length - 1];
  const key = typeof last === 'string' ? last : '';
  const dotted = path.filter((s): s is string => typeof s === 'string').join('.');
  const known = path.every((s) => typeof s === 'string') ? inferred?.get(dotted) : undefined;
  if (known) {
    return {
      key,
      kind: known.kind,
      label: humanizeKey(key),
      group: 'vault',
      enumValues: known.values.map((v) => ({ value: v.value })),
    };
  }
  return { key, kind: kindOfValue(value), label: typeof last === 'number' ? `#${last + 1}` : humanizeKey(key), group: 'vault' };
}

/** One "+ Add property" suggestion. */
export interface AddableField {
  readonly key: string;
  readonly label: string;
  readonly kind: FieldKind;
  readonly group: FieldGroup;
  readonly help?: string;
  /** Notes in the vault using the key (inferred keys only). */
  readonly count?: number;
}

const GROUP_ORDER: readonly FieldGroup[] = ['mbr', 'contact', 'attested', 'okf'];

/**
 * Keys to offer for a new top-level property, excluding those present:
 * built-ins that apply to this note (mbr, then contact and Attested
 * Computation fields when the type calls for them, then OKF), then keys the
 * vault uses, most used first. Deprecated spellings are not offered.
 */
export function addableFields(frontmatter: Frontmatter, catalog: FieldCatalog, inferred?: ReadonlyMap<string, InferredFieldLike & { count: number }>): AddableField[] {
  const present = new Set(Object.keys(frontmatter));
  // A dotted key may be written nested or as one flat key (`dates.birthday:`); either is taken.
  const taken = (key: string) => present.has(key) || (present.has(key.split('.')[0]) && hasDotted(frontmatter, key));
  const out: AddableField[] = [];
  const seen = new Set<string>();
  for (const group of GROUP_ORDER) {
    for (const def of catalog.fields) {
      if (def.group !== group || def.deprecated || !applies(def, frontmatter) || taken(def.key) || seen.has(def.key)) continue;
      seen.add(def.key);
      out.push({ key: def.key, label: def.label, kind: def.kind, group: def.group, help: def.help });
    }
  }
  const vault = [...(inferred?.values() ?? [])].filter((f) => !f.key.includes('.') && !seen.has(f.key) && !present.has(f.key));
  vault.sort((a, b) => b.count - a.count || a.key.localeCompare(b.key));
  for (const f of vault) out.push({ key: f.key, label: humanizeKey(f.key), kind: f.kind, group: 'vault', count: f.count });
  return out;
}

function hasDotted(fm: Frontmatter, dotted: string): boolean {
  let v: unknown = fm;
  for (const seg of dotted.split('.')) {
    if (!v || typeof v !== 'object' || Array.isArray(v) || !(seg in v)) return false;
    v = (v as Record<string, unknown>)[seg];
  }
  return true;
}

// ============================================================================
// Validation (soft hints)
// ============================================================================

export interface FieldHint {
  readonly path: readonly (string | number)[];
  readonly message: string;
}

const isBlank = (v: unknown): boolean => v === undefined || v === null || (typeof v === 'string' && v.trim() === '');

/**
 * Soft hints for the form, never blocking a save: an OKF `status` outside
 * its enum, a required value missing (a source without `resource`, an
 * Attested Computation without `runtime`, a relationship without `type`), a
 * timestamp without a UTC offset, a relationship type the registry does not
 * know (only when there is a registry).
 */
export function validate(frontmatter: Frontmatter, catalog: FieldCatalog): FieldHint[] {
  const hints: FieldHint[] = [];
  const registry = new Set(catalog.relationshipTypes.map((t) => t.name.toLowerCase()));

  const visit = (def: FieldDef, value: unknown, path: (string | number)[]): void => {
    if (value === undefined) return;
    if (def.key === 'status' && def.group === 'okf' && typeof value === 'string' && !OKF_STATUSES.some((s) => s.value === value)) {
      hints.push({ path, message: `OKF status is one of draft, stable or deprecated; “${value}” is not.` });
    }
    if (def.kind === 'datetime' && typeof value === 'string' && value.trim() !== '' && !hasUtcOffset(value)) {
      hints.push({ path, message: 'Timestamps should carry a UTC offset (Z or ±HH:MM).' });
    }
    if (def.kind === 'relationships' && Array.isArray(value)) {
      value.forEach((edge, i) => {
        const type = edge && typeof edge === 'object' ? (edge as Record<string, unknown>).type : undefined;
        if (typeof type === 'string' && type.trim() && registry.size > 0 && !registry.has(type.trim().toLowerCase())) {
          hints.push({ path: [...path, i, 'type'], message: `“${type}” is not in relationship_types; no inverse edge is derived.` });
        }
      });
    }
    const items = Array.isArray(value) ? value.map((v, i) => [i, v] as const) : def.singleOrList && value && typeof value === 'object' ? [[null, value] as const] : [];
    if (def.item && (def.kind === 'list' || def.kind === 'relationships')) {
      for (const [i, v] of items) visit(def.item, v, i === null ? path : [...path, i]);
    }
    const children = def.children ?? (def.kind === 'map' && def.item?.children) ?? undefined;
    if (children && value && typeof value === 'object' && !Array.isArray(value)) {
      const obj = value as Record<string, unknown>;
      for (const c of children) {
        if (c.required && isBlank(obj[c.key])) hints.push({ path: [...path, c.key], message: `${c.label} is required here.` });
        visit(c, obj[c.key], [...path, c.key]);
      }
    }
  };

  for (const def of catalog.fields) {
    if (!applies(def, frontmatter)) continue;
    const segs = def.key.split('.');
    let v: unknown = frontmatter;
    for (const s of segs) v = v && typeof v === 'object' && !Array.isArray(v) ? (v as Record<string, unknown>)[s] : undefined;
    if (def.required && isBlank(v)) {
      hints.push({ path: segs, message: `${def.label} is required for ${typeOf(frontmatter) || 'this type'}.` });
      continue;
    }
    visit(def, v, segs);
  }
  return hints;
}

// ============================================================================
// Dates and times
// ============================================================================

const DATETIME = /^(\d{4}-\d{2}-\d{2})([Tt ])(\d{2}):(\d{2})(?::(\d{2})(\.\d+)?)?\s*(Z|z|[+-]\d{2}(?::?\d{2})?)?$/;

/** A date with a time of day (ISO 8601 / RFC 3339 / YAML timestamp spelling). */
export function isDateTimeString(s: string): boolean {
  return DATETIME.test(s.trim());
}

/** Whether a timestamp names its offset (`Z`, `+02:00`, `-0600`). */
export function hasUtcOffset(s: string): boolean {
  const m = DATETIME.exec(s.trim());
  return Boolean(m && m[7]);
}

/** A stored timestamp's parts. */
export interface DateTimeParts {
  readonly date: string;
  readonly separator: string;
  readonly hour: string;
  readonly minute: string;
  readonly second?: string;
  readonly fraction?: string;
  readonly offset?: string;
}

export function parseDateTime(s: string): DateTimeParts | null {
  const m = DATETIME.exec(s.trim());
  if (!m) return null;
  return { date: m[1], separator: m[2], hour: m[3], minute: m[4], second: m[5], fraction: m[6], offset: m[7] };
}

/** `minutes` east of UTC as `Z` or `±HH:MM`. */
export function formatOffset(minutes: number): string {
  if (minutes === 0) return 'Z';
  const sign = minutes > 0 ? '+' : '-';
  const abs = Math.abs(minutes);
  return `${sign}${String(Math.floor(abs / 60)).padStart(2, '0')}:${String(abs % 60).padStart(2, '0')}`;
}

/** The local offset for `date`, minutes east of UTC. */
export function localOffsetMinutes(date: Date = new Date()): number {
  return -date.getTimezoneOffset();
}

/**
 * The value for an `<input type="datetime-local">` from a stored timestamp:
 * its own wall-clock time (not converted), seconds kept when written.
 */
export function datetimeLocalValue(stored: string): string {
  const p = parseDateTime(stored);
  if (!p) return /^\d{4}-\d{2}-\d{2}$/.test(stored.trim()) ? `${stored.trim()}T00:00` : '';
  return `${p.date}T${p.hour}:${p.minute}${p.second ? `:${p.second}` : ''}`;
}

/**
 * The timestamp to store for a `datetime-local` input value, keeping the
 * previous value's format: its separator, whether it had seconds, and its
 * offset (the wall time typed is read in that offset). A value that had no
 * offset, or a new one, takes `offsetMinutes` (the local offset by default),
 * since OKF requires one. New values are written `YYYY-MM-DDTHH:MM:SS±HH:MM`
 * (`Z` at UTC). Returns `null` for an input value that is not a date-time.
 */
export function applyDatetimeLocal(input: string, previous?: string, offsetMinutes?: number): string | null {
  const m = /^(\d{4}-\d{2}-\d{2})T(\d{2}):(\d{2})(?::(\d{2})(\.\d+)?)?$/.exec(input.trim());
  if (!m) return null;
  const prev = previous ? parseDateTime(previous) : null;
  const separator = prev?.separator ?? 'T';
  const withSeconds = prev ? prev.second !== undefined : true;
  const second = m[4] ?? '00';
  const offset = prev?.offset ?? formatOffset(offsetMinutes ?? localOffsetMinutes(new Date(`${m[1]}T${m[2]}:${m[3]}:00`)));
  return `${m[1]}${separator}${m[2]}:${m[3]}${withSeconds ? `:${second}${m[5] ?? ''}` : ''}${offset}`;
}

/** `date` as a timestamp in its local offset, to the second (`2026-10-21T14:05:09-06:00`). */
export function timestampNow(date: Date = new Date()): string {
  const pad = (n: number) => String(n).padStart(2, '0');
  const day = `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
  const time = `${pad(date.getHours())}:${pad(date.getMinutes())}:${pad(date.getSeconds())}`;
  return `${day}T${time}${formatOffset(localOffsetMinutes(date))}`;
}

/** A date with any of year, month and day missing (contact.rs `PartialDate`). */
export interface PartialDate {
  readonly year?: number;
  readonly month?: number;
  readonly day?: number;
}

const DAYS_IN_MONTH = [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

const digits = (s: string, n: number): number | undefined => (s.length === n && /^\d+$/.test(s) ? Number(s) : undefined);
const monthOf = (s: string): number | undefined => {
  const m = digits(s, 2);
  return m !== undefined && m >= 1 && m <= 12 ? m : undefined;
};
const dayOf = (s: string, month: number): number | undefined => {
  const d = digits(s, 2);
  return d !== undefined && d >= 1 && d <= DAYS_IN_MONTH[month - 1] ? d : undefined;
};

/**
 * `PartialDate::parse`: `YYYY-MM-DD`, `YYYY-MM`, `YYYY`, `MM-DD` and
 * `--MM-DD`, zero-padded and in range (February allows the 29th). A number
 * (YAML read `1927` as one) is its digits. Anything else is `null`, and the
 * caller keeps the authored text.
 */
export function parsePartialDate(input: unknown): PartialDate | null {
  if (typeof input === 'number' && Number.isInteger(input)) return parsePartialDate(String(input));
  if (typeof input !== 'string') return null;
  const s = input.trim();
  const monthDay = (md: string): PartialDate | null => {
    const parts = md.split('-');
    if (parts.length !== 2) return null;
    const month = monthOf(parts[0]);
    const day = month === undefined ? undefined : dayOf(parts[1], month);
    return month !== undefined && day !== undefined ? { month, day } : null;
  };
  if (s.startsWith('--')) return monthDay(s.slice(2));
  const parts = s.split('-');
  if (parts.length === 3) {
    const year = digits(parts[0], 4);
    const month = monthOf(parts[1]);
    const day = month === undefined ? undefined : dayOf(parts[2], month);
    return year !== undefined && day !== undefined ? { year, month, day } : null;
  }
  if (parts.length === 2) {
    if (parts[0].length === 4) {
      const year = digits(parts[0], 4);
      const month = monthOf(parts[1]);
      return year !== undefined && month !== undefined ? { year, month } : null;
    }
    return monthDay(s);
  }
  if (parts.length === 1) {
    const year = digits(parts[0], 4);
    return year !== undefined ? { year } : null;
  }
  return null;
}

/**
 * The stored form of a partial date (`PartialDate::iso`: `YYYY-MM-DD`,
 * `YYYY-MM`, `YYYY`, `--MM-DD`), or `null` for a combination that is not one
 * (a day without a month, a year and day without a month). A yearless date
 * keeps the `MM-DD` spelling when `previous` used it.
 */
export function formatPartialDate(p: PartialDate, previous?: unknown): string | null {
  const pad = (n: number, w: number) => String(n).padStart(w, '0');
  const { year, month, day } = p;
  if (year !== undefined && (year < 0 || year > 9999)) return null;
  if (month !== undefined && (month < 1 || month > 12)) return null;
  if (day !== undefined && (month === undefined || day < 1 || day > DAYS_IN_MONTH[month - 1])) return null;
  if (year !== undefined && month !== undefined && day !== undefined) return `${pad(year, 4)}-${pad(month, 2)}-${pad(day, 2)}`;
  if (year !== undefined && month !== undefined) return `${pad(year, 4)}-${pad(month, 2)}`;
  if (year !== undefined && day === undefined && month === undefined) return pad(year, 4);
  if (year === undefined && month !== undefined && day !== undefined) {
    const bare = typeof previous === 'string' && /^\d{2}-\d{2}$/.test(previous.trim());
    return `${bare ? '' : '--'}${pad(month, 2)}-${pad(day, 2)}`;
  }
  return null;
}

const MONTH_NAMES = ['January', 'February', 'March', 'April', 'May', 'June', 'July', 'August', 'September', 'October', 'November', 'December'];

/** `PartialDate::display`: "March 19, 1927", "March 1927", "1927", "March 19". */
export function displayPartialDate(p: PartialDate): string {
  const month = p.month !== undefined ? MONTH_NAMES[p.month - 1] : undefined;
  if (p.year !== undefined && month && p.day !== undefined) return `${month} ${p.day}, ${p.year}`;
  if (p.year !== undefined && month) return `${month} ${p.year}`;
  if (p.year !== undefined) return String(p.year);
  if (month && p.day !== undefined) return `${month} ${p.day}`;
  return '';
}
