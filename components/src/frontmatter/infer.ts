/**
 * Field knowledge inferred from the vault: which keys its notes use, how
 * often, what kind of value each usually holds, and its most common values.
 *
 * Read from `site.json`'s `markdown_files[].frontmatter`, which is the
 * *simplified* frontmatter: nested maps flattened to dot keys
 * (`dates.birthday`), lists of maps reduced to `[]`, and contact details
 * (`emails`, `phones`, …) left out. So this sees top-level keys and scalar
 * lists well and nested structure barely; nested values are completed from
 * the built-in catalog instead (`fields.ts`).
 *
 * Pure. Computed once per editor open; the caller memoizes.
 */

import { kindOfValue, type FieldKind, type InferredFieldLike } from './fields.js';

/** A `site.json` `markdown_files` entry, as far as inference reads it. */
export interface SiteFileLike {
  readonly frontmatter?: Readonly<Record<string, unknown>> | null;
}

export interface ValueCount {
  readonly value: string;
  readonly count: number;
}

export interface InferredField extends InferredFieldLike {
  /** The key; nested keys are dotted (`dates.birthday`). */
  readonly key: string;
  /** Notes using the key. */
  readonly count: number;
  /** The most common kind of value. */
  readonly kind: FieldKind;
  /** Notes per kind of value. */
  readonly kinds: Readonly<Partial<Record<FieldKind, number>>>;
  /** The most common values (list items counted one by one), most used first. */
  readonly values: readonly ValueCount[];
}

export interface InferOptions {
  /** How many values to keep per key (default 20). */
  readonly topValues?: number;
  /** Tag fields (`tagSources[].field`): their lists are `tags`, not `list`. */
  readonly tagFields?: readonly string[];
}

/** Longer values are not suggestions: nobody picks a paragraph from a list. */
const MAX_SUGGESTED_LENGTH = 80;

interface Acc {
  count: number;
  kinds: Map<FieldKind, number>;
  values: Map<string, number>;
}

function suggestion(v: unknown): string | null {
  if (typeof v === 'string') {
    const t = v.trim();
    return t && t.length <= MAX_SUGGESTED_LENGTH && !t.includes('\n') ? t : null;
  }
  if (typeof v === 'number' || typeof v === 'boolean') return String(v);
  return null;
}

/**
 * Key statistics over every note's frontmatter, keyed by (dotted) key, in
 * descending order of use. Objects that did arrive nested are walked into
 * dotted keys as well, so either `site.json` shape gives the same keys.
 */
export function inferCatalog(siteFiles: readonly SiteFileLike[], opts: InferOptions = {}): Map<string, InferredField> {
  const topValues = opts.topValues ?? 20;
  const tagFields = new Set(opts.tagFields ?? ['tags']);
  const acc = new Map<string, Acc>();

  const record = (key: string, value: unknown): void => {
    let a = acc.get(key);
    if (!a) {
      a = { count: 0, kinds: new Map(), values: new Map() };
      acc.set(key, a);
    }
    a.count++;
    let kind = kindOfValue(value);
    if (kind === 'list' && tagFields.has(key)) kind = 'tags';
    a.kinds.set(kind, (a.kinds.get(kind) ?? 0) + 1);
    const items = Array.isArray(value) ? value : [value];
    for (const item of items) {
      const s = suggestion(item);
      if (s !== null) a.values.set(s, (a.values.get(s) ?? 0) + 1);
    }
  };

  const walk = (obj: Readonly<Record<string, unknown>>, prefix: string): void => {
    for (const [k, v] of Object.entries(obj)) {
      const key = prefix ? `${prefix}.${k}` : k;
      if (v && typeof v === 'object' && !Array.isArray(v)) {
        record(key, v);
        walk(v as Record<string, unknown>, key);
      } else {
        record(key, v);
      }
    }
  };

  for (const file of siteFiles) {
    if (file.frontmatter && typeof file.frontmatter === 'object') walk(file.frontmatter, '');
  }

  const out: InferredField[] = [...acc].map(([key, a]) => {
    const kinds = [...a.kinds].sort((x, y) => y[1] - x[1] || x[0].localeCompare(y[0]));
    const values = [...a.values]
      .sort((x, y) => y[1] - x[1] || x[0].localeCompare(y[0]))
      .slice(0, topValues)
      .map(([value, count]) => ({ value, count }));
    return { key, count: a.count, kind: kinds[0]?.[0] ?? 'text', kinds: Object.fromEntries(kinds), values };
  });
  out.sort((x, y) => y.count - x.count || x.key.localeCompare(y.key));
  return new Map(out.map((f) => [f.key, f]));
}

/** The vault's `type` values, most used first. */
export function vaultTypes(inferred: ReadonlyMap<string, InferredField>): ValueCount[] {
  return [...(inferred.get('type')?.values ?? [])];
}

/** Keys whose values are OKF actors (`fields.ts`'s `actor` fields), dotted. */
const ACTOR_KEYS = ['generated.by', 'verified.by', 'sources.author'];

/** OKF §7 actor spellings: `human:<id>`, `process:<id>`, `<producer>/<version>`. */
export function isActor(value: string): boolean {
  return /^(human|process):\S+$/.test(value) || /^[^\s/:]+\/\S+$/.test(value);
}

/**
 * Actors the vault already uses: values of the actor keys, plus any value
 * anywhere that is spelled `human:`/`process:`. Most used first, deduplicated.
 */
export function vaultActors(inferred: ReadonlyMap<string, InferredField>): ValueCount[] {
  const counts = new Map<string, number>();
  for (const [key, field] of inferred) {
    const actorKey = ACTOR_KEYS.includes(key);
    for (const v of field.values) {
      if (actorKey ? isActor(v.value) : /^(human|process):\S+$/.test(v.value)) counts.set(v.value, (counts.get(v.value) ?? 0) + v.count);
    }
  }
  return [...counts].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0])).map(([value, count]) => ({ value, count }));
}
