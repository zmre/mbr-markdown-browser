/**
 * What the YAML views know about the frontmatter they edit: a CodeMirror
 * completion source (keys and values from the field catalog, the vault's
 * inferred values, tags, actors, relationship types and note links) and the
 * diagnostics the linter shows (the `yaml` package's parse errors, the same
 * parser the model uses, so the lint and the form agree on what is broken).
 *
 * No DOM: everything here runs on an `EditorState`, which is how the tests
 * drive it. `yaml-source.ts` puts it in an editor.
 */

import { startCompletion, type Completion, type CompletionContext, type CompletionResult } from '@codemirror/autocomplete';
import { ensureSyntaxTree, syntaxTree } from '@codemirror/language';
import type { Diagnostic } from '@codemirror/lint';
import type { EditorState } from '@codemirror/state';
import type { EditorView } from '@codemirror/view';
import { parseDocument } from 'yaml';
import { CONTACT_LABELS, addableFields, fieldFor, resolveField, timestampNow, type FieldCatalog, type FieldDef, type Frontmatter } from './fields.js';
import type { InferredField } from './infer.js';
import { valueSuggestions, type ComboOption } from './form-logic.js';
import {
  DEFAULT_RELATIONSHIP_TYPES,
  actorOptions,
  noteLinkOptions,
  relationTypeOptions,
  tagItems,
  tagSuggestions,
  type ActorMemory,
} from './mbr-logic.js';
import { buildLinkTargets, type LinkTargetSite } from '../link-targets.js';
import { unresolvedAlias, type FmPath, type FmValue } from './model.js';
import { yamlContextAt, type YamlContext } from './yaml-context.js';

/** The vault knowledge completion draws on; read on every request, since site.json arrives late. */
export interface YamlKnowledge {
  readonly catalog: FieldCatalog;
  readonly inferred: ReadonlyMap<string, InferredField>;
  readonly siteFiles: readonly LinkTargetSite[];
  readonly actorMemory?: ActorMemory;
}

export interface YamlAssistOptions {
  readonly knowledge: () => YamlKnowledge;
  /**
   * Where the edited text sits in the note's frontmatter: `[]` for the whole
   * block (the YAML tab), the value's path for "Edit as YAML" on one value.
   */
  readonly basePath?: FmPath;
  /**
   * The note's whole frontmatter, for a value edited on its own (which
   * fields apply depends on `type`, which is outside the slice). By default
   * the edited text is the whole frontmatter.
   */
  readonly frontmatter?: () => Frontmatter;
}

// ============================================================================
// Reading the text being edited
// ============================================================================

type Obj = { [key: string]: FmValue };

const isObj = (v: unknown): v is Obj => v !== null && typeof v === 'object' && !Array.isArray(v);

/** The value of YAML that may not parse (what does parse is still read). */
function looseValue(text: string): FmValue | undefined {
  try {
    return parseDocument(text).toJS() as FmValue | undefined;
  } catch {
    return undefined;
  }
}

function getAt(value: FmValue | undefined, path: FmPath): FmValue | undefined {
  let v = value;
  for (const seg of path) {
    if (Array.isArray(v) && typeof seg === 'number') v = v[seg];
    else if (isObj(v) && typeof seg === 'string' && Object.prototype.hasOwnProperty.call(v, seg)) v = v[seg];
    else return undefined;
  }
  return v;
}

/** `root` with `value` at `path` (copies along the path; a missing step becomes a map). */
function setAt(root: FmValue | undefined, path: FmPath, value: FmValue | undefined): FmValue | undefined {
  if (path.length === 0) return value;
  const [head, ...rest] = path;
  if (Array.isArray(root) && typeof head === 'number') {
    const copy = root.slice();
    copy[head] = setAt(copy[head], rest, value) ?? null;
    return copy;
  }
  const obj: Obj = isObj(root) ? { ...root } : {};
  obj[String(head)] = setAt(obj[String(head)], rest, value) ?? null;
  return obj;
}

/** The note's frontmatter with the edited text in its place. */
function noteFrontmatter(text: string, opts: YamlAssistOptions): Frontmatter {
  const own = looseValue(text);
  const base = opts.basePath ?? [];
  if (base.length === 0) return isObj(own) ? own : {};
  const whole = setAt(opts.frontmatter?.() as FmValue | undefined, base, own);
  return isObj(whole) ? whole : {};
}

// ============================================================================
// Suggestions
// ============================================================================

const plural = (n: number, word: string) => `${n} ${word}${n === 1 ? '' : 's'}`;

function withoutKey(fm: Frontmatter, key: string): Frontmatter {
  const copy = { ...fm };
  delete (copy as Record<string, unknown>)[key];
  return copy;
}

const strings = (path: FmPath): string[] | null => (path.every((s): s is string => typeof s === 'string') ? (path as string[]) : null);

/** The labeled field (`phones`, `dates`, `aliases`…) whose entries live in the map or item at `path`. */
function labeledAt(path: FmPath, fm: Frontmatter, catalog: FieldCatalog): FieldDef | undefined {
  const own = fieldFor(path, fm, catalog);
  if (own?.kind === 'labeled') return own;
  const parent = path.length > 1 && typeof path[path.length - 1] === 'number' ? fieldFor(path.slice(0, -1), fm, catalog) : undefined;
  return parent?.kind === 'labeled' ? parent : undefined;
}

/** The known keys of the map at `path`. */
function childFields(def: FieldDef | undefined): readonly FieldDef[] {
  if (!def) return [];
  return def.children ?? (def.singleOrList || def.kind === 'map' ? (def.item?.children ?? []) : []);
}

const LIST_KINDS = new Set(['list', 'relationships']);

/** What follows a key a completion inserts: the start of its value, in the shape the field takes. */
function afterKey(def: FieldDef | undefined, column: number): { text: string; nested: boolean } {
  const pad = ' '.repeat(column + 2);
  if (def && (LIST_KINDS.has(def.kind) || def.singleOrList)) return { text: `:\n${pad}- `, nested: true };
  if (def && (def.kind === 'map' || (def.kind === 'labeled' && def.key !== 'aliases'))) return { text: `:\n${pad}`, nested: true };
  return { text: ': ', nested: false };
}

interface KeyOption {
  readonly key: string;
  readonly detail?: string;
  readonly info?: string;
  readonly def?: FieldDef;
}

/**
 * The keys to offer in the map at `path` (full path in the note), without
 * those it already has, except `editing` (the key under the cursor).
 */
export function keyOptions(path: FmPath, fm: Frontmatter, k: YamlKnowledge, editing = ''): KeyOption[] {
  const present = new Set(Object.keys(isObj(getAt(fm as FmValue, path)) ? (getAt(fm as FmValue, path) as Obj) : {}));
  present.delete(editing);
  // The catalog's own "is it there yet" check, for the key being retyped.
  const fmForAdd: Frontmatter = editing && Object.prototype.hasOwnProperty.call(fm, editing) && path.length === 0 ? withoutKey(fm, editing) : fm;
  const out: KeyOption[] = [];
  const seen = new Set<string>();
  const push = (o: KeyOption) => {
    if (seen.has(o.key) || present.has(o.key) || o.key === '') return;
    seen.add(o.key);
    out.push(o);
  };
  if (path.length === 0) {
    for (const f of addableFields(fmForAdd, k.catalog, k.inferred)) {
      // A dotted catalog key (`taxonomy.tags`) is typed as its first key.
      const [first, ...rest] = f.key.split('.');
      const def = rest.length === 0 ? fieldFor([first], fm, k.catalog) : undefined;
      const detail = f.count ? `${f.label} · ${plural(f.count, 'note')}` : rest.length > 0 ? f.key : f.label;
      push({ key: first, detail, info: f.help, def: rest.length === 0 ? def : { key: first, kind: 'map', label: first, group: 'vault' } });
    }
    // Nested keys the vault uses (`taxonomy.genre`), as the map holding them.
    const nested = new Map<string, number>();
    for (const f of k.inferred.values()) {
      const dot = f.key.indexOf('.');
      if (dot > 0) nested.set(f.key.slice(0, dot), Math.max(nested.get(f.key.slice(0, dot)) ?? 0, f.count));
    }
    for (const [key, count] of [...nested].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))) {
      push({ key, detail: plural(count, 'note'), def: { key, kind: 'map', label: key, group: 'vault' } });
    }
    return out;
  }
  const labeled = labeledAt(path, fm, k.catalog);
  const def = fieldFor(path, fm, k.catalog);
  for (const c of childFields(def)) push({ key: c.key, detail: c.label, info: c.help, def: c });
  if (labeled) {
    for (const label of labeled.labels ?? CONTACT_LABELS[labeled.key] ?? []) {
      push({ key: label, detail: `${labeled.label} label`, def: labeled.item });
    }
  }
  const dotted = strings(path);
  if (dotted) {
    const prefix = `${dotted.join('.')}.`;
    for (const f of k.inferred.values()) {
      if (!f.key.startsWith(prefix)) continue;
      const next = f.key.slice(prefix.length).split('.')[0];
      push({ key: next, detail: plural(f.count, 'note') });
    }
  }
  return out;
}

interface ValueOption extends ComboOption {
  /** Written as is, unquoted (numbers, booleans, a prefix to type on from). */
  readonly raw?: boolean;
  readonly info?: string;
}

/** Per site.json array: building them walks every note, and completion asks on every keystroke. */
function memoized<T>(fn: (files: readonly LinkTargetSite[]) => T): (files: readonly LinkTargetSite[]) => T {
  const memo = new WeakMap<readonly LinkTargetSite[], T>();
  return (files) => {
    if (!memo.has(files)) memo.set(files, fn(files));
    return memo.get(files)!;
  };
}
const linkOptions = memoized(noteLinkOptions);
const linkTargets = memoized(buildLinkTargets);

/** The tag field a value at `path` belongs to (the field itself, or one item of it). */
function tagFieldOf(path: FmPath, fm: Frontmatter, catalog: FieldCatalog): FmPath | null {
  if (fieldFor(path, fm, catalog)?.kind === 'tags') return path;
  const parent = path.slice(0, -1);
  return typeof path[path.length - 1] === 'number' && parent.length > 0 && fieldFor(parent, fm, catalog)?.kind === 'tags' ? parent : null;
}

/** The values to offer at `path` (full path in the note); `item` when it is a sequence item. */
export function valueOptions(path: FmPath, fm: Frontmatter, k: YamlKnowledge): ValueOption[] {
  const last = path[path.length - 1];
  const parentPath = path.slice(0, -1);
  const parentDef = parentPath.length > 0 ? fieldFor(parentPath, fm, k.catalog) : undefined;
  const own = fieldFor(path, fm, k.catalog);

  // Tags: a tag field written as a string, or one item of it.
  const tagField = tagFieldOf(path, fm, k.catalog);
  if (tagField) {
    const field = strings(tagField)?.join('.') ?? '';
    return tagSuggestions(field, k.catalog, k.inferred, tagItems(getAt(fm as FmValue, tagField)));
  }

  // A `{label: …, value: …}` entry's label.
  if (last === 'label') {
    const labeled = labeledAt(parentPath, fm, k.catalog);
    if (labeled) return (labeled.labels ?? []).map((value) => ({ value, detail: `${labeled.label} label` }));
  }

  const def = own ?? resolveField(path, fm, k.catalog, k.inferred, getAt(fm as FmValue, path));
  // The relationship type picker reads the registry, or mbr's defaults without one.
  if (def.key === 'type' && parentDef?.kind === 'map' && path[0] === 'relationships') {
    const registry = k.catalog.relationshipTypes.length > 0 ? k.catalog.relationshipTypes : DEFAULT_RELATIONSHIP_TYPES;
    return relationTypeOptions(registry);
  }

  const out: ValueOption[] = [];
  const seen = new Set<string>();
  const add = (o: ValueOption) => {
    if (seen.has(o.value)) return;
    seen.add(o.value);
    out.push(o);
  };
  // Actors lead with the reader's own id and the vault's (the prefixes are kept for last).
  const actors = def.kind === 'actor' ? actorOptions(k.inferred, k.actorMemory).map((o) => ({ ...o, raw: o.value.endsWith(':') })) : [];
  for (const o of actors) if (!o.raw) add(o);
  // An enum value's help is its info; a vault value's count is its detail.
  for (const o of valueSuggestions(path, def, k.inferred)) {
    const help = def.enumValues?.find((e) => e.value === o.value)?.help;
    add({ ...o, detail: help ? undefined : o.detail, info: help, raw: def.kind === 'number' || def.kind === 'bool' });
  }
  switch (def.kind) {
    case 'actor':
      for (const o of actors) add(o);
      break;
    case 'wikilink':
      for (const o of linkOptions(k.siteFiles)) add(o);
      break;
    case 'bool':
      for (const value of ['true', 'false']) add({ value, raw: true });
      break;
    case 'date':
      add({ value: timestampNow().slice(0, 10), detail: 'Today' });
      break;
    case 'datetime':
      add({ value: timestampNow(), detail: 'Now' });
      break;
    default:
      break;
  }
  return out;
}

// ============================================================================
// Writing a suggestion as YAML
// ============================================================================

/** Whether `value` reads back as that same string unquoted in this place. */
function plainReadsAs(value: string, flow: boolean): boolean {
  if (value === '' || value !== value.trim() || value.includes('\n')) return false;
  const doc = parseDocument(flow ? `[${value}]` : `k: ${value}`);
  if (doc.errors.length > 0) return false;
  const js = doc.toJS() as unknown;
  return flow ? Array.isArray(js) && js.length === 1 && js[0] === value : isObj(js) && js.k === value;
}

/** `value` as the text to put at `ctx`: escaped inside its quotes, else plain where that reads the same, else double-quoted. */
export function yamlScalarText(value: string, ctx: Pick<YamlContext, 'quote' | 'flow'>, raw = false): string {
  if (ctx.quote === '"') return JSON.stringify(value).slice(1, -1);
  if (ctx.quote === "'") return value.replace(/'/g, "''");
  if (raw || plainReadsAs(value, ctx.flow)) return value;
  return JSON.stringify(value);
}

function linkText(name: string, quote: '"' | "'" | null): string {
  const link = `[[${name}]]`;
  return quote === '"' ? JSON.stringify(link).slice(1, -1) : quote === "'" ? link.replace(/'/g, "''") : link;
}

/**
 * Where the key or value under the cursor ends, in the current text: the
 * rest of it is replaced too (`dr|aft` → `stable`, not `stableaft`). The
 * result's own `to` stays at the cursor, because CodeMirror filters on
 * `from`–`to`, so the end is found when the completion is applied, after any
 * typing since it was offered. `untilComma` stops at a comma (one tag of a
 * `tags: a, b` string).
 */
function tokenEnd(state: EditorState, pos: number, untilComma: boolean): number {
  const ctx = yamlContextAt(state.doc.toString(), ensureSyntaxTree(state, state.doc.length, 50) ?? syntaxTree(state), pos);
  if (!ctx || ctx.to < pos) return pos;
  if (!untilComma) return ctx.to;
  const comma = state.sliceDoc(pos, ctx.to).indexOf(',');
  return comma < 0 ? ctx.to : pos + comma;
}

/** Inserts `text` over `from` to the end of the token, and, when `reopen`, asks for completions again (the next key or value). */
function insert(text: string, reopen = false, untilComma = false) {
  return (view: EditorView, _c: Completion, from: number, to: number) => {
    const end = tokenEnd(view.state, to, untilComma);
    view.dispatch({ changes: { from, to: end, insert: text }, selection: { anchor: from + text.length }, scrollIntoView: true, userEvent: 'input.complete' });
    if (reopen) setTimeout(() => startCompletion(view), 0);
  };
}

// ============================================================================
// The completion source
// ============================================================================

/** A path made relative to the note (a slice's own paths start at its value). */
function fullPath(ctx: YamlContext, opts: YamlAssistOptions, fm: Frontmatter): { role: YamlContext['role']; path: FmPath } {
  const base = opts.basePath ?? [];
  const path = [...base, ...ctx.path];
  // A key typed at the top of a slice whose value is not a map is the value itself.
  if (base.length > 0 && ctx.path.length === 0 && ctx.role === 'key' && !ctx.flow) {
    const def = fieldFor(base, fm, opts.knowledge().catalog);
    const current = getAt(fm as FmValue, base);
    const mapLike = isObj(current) || def?.kind === 'map' || (def?.kind === 'labeled' && !Array.isArray(current));
    if (!mapLike) return { role: 'value', path };
  }
  return { role: ctx.role, path };
}

/** The completions at the cursor, or `null` where there are none. */
export function yamlCompletions(state: EditorState, pos: number, explicit: boolean, opts: YamlAssistOptions): CompletionResult | null {
  const text = state.doc.toString();
  const tree = ensureSyntaxTree(state, state.doc.length, 50) ?? syntaxTree(state);
  const ctx = yamlContextAt(text, tree, pos);
  if (!ctx) return null;
  if (!explicit && ctx.prefix === '' && ctx.role !== 'value' && !ctx.wikilink) {
    // Typing a space or a new line opens nothing on its own; Ctrl-Space does.
    if (state.sliceDoc(pos - 1, pos) !== ' ' || ctx.role !== 'item') return null;
  }
  const k = opts.knowledge();
  const fm = noteFrontmatter(text, opts);
  const { role, path } = fullPath(ctx, opts, fm);
  const options: Completion[] = [];

  if (ctx.wikilink) {
    const w = ctx.wikilink;
    linkTargets(k.siteFiles).forEach((t, i) => {
      options.push({
        label: `[[${t.wiki}]]`,
        displayLabel: t.title,
        detail: t.path,
        type: 'text',
        boost: -Math.min(99, i / 100),
        apply: insert(`${w.open}${linkText(t.wiki, w.quote)}${w.close}`, false),
      });
    });
    return { from: w.from, options, filter: true };
  }

  if (role === 'key' || role === 'item') {
    // An item may start a map (`- type: …`): offer its keys, unless it is a list of scalars.
    const editing = role === 'key' && ctx.hasColon ? state.sliceDoc(ctx.from, ctx.to) : '';
    const keys = role === 'key' || childFields(fieldFor(path, fm, k.catalog)).length > 0 || labeledAt(path, fm, k.catalog) ? keyOptions(path, fm, k, editing) : [];
    keys.forEach((o, i) => {
      const after = ctx.hasColon ? { text: '', nested: false } : afterKey(o.def, ctx.column);
      options.push({
        label: o.key,
        detail: o.detail,
        info: o.info,
        type: 'property',
        boost: Math.max(-99, 50 - i),
        apply: insert(o.key + after.text, !ctx.hasColon),
      });
    });
  }
  // `tags: a, b` is one string of tags: complete the one at the cursor.
  const commaTags = role === 'value' && !ctx.flow && tagFieldOf(path, fm, k.catalog) !== null;
  if (role === 'value' || role === 'item') {
    valueOptions(path, fm, k).forEach((o, i) => {
      options.push({
        label: o.value,
        displayLabel: o.label,
        detail: o.detail,
        info: o.info,
        type: 'constant',
        boost: Math.max(-99, 40 - i),
        apply: insert(yamlScalarText(o.value, ctx, o.raw), false, commaTags),
      });
    });
  }
  if (options.length === 0) return null;
  let from = ctx.from;
  if (commaTags) {
    const comma = ctx.prefix.lastIndexOf(',');
    if (comma >= 0) from += comma + 1 + (ctx.prefix.slice(comma + 1).length - ctx.prefix.slice(comma + 1).trimStart().length);
  }
  return { from, options, filter: true };
}

/** The CodeMirror completion source for frontmatter YAML. */
export function yamlCompletionSource(opts: YamlAssistOptions) {
  return (context: CompletionContext): CompletionResult | null => yamlCompletions(context.state, context.pos, context.explicit, opts);
}

// ============================================================================
// Lint
// ============================================================================

/**
 * The parse errors (and warnings) of `text` as diagnostics: the `yaml`
 * package's own spans, widened to one character where a span is empty so
 * the mark is visible, and pulled back inside the text at its end.
 */
export function yamlDiagnostics(text: string): Diagnostic[] {
  const doc = parseDocument(text, { prettyErrors: false });
  const len = text.length;
  const toDiagnostic = (e: { pos: [number, number]; message: string }, severity: Diagnostic['severity']): Diagnostic => {
    let from = Math.max(0, Math.min(e.pos[0], len));
    let to = Math.max(from, Math.min(e.pos[1], len));
    // An empty span, or one of only line breaks (an unclosed quote is
    // reported past the end), marks the last character before it instead.
    if (to > from && /^[\r\n]+$/.test(text.slice(from, to))) to = from;
    if (to === from) {
      if (from < len && !/[\r\n]/.test(text[from])) to = from + 1;
      else {
        while (from > 0 && /[\r\n]/.test(text[from - 1])) from--;
        to = from;
        if (from > 0) from -= 1;
      }
    }
    return { from, to, severity, message: e.message.split('\n')[0], source: 'YAML' };
  };
  const out = [...doc.errors.map((e) => toDiagnostic(e, 'error')), ...doc.warnings.map((w) => toDiagnostic(w, 'warning'))];
  const alias = out.length === 0 ? unresolvedAlias(doc) : null;
  if (alias) {
    const [start, end] = alias.range ?? [0, 0];
    out.push(toDiagnostic({ pos: [start, end], message: `The alias *${alias.source} names no anchor before it` }, 'error'));
  }
  return out;
}
