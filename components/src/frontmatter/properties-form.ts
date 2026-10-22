/**
 * `<mbr-fm-editor>`: the editor's frontmatter ("Properties") editor.
 *
 * Two views of one {@link FrontmatterModel}: a form (a row per key, typed
 * widgets, a generic tree for nested values) and the YAML source in a
 * CodeMirror editor (`yaml-source.ts`) with the same field knowledge as
 * completions. The YAML view sets the text wholesale: it is explicit source,
 * so what the user typed is what a save writes.
 * Every form edit is a `model.ts` operation, which patches only the edited
 * node's bytes, so a save writes the frontmatter as authored except for what
 * was changed; an untouched form leaves `text` exactly as it was given.
 *
 * Nothing the form cannot represent is lost or flattened: a value holding
 * anchors, aliases, tags or comments is shown read-only with "Edit as YAML"
 * (its own source, applied with `replaceSubtree`), and YAML that does not
 * parse makes the form read-only until it is fixed in the YAML view.
 *
 * Public API:
 * - properties `model` (or `text`), `catalog`, `inferred`, `siteFiles`, and
 *   `widgets` (the kind → renderer table; phase-3 widgets extend
 *   `GENERIC_WIDGETS`);
 * - `text` (getter): what a save writes;
 * - `openAddProperty()`; Cmd/Ctrl+; inside the form does the same;
 * - event {@link FM_CHANGE_EVENT} (`detail: { model, text }`) on every change.
 *
 * Light DOM, styled by `fm-styles.ts`.
 */

import { LitElement, html, nothing, type PropertyValues, type TemplateResult } from 'lit';
import { customElement, property, state } from 'lit/decorators.js';
import { ref } from 'lit/directives/ref.js';
import {
  FrontmatterEditError,
  addKey,
  addItem,
  entries,
  getIn,
  openFrontmatter,
  removeItem,
  removeKey,
  renameKey,
  replaceSubtree,
  setValue,
  sliceAt,
  toValue,
  type FmPath,
  type FmValue,
  type FrontmatterModel,
  type ModelEntry,
} from './model.js';
import {
  addableFields,
  builtinCatalog,
  resolveField,
  validate,
  type FieldCatalog,
  type FieldDef,
  type FieldHint,
  type Frontmatter,
} from './fields.js';
import type { InferredField, SiteFileLike } from './infer.js';
import {
  KIND_LABELS,
  addProperty,
  addToSingleOrList,
  existingPropertyPath,
  fits,
  itemSummary,
  newValueFor,
  pathKey,
  propertyPath,
  typeConversions,
  valueSuggestions,
  widgetKindFor,
  type ComboOption,
  type Conversion,
  type WidgetKind,
} from './form-logic.js';
import { actorOptions, createActorMemory, noteLinkOptions, tagItems, tagSuggestions, type ActorMemory } from './mbr-logic.js';
import { GENERIC_WIDGETS, widgetFor, type WidgetContext, type WidgetHost, type WidgetTable } from './fm-widgets.js';
import type { ComboCommitDetail, MbrFmCombobox } from './fm-combobox.js';
import './fm-combobox.js';
import { ensureFmStyles } from './fm-styles.js';
import { createYamlEditor, type YamlSourceEditor } from './yaml-source.js';
import type { YamlKnowledge } from './yaml-assist.js';
import {
  ICON_SM,
  icon,
  iconArrowLeftRight,
  iconBraces,
  iconCalendar,
  iconCalendarClock,
  iconCalendarDays,
  iconChevronRight,
  iconCode,
  iconEllipsis,
  iconHash,
  iconLink,
  iconList,
  iconListChecks,
  iconMessageSquareText,
  iconPlus,
  iconTag,
  iconText,
  iconTextCursorInput,
  iconToggleLeft,
  iconTrash2,
  iconTriangleAlert,
  iconUser,
} from '../icons.js';

/** Fired on the element for every change; `detail: { model, text }`. Bubbles. */
export const FM_CHANGE_EVENT = 'mbr-fm-change';

export interface FmChangeDetail {
  readonly model: FrontmatterModel;
  readonly text: string;
}

export type FmTab = 'form' | 'yaml';

const KIND_ICONS: Readonly<Record<WidgetKind, string>> = {
  text: iconText,
  longtext: iconMessageSquareText,
  number: iconHash,
  bool: iconToggleLeft,
  date: iconCalendar,
  datetime: iconCalendarClock,
  partialDate: iconCalendarDays,
  list: iconList,
  tags: iconTag,
  enum: iconListChecks,
  wikilink: iconLink,
  map: iconBraces,
  relationships: iconArrowLeftRight,
  labeled: iconUser,
  actor: iconUser,
  tree: iconBraces,
};

/** An "Edit as YAML" in progress. */
interface SliceEdit {
  readonly key: string;
  readonly path: FmPath;
  /** The value's YAML when the slice was opened. */
  readonly original: string;
  readonly text: string;
  readonly error?: string;
}

/** A "+ add key" field open under a nested map. */
interface NestedAdd {
  readonly key: string;
  readonly path: FmPath;
}

const asObject = (v: FmValue | undefined): Frontmatter =>
  v !== null && v !== undefined && typeof v === 'object' && !Array.isArray(v) ? v : {};

/** `path` is `prefix` or below it. */
const isUnder = (path: FmPath, prefix: FmPath): boolean => path.length >= prefix.length && prefix.every((s, i) => path[i] === s);

let nextId = 0;

@customElement('mbr-fm-editor')
export class MbrFmEditor extends LitElement implements WidgetHost {
  /** The frontmatter being edited. Replaced (never mutated) on every change. */
  @property({ attribute: false }) model: FrontmatterModel = openFrontmatter('');
  /** Built-in field knowledge (tag sources, relationship registry). */
  @property({ attribute: false }) catalog: FieldCatalog = builtinCatalog();
  /** Field knowledge inferred from the vault (`inferCatalog`). */
  @property({ attribute: false }) inferred: ReadonlyMap<string, InferredField> = new Map();
  /** `site.json`'s notes: titles for note-link suggestions. */
  @property({ attribute: false }) siteFiles: readonly (SiteFileLike & { url_path?: string })[] = [];
  /** The widget for each kind; extend {@link GENERIC_WIDGETS} to add one. */
  @property({ attribute: false }) widgets: WidgetTable = GENERIC_WIDGETS;
  /** The reader's actor id for the "me" shortcut; share one object to keep it across opens. */
  @property({ attribute: false }) actorMemory: ActorMemory = createActorMemory();
  @property() tab: FmTab = 'form';

  @state() private _status = '';
  @state() private _menu: string | null = null;
  @state() private _renaming: string | null = null;
  @state() private _slice: SliceEdit | null = null;
  @state() private _adding = false;
  @state() private _nestedAdd: NestedAdd | null = null;
  @state() private _collapsed: ReadonlySet<string> = new Set();
  /** A widget chosen with "Change type" (or by "+ Add property") for a value that does not show it yet. */
  @state() private _kinds: ReadonlyMap<string, WidgetKind> = new Map();
  @state() private _localHints: ReadonlyMap<string, string> = new Map();

  private readonly _id = `mbr-fm-${++nextId}`;
  /** The YAML tab's editor, while the tab is shown. */
  private _yamlEditor: YamlSourceEditor | null = null;
  /** The "Edit as YAML" editor, while a slice is open; keyed by the slice's row. */
  private _sliceEditor: { key: string; editor: YamlSourceEditor } | null = null;
  private _focus: { key: string; item?: number; target: 'value' | 'rename' | 'add' | 'menu' | 'menu-button' } | null = null;

  /** What a save writes: the model's text. */
  get text(): string {
    return this.model.text;
  }

  /**
   * Edits typed but not in {@link text} yet: an open "Edit as YAML" whose
   * text changed, a tag typed without Enter, a rename in progress. The
   * editor's dirty check counts them, and a save calls {@link flushPending}.
   */
  get hasPendingEdits(): boolean {
    if (this._slice && this._slice.text !== this._slice.original) return true;
    if (this._pendingRename() !== null) return true;
    return this._pendingTagFields().length > 0;
  }

  /**
   * Applies the edits {@link hasPendingEdits} names, as Apply, Enter and
   * leaving the rename field would. Returns why one could not be applied
   * (YAML that does not parse, a name already taken), with the edit left open
   * for the user to fix, or `null` when {@link text} now holds them all.
   */
  flushPending(): string | null {
    const rename = this._pendingRename();
    if (this._renaming !== null) {
      const path = JSON.parse(this._renaming) as FmPath;
      this._renaming = null;
      if (rename !== null && !this._rename(path, rename)) return `Properties: could not rename “${String(path[path.length - 1])}”: ${this._status}`;
    }
    for (const field of this._pendingTagFields()) field.commitTyped();
    const slice = this._slice;
    if (slice) {
      if (slice.text === slice.original) this._slice = null;
      else if (!this._applySlice()) return `Properties: the YAML for “${slice.path.join('.')}” is not valid: ${this._slice?.error ?? ''}`;
    }
    return null;
  }

  /** The new name in an open rename field, when it is one (not blank, not the old name). */
  private _pendingRename(): string | null {
    if (this._renaming === null) return null;
    const path = JSON.parse(this._renaming) as FmPath;
    const typed = this.querySelector<HTMLInputElement>('.fm-rename')?.value.trim() ?? '';
    return typed && typed !== path[path.length - 1] ? typed : null;
  }

  /** Tag fields holding a tag typed but not added. */
  private _pendingTagFields(): MbrFmCombobox[] {
    return [...this.querySelectorAll<MbrFmCombobox>('mbr-fm-combobox.fm-tag-entry')].filter((c) => c.pendingText !== '');
  }

  /** Replaces the frontmatter (wholesale, as the YAML view does). */
  set text(value: string) {
    this.model = openFrontmatter(value);
  }

  protected override createRenderRoot(): HTMLElement {
    return this;
  }

  override connectedCallback(): void {
    super.connectedCallback();
    ensureFmStyles(this.ownerDocument);
    this.classList.add('mbr-fm');
    this.addEventListener('keydown', this._onHostKeydown);
  }

  override disconnectedCallback(): void {
    super.disconnectedCallback();
    this.removeEventListener('keydown', this._onHostKeydown);
    this._removeMenuListeners();
    this._mountYaml(undefined);
    this._mountSlice(undefined);
  }

  /** What completion knows, read when it is asked (site.json arrives after the editor opens). */
  private readonly _knowledge = (): YamlKnowledge => ({
    catalog: this.catalog,
    inferred: this.inferred,
    siteFiles: this._linkableFiles(),
    actorMemory: this.actorMemory,
  });

  private _linkable: { from: MbrFmEditor['siteFiles']; files: (SiteFileLike & { url_path: string })[] } | null = null;

  /** `siteFiles` that can be linked to (with a URL); one array per `siteFiles`, so what is built from it can be memoized. */
  private _linkableFiles(): (SiteFileLike & { url_path: string })[] {
    if (this._linkable?.from !== this.siteFiles) {
      this._linkable = { from: this.siteFiles, files: this.siteFiles.filter((f): f is SiteFileLike & { url_path: string } => typeof f.url_path === 'string') };
    }
    return this._linkable.files;
  }

  private _noteLinks: { from: readonly (SiteFileLike & { url_path: string })[]; options: ComboOption[] } | null = null;

  /** Note-link suggestions, built once per `_linkableFiles()` array (every wikilink field asks on every render). */
  private _noteLinkOptions(): ComboOption[] {
    const files = this._linkableFiles();
    if (this._noteLinks?.from !== files) this._noteLinks = { from: files, options: noteLinkOptions(files) };
    return this._noteLinks.options;
  }

  /** Mounts the YAML tab's editor in its container (Lit `ref`), or tears it down. */
  private readonly _mountYaml = (el: Element | undefined): void => {
    if (this._yamlEditor && el && this._yamlEditor.view.dom.parentElement === el) return;
    this._yamlEditor?.destroy();
    this._yamlEditor = null;
    if (!el) return;
    this._yamlEditor = createYamlEditor({
      parent: el as HTMLElement,
      doc: this.model.text,
      label: 'Frontmatter YAML',
      placeholder: 'title: My note',
      knowledge: this._knowledge,
      onChange: (text) => this._commit(openFrontmatter(text)),
    });
  };

  /** Mounts the "Edit as YAML" editor for the open slice, or tears it down. */
  private readonly _mountSlice = (el: Element | undefined): void => {
    const slice = this._slice;
    if (this._sliceEditor && el && slice && this._sliceEditor.key === slice.key && this._sliceEditor.editor.view.dom.parentElement === el) return;
    this._sliceEditor?.editor.destroy();
    this._sliceEditor = null;
    if (!el || !slice) return;
    const editor = createYamlEditor({
      parent: el as HTMLElement,
      doc: slice.text,
      label: 'Value as YAML',
      knowledge: this._knowledge,
      basePath: slice.path,
      frontmatter: () => this.frontmatter,
      onChange: (text) => {
        if (this._slice) this._slice = { ...this._slice, text, error: undefined };
      },
      onEscape: () => {
        this._slice = null;
        this._focus = { key: slice.key, target: 'menu-button' };
      },
      onSubmit: () => this._applySlice(),
    });
    this._sliceEditor = { key: slice.key, editor };
  };

  /** Opens the "+ Add property" field (switching to the form). */
  openAddProperty(): void {
    if (this.model.errors.length > 0) return;
    this.tab = 'form';
    this._adding = true;
    this._focus = { key: '', target: 'add' };
  }

  // ---------------------------------------------------------------------------
  // WidgetHost
  // ---------------------------------------------------------------------------

  apply(op: (model: FrontmatterModel) => FrontmatterModel): boolean {
    try {
      const next = op(this.model);
      if (next !== this.model) this._commit(next);
      this._status = '';
      return true;
    } catch (err) {
      this._status = err instanceof FrontmatterEditError ? err.message : `Could not apply the edit: ${(err as Error).message}`;
      return false;
    }
  }

  /** The whole frontmatter value, `{}` when it is not a map. */
  get frontmatter(): Frontmatter {
    return this.model.errors.length > 0 ? {} : asObject(toValue(this.model));
  }

  suggestions(path: FmPath, def: FieldDef, kind: WidgetKind): ComboOption[] {
    if (kind === 'tags') {
      const field = path.filter((s): s is string => typeof s === 'string').join('.');
      return tagSuggestions(field, this.catalog, this.inferred, tagItems(getIn(this.model, path)));
    }
    const out = valueSuggestions(path, def, this.inferred);
    const extra =
      kind === 'wikilink'
        ? this._noteLinkOptions()
        : kind === 'actor'
          ? actorOptions(this.inferred, this.actorMemory)
          : [];
    const seen = new Set(out.map((o) => `${o.value}\u0000${o.detail ?? ''}`));
    for (const o of extra) {
      const key = `${o.value}\u0000${o.detail ?? ''}`;
      if (seen.has(key) || (kind === 'actor' && out.some((x) => x.value === o.value))) continue;
      seen.add(key);
      out.push(o);
    }
    return out;
  }

  renderChildRows(path: FmPath, keep: (key: string | number) => boolean): TemplateResult {
    const fm = this.frontmatter;
    const hints = validate(fm, this.catalog);
    return html`${entries(this.model, path)
      .filter((e) => keep(e.key))
      .map((e) => this._renderRow(e, fm, hints))}`;
  }

  renderScalar(ctx: WidgetContext): TemplateResult {
    return widgetFor(this.widgets, ctx.kind, ctx.value)(ctx);
  }

  renderTreeSummary(ctx: WidgetContext): TemplateResult {
    const key = pathKey(ctx.path);
    const open = !this._collapsed.has(key);
    const v = ctx.value;
    const n = Array.isArray(v) ? v.length : v && typeof v === 'object' ? Object.keys(v).length : 0;
    const what = Array.isArray(v) ? (n === 1 ? 'item' : 'items') : n === 1 ? 'key' : 'keys';
    const summary = typeof ctx.path[ctx.path.length - 1] === 'number' ? itemSummary(v, ctx.def) : '';
    return html`<button
      type="button"
      class="fm-link fm-disclosure"
      aria-expanded=${open ? 'true' : 'false'}
      aria-controls=${`${this._id}-c-${this._domId(key)}`}
      @click=${() => this._toggleCollapsed(key)}
    >
      ${icon(iconChevronRight, { size: ICON_SM, className: 'chevron' })}${summary
        ? html`<span class="fm-summary">${summary}</span>`
        : n === 0
          ? 'Empty'
          : `${n} ${what}`}
    </button>`;
  }

  removeListItem(path: FmPath, index: number): boolean {
    const ok = this.apply((m) => removeItem(m, path, index));
    if (ok) this._removeUiState([...path, index]);
    return ok;
  }

  focusAfterUpdate(path: FmPath, item?: number): void {
    this._focus = { key: pathKey(path), item, target: 'value' };
  }

  setLocalHint(path: FmPath, message: string | null): void {
    const key = pathKey(path);
    if ((this._localHints.get(key) ?? null) === message) return;
    const next = new Map(this._localHints);
    if (message === null) next.delete(key);
    else next.set(key, message);
    this._localHints = next;
  }

  // ---------------------------------------------------------------------------
  // State changes
  // ---------------------------------------------------------------------------

  private _commit(next: FrontmatterModel): void {
    this.model = next;
    this.dispatchEvent(
      new CustomEvent<FmChangeDetail>(FM_CHANGE_EVENT, { detail: { model: next, text: next.text }, bubbles: true }),
    );
  }

  private _toggleCollapsed(key: string): void {
    const next = new Set(this._collapsed);
    if (next.has(key)) next.delete(key);
    else next.add(key);
    this._collapsed = next;
  }

  private _setKind(path: FmPath, kind: WidgetKind | null): void {
    const next = new Map(this._kinds);
    if (kind === null) next.delete(pathKey(path));
    else next.set(pathKey(path), kind);
    this._kinds = next;
  }

  /** UI state (chosen kinds, hints, collapsed rows) re-keyed by `map`; a `null` path drops it. */
  private _remapUiState(map: (path: FmPath) => FmPath | null): void {
    const remap = <V>(m: ReadonlyMap<string, V>): Map<string, V> => {
      const out = new Map<string, V>();
      for (const [k, v] of m) {
        const to = map(JSON.parse(k) as FmPath);
        if (to) out.set(pathKey(to), v);
      }
      return out;
    };
    this._kinds = remap(this._kinds);
    this._localHints = remap(this._localHints);
    this._collapsed = new Set(remap(new Map([...this._collapsed].map((k) => [k, true]))).keys());
  }

  /** UI state keyed by `path` (and anything below it) moved to `to`. */
  private _moveUiState(path: FmPath, to: FmPath): void {
    this._remapUiState((p) => (isUnder(p, path) ? [...to, ...p.slice(path.length)] : p));
  }

  /**
   * UI state for the removed `path` (and anything below it) dropped; when it
   * was a list item, the later items' state moves up one, as the items do.
   */
  private _removeUiState(path: FmPath): void {
    const parent = path.slice(0, -1);
    const removed = path[path.length - 1];
    const depth = parent.length;
    this._remapUiState((p) => {
      if (isUnder(p, path)) return null;
      const i = p[depth];
      if (typeof removed === 'number' && typeof i === 'number' && i > removed && isUnder(p, parent)) {
        return [...parent, i - 1, ...p.slice(depth + 1)];
      }
      return p;
    });
  }

  // ---------------------------------------------------------------------------
  // Keyboard and the row menu
  // ---------------------------------------------------------------------------

  private readonly _onHostKeydown = (e: KeyboardEvent): void => {
    if ((e.metaKey || e.ctrlKey) && !e.altKey && e.key === ';') {
      e.preventDefault();
      this.openAddProperty();
    }
  };

  private readonly _onMenuKeydown = (e: KeyboardEvent): void => {
    if (this._menu === null) return;
    if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      this._closeMenu(true);
      return;
    }
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      const items = [...this.querySelectorAll<HTMLButtonElement>('.fm-menu [role="menuitem"]')];
      if (items.length === 0) return;
      e.preventDefault();
      const at = items.indexOf(document.activeElement as HTMLButtonElement);
      const step = e.key === 'ArrowDown' ? 1 : -1;
      items[at < 0 ? (step > 0 ? 0 : items.length - 1) : (at + step + items.length) % items.length].focus();
    }
  };

  private readonly _onOutsidePointer = (e: Event): void => {
    const target = e.target as Node | null;
    const menu = this.querySelector('.fm-menu');
    const button = this.querySelector('.fm-menu-btn[aria-expanded="true"]');
    if (target && (menu?.contains(target) || button?.contains(target))) return;
    this._closeMenu(false);
  };

  private _openMenu(key: string): void {
    this._menu = key;
    this._focus = { key, target: 'menu' };
    document.addEventListener('keydown', this._onMenuKeydown, true);
    document.addEventListener('pointerdown', this._onOutsidePointer, true);
  }

  private _closeMenu(refocus: boolean): void {
    if (this._menu === null) return;
    if (refocus) this._focus = { key: this._menu, target: 'menu-button' };
    this._menu = null;
    this._removeMenuListeners();
  }

  private _removeMenuListeners(): void {
    document.removeEventListener('keydown', this._onMenuKeydown, true);
    document.removeEventListener('pointerdown', this._onOutsidePointer, true);
  }

  // ---------------------------------------------------------------------------
  // Row actions
  // ---------------------------------------------------------------------------

  private _remove(entry: ModelEntry): void {
    const path = entry.path;
    const parent = path.slice(0, -1);
    const last = path[path.length - 1];
    if (typeof last === 'number') this.removeListItem(parent, last);
    else if (this.apply((m) => removeKey(m, path))) this._removeUiState(path);
  }

  /** Renames the key at `path` to `raw` (trimmed; blank or unchanged is no rename). Returns false when the rename was refused. */
  private _rename(path: FmPath, raw: string): boolean {
    const newKey = raw.trim();
    this._renaming = null;
    if (!newKey || newKey === path[path.length - 1]) {
      this._focus = { key: pathKey(path), target: 'value' };
      return true;
    }
    const to = [...path.slice(0, -1), newKey];
    if (!this.apply((m) => renameKey(m, path, newKey))) return false;
    this._moveUiState(path, to);
    this._focus = { key: pathKey(to), target: 'value' };
    return true;
  }

  private _convert(entry: ModelEntry, conversion: Conversion): void {
    this._closeMenu(false);
    if (conversion.write !== undefined) {
      const write = conversion.write;
      if (!this.apply((m) => setValue(m, entry.path, write))) return;
    }
    this._setKind(entry.path, conversion.kind);
    this._focus = { key: pathKey(entry.path), target: 'value' };
  }

  /** Applies the open "Edit as YAML"; on an error it stays open, saying why. Returns whether it applied. */
  private _applySlice(): boolean {
    const slice = this._slice;
    if (!slice) return true;
    try {
      const next = replaceSubtree(this.model, slice.path, slice.text);
      if (next.text !== this.model.text) this._commit(next);
      this._slice = null;
      this._setKind(slice.path, null);
      this._focus = { key: slice.key, target: 'menu-button' };
      return true;
    } catch (err) {
      this._slice = { ...slice, error: (err as Error).message };
      return false;
    }
  }

  private _addProperty(raw: string): void {
    const key = raw.trim();
    if (!key) {
      this._adding = false;
      return;
    }
    const fm = asObject(toValue(this.model));
    const path = propertyPath(key);
    // Nested or as one flat key (`dates.birthday:`): either way it is there.
    const existing = existingPropertyPath(this.model, key);
    if (existing) {
      this._status = `“${key}” is already a property.`;
      this._adding = false;
      this._focus = { key: pathKey(existing), target: 'value' };
      return;
    }
    const suggested = addableFields(fm, this.catalog, this.inferred).find((f) => f.key === key);
    const def = resolveField(path, fm, this.catalog, this.inferred);
    const kind: WidgetKind = suggested?.kind ?? def.kind;
    const value = newValueFor({ ...def, kind });
    if (!this.apply((m) => addProperty(m, key, value))) return;
    this._adding = false;
    this._setKind(path, kind);
    this._focus = { key: pathKey(path), target: 'value' };
  }

  private _addNestedKey(path: FmPath, raw: string, def: FieldDef): void {
    const key = raw.trim();
    this._nestedAdd = null;
    if (!key) return;
    const childPath = [...path, key];
    const fm = asObject(toValue(this.model));
    const childDef = def.children?.find((c) => c.key === key) ?? resolveField(childPath, fm, this.catalog, this.inferred);
    if (this.apply((m) => addKey(m, path, key, newValueFor(childDef)))) {
      this._setKind(childPath, childDef.kind);
      this._focus = { key: pathKey(childPath), target: 'value' };
    }
  }

  private _addNestedItem(path: FmPath, def: FieldDef, count: number): void {
    const item = newValueFor(def.item ?? { key: '', kind: 'text', label: 'Item', group: def.group });
    // A lone `verified: {by, at}` becomes a list holding it before the new item.
    if (this.apply((m) => (def.singleOrList ? addToSingleOrList(m, path, item) : addItem(m, path, item)))) {
      const itemPath = [...path, count];
      if (def.item) this._setKind(itemPath, def.item.kind);
      this._focus = { key: pathKey(itemPath), target: 'value' };
    }
  }

  // ---------------------------------------------------------------------------
  // Focus after render
  // ---------------------------------------------------------------------------

  private _row(key: string): HTMLElement | undefined {
    return [...this.querySelectorAll<HTMLElement>('.fm-row')].find((r) => r.dataset.path === key);
  }

  protected override updated(changed: PropertyValues<this>): void {
    super.updated(changed);
    // A model set from outside (or by the form) while the YAML tab is open.
    this._yamlEditor?.setText(this.model.text);
    const focus = this._focus;
    if (!focus) return;
    this._focus = null;
    if (focus.target === 'add') {
      (this.querySelector('.fm-add mbr-fm-combobox') as HTMLElement | null)?.focus();
      return;
    }
    const row = this._row(focus.key);
    if (!row) return;
    const own = (sel: string) => [...row.querySelectorAll<HTMLElement>(sel)].find((el) => el.closest('.fm-row') === row);
    if (focus.target === 'menu') own('.fm-menu [role="menuitem"]')?.focus();
    else if (focus.target === 'menu-button') own('.fm-menu-btn')?.focus();
    else if (focus.target === 'rename') own('.fm-rename')?.focus();
    else if (focus.item !== undefined) {
      own(`.fm-value li[data-index="${focus.item}"] input, .fm-value li[data-index="${focus.item}"] textarea`)?.focus();
    } else {
      own('.fm-value input, .fm-value textarea, .fm-value select, .fm-value button')?.focus();
    }
  }

  // ---------------------------------------------------------------------------
  // Rendering
  // ---------------------------------------------------------------------------

  private _domId(key: string): string {
    return key.replace(/[^A-Za-z0-9_-]/g, (c) => `_${c.charCodeAt(0).toString(16)}`);
  }

  private _kindFor(path: FmPath, def: FieldDef, value: FmValue | undefined): WidgetKind {
    const chosen = this._kinds.get(pathKey(path));
    if (chosen && fits(chosen, value)) return chosen;
    return widgetKindFor(def, value);
  }

  protected override render(): TemplateResult {
    const yaml = this.tab === 'yaml';
    return html`<div class="fm-tabs" role="tablist" aria-label="Properties view">
        ${this._renderTab('form', 'Form')}${this._renderTab('yaml', 'YAML')}
      </div>
      <div role="tabpanel" id=${`${this._id}-panel`} aria-labelledby=${`${this._id}-tab-${this.tab}`}>
        ${yaml ? this._renderYaml() : this._renderForm()}
      </div>`;
  }

  private _renderTab(tab: FmTab, label: string): TemplateResult {
    const selected = this.tab === tab;
    return html`<button
      type="button"
      role="tab"
      class="fm-tab"
      id=${`${this._id}-tab-${tab}`}
      aria-selected=${selected ? 'true' : 'false'}
      aria-controls=${`${this._id}-panel`}
      tabindex=${selected ? '0' : '-1'}
      @click=${() => this._selectTab(tab)}
      @keydown=${(e: KeyboardEvent) => {
        if (e.key === 'ArrowRight' || e.key === 'ArrowLeft') {
          e.preventDefault();
          const other: FmTab = tab === 'form' ? 'yaml' : 'form';
          this._selectTab(other);
          void this.updateComplete.then(() => (this.querySelector(`#${this._id}-tab-${other}`) as HTMLElement | null)?.focus());
        }
      }}
    >
      ${label}
    </button>`;
  }

  private _selectTab(tab: FmTab): void {
    if (this.tab === tab) return;
    this._closeMenu(false);
    this._slice = null;
    this._renaming = null;
    this._adding = false;
    this.tab = tab;
  }

  private _renderErrors(): TemplateResult | typeof nothing {
    const errors = this.model.errors;
    if (errors.length === 0) return nothing;
    return html`<div class="fm-errors" role="alert">
      <p>The YAML has ${errors.length === 1 ? 'an error' : `${errors.length} errors`}; the form is read-only until it is fixed.</p>
      <ul>
        ${errors.map((e) => html`<li>Line ${e.line}, column ${e.col}: ${e.message}</li>`)}
      </ul>
    </div>`;
  }

  private _renderYaml(): TemplateResult {
    return html`<div class="fm-cm fm-yaml-text" ${ref(this._mountYaml)}></div>
      ${this._renderErrors()}`;
  }

  private _renderForm(): TemplateResult {
    const model = this.model;
    if (model.errors.length > 0) {
      return html`${this._renderErrors()}
        <pre class="fm-preview" aria-label="Frontmatter YAML (read-only)">${model.text}</pre>
        <button type="button" class="fm-link" @click=${() => this._selectTab('yaml')}>${icon(iconCode, { size: ICON_SM })}Fix it in YAML</button>`;
    }
    const root = toValue(model);
    if (root === null || typeof root !== 'object' || Array.isArray(root)) {
      return html`<p class="fm-note">This frontmatter is not a set of properties; edit it as YAML.</p>
        <button type="button" class="fm-link" @click=${() => this._selectTab('yaml')}>${icon(iconCode, { size: ICON_SM })}Edit YAML</button>`;
    }
    const fm = root as Frontmatter;
    const hints = validate(fm, this.catalog);
    const rows = entries(model, []);
    const shownHint = (h: FieldHint) => getIn(model, h.path) !== undefined;
    const orphanHints = hints.filter((h) => !shownHint(h));
    return html`
      <div class="fm-rows" role="list" aria-label="Properties">
        ${rows.length === 0 && !this._adding ? html`<p class="fm-empty">No properties yet.</p>` : nothing}
        ${rows.map((entry) => this._renderRow(entry, fm, hints))}
      </div>
      ${orphanHints.map(
        (h) => html`<p class="fm-note">${icon(iconTriangleAlert, { size: 14 })} ${h.message}</p>`,
      )}
      <div class="fm-add">
        ${this._adding
          ? html`<mbr-fm-combobox
              label="New property name"
              placeholder="Property name"
              openOnFocus
              commitOnTab
              escapeCancels
              clearOnCommit
              .options=${addableFields(fm, this.catalog, this.inferred).map((f) => ({
                value: f.key,
                detail: f.count ? `${f.label} · ${f.count} note${f.count === 1 ? '' : 's'}` : f.help ? `${f.label} · ${f.help}` : f.label,
              }))}
              @fm-combo-commit=${(e: CustomEvent<ComboCommitDetail>) => this._addProperty(e.detail.value)}
              @fm-combo-cancel=${() => {
                this._adding = false;
              }}
              @focusout=${(e: FocusEvent) => {
                const to = e.relatedTarget as Node | null;
                if (!to || !(e.currentTarget as HTMLElement).contains(to)) this._adding = false;
              }}
            ></mbr-fm-combobox>`
          : html`<button
              type="button"
              class="fm-link"
              title="Add a property (⌘; / Ctrl+;)"
              @click=${() => this.openAddProperty()}
            >
              ${icon(iconPlus, { size: ICON_SM })}Add property
            </button>`}
      </div>
      ${model.normalized
        ? html`<p class="fm-note">One edit could not be made in place, so the YAML was rewritten in a standard layout.</p>`
        : nothing}
      <div class="fm-status" role="status">${this._status}</div>
    `;
  }

  private _renderRow(entry: ModelEntry, fm: Frontmatter, hints: readonly FieldHint[]): TemplateResult {
    const key = pathKey(entry.path);
    const isItem = typeof entry.key === 'number';
    const parentLabel = entry.path.slice(0, -1).filter((s) => typeof s === 'string').join('.');
    const name = isItem ? `#${(entry.key as number) + 1}` : String(entry.key);
    const label = isItem ? `${parentLabel || 'list'}, item ${(entry.key as number) + 1}` : name;
    const def = resolveField(entry.path, fm, this.catalog, this.inferred, entry.value);
    const kind = this._kindFor(entry.path, def, entry.value);
    const info = entry.info;
    const ctx: WidgetContext = { host: this, path: entry.path, value: entry.value, def, kind, info, label };
    const slicing = this._slice?.key === key;
    const renderer = widgetFor(this.widgets, kind, entry.value);
    const isTree = renderer === (this.widgets.tree ?? GENERIC_WIDGETS.tree);
    const rowHints = hints.filter((h) => pathKey(h.path) === key);
    const localHint = this._localHints.get(key);
    const nameId = `${this._id}-n-${this._domId(key)}`;
    return html`<div class="fm-row" role="listitem" data-path=${key} aria-labelledby=${nameId}>
      <span class="fm-icon" title=${KIND_LABELS[kind]}>${icon(KIND_ICONS[kind], { size: ICON_SM })}</span>
      ${this._renaming === key
        ? html`<input
            type="text"
            class="fm-in fm-rename"
            aria-label=${`Rename ${name}`}
            .value=${name}
            @keydown=${(e: KeyboardEvent) => {
              if (e.key === 'Enter' || (e.key === 'Tab' && !e.shiftKey)) {
                e.preventDefault();
                this._rename(entry.path, (e.target as HTMLInputElement).value);
              } else if (e.key === 'Escape') {
                e.preventDefault();
                e.stopPropagation();
                this._renaming = null;
                this._focus = { key, target: 'menu-button' };
              }
            }}
            @blur=${(e: FocusEvent) => {
              if (this._renaming === key) this._rename(entry.path, (e.target as HTMLInputElement).value);
            }}
          />`
        : html`<span class="fm-name" id=${nameId} title=${def.help ?? def.label}><span>${name}</span></span>`}
      <div class="fm-value">
        ${slicing
          ? this._renderSliceEditor()
          : info.needsYaml
            ? this._renderYamlPreview(entry)
            : renderer(ctx)}
      </div>
      <div class="fm-actions">${this._renderMenu(entry, kind, name)}</div>
      ${rowHints.map((h) => html`<p class="fm-hint">${icon(iconTriangleAlert, { size: 14 })}${h.message}</p>`)}
      ${localHint ? html`<p class="fm-local-hint" role="status">${localHint}</p>` : nothing}
      ${isTree && !info.needsYaml && !slicing && !this._collapsed.has(key) ? this._renderChildren(entry, def, fm, hints) : nothing}
    </div>`;
  }

  private _renderChildren(entry: ModelEntry, def: FieldDef, fm: Frontmatter, hints: readonly FieldHint[]): TemplateResult {
    const key = pathKey(entry.path);
    const value = entry.value;
    const children = entries(this.model, entry.path);
    const isMap = value === null || (typeof value === 'object' && !Array.isArray(value));
    const single = Boolean(def.singleOrList) && isMap && value !== null;
    const isSeq = value === null || Array.isArray(value) || single;
    const adding = this._nestedAdd?.key === key;
    const present = new Set(children.map((c) => String(c.key)));
    const keyOptions: ComboOption[] = (def.children ?? [])
      .filter((c) => !present.has(c.key))
      .map((c) => ({ value: c.key, detail: c.help ? `${c.label} · ${c.help}` : c.label }));
    return html`<div class="fm-children" id=${`${this._id}-c-${this._domId(key)}`} role="list">
      ${children.map((child) => this._renderRow(child, fm, hints))}
      <div class="fm-add">
        ${adding
          ? html`<mbr-fm-combobox
              label="New key name"
              placeholder="Key"
              openOnFocus
              commitOnTab
              escapeCancels
              clearOnCommit
              .options=${keyOptions}
              @fm-combo-commit=${(e: CustomEvent<ComboCommitDetail>) => this._addNestedKey(entry.path, e.detail.value, def)}
              @fm-combo-cancel=${() => {
                this._nestedAdd = null;
              }}
              @focusout=${(e: FocusEvent) => {
                const to = e.relatedTarget as Node | null;
                if (!to || !(e.currentTarget as HTMLElement).contains(to)) this._nestedAdd = null;
              }}
            ></mbr-fm-combobox>`
          : html`${isMap
              ? html`<button
                  type="button"
                  class="fm-link"
                  @click=${() => {
                    this._nestedAdd = { key, path: entry.path };
                    void this.updateComplete.then(() => (this._row(key)?.querySelector('.fm-children > .fm-add mbr-fm-combobox') as HTMLElement | null)?.focus());
                  }}
                >
                  ${icon(iconPlus, { size: ICON_SM })}Add key
                </button>`
              : nothing}
            ${isSeq
              ? html`<button type="button" class="fm-link" @click=${() => this._addNestedItem(entry.path, def, single ? 1 : children.length)}>
                  ${icon(iconPlus, { size: ICON_SM })}Add item
                </button>`
              : nothing}`}
      </div>
    </div>`;
  }

  private _renderYamlPreview(entry: ModelEntry): TemplateResult {
    let source = '';
    try {
      source = sliceAt(this.model, entry.path);
    } catch {
      source = '';
    }
    return html`<pre class="fm-preview" aria-label=${`${String(entry.key)} as YAML (read-only)`}>${source}</pre>
      <button type="button" class="fm-link" @click=${() => this._startSlice(entry)}>
        ${icon(iconCode, { size: ICON_SM })}Edit as YAML
      </button>`;
  }

  private _startSlice(entry: ModelEntry): void {
    this._closeMenu(false);
    const text = sliceAt(this.model, entry.path);
    this._slice = { key: pathKey(entry.path), path: entry.path, original: text, text };
    void this.updateComplete.then(() => this._sliceEditor?.editor.focus());
  }

  private _renderSliceEditor(): TemplateResult {
    const slice = this._slice!;
    // Escape cancels and Cmd/Ctrl+Enter applies (the editor's own keys, in `_mountSlice`).
    return html`<div class="fm-cm fm-yaml-slice" data-invalid=${slice.error ? 'true' : 'false'} ${ref(this._mountSlice)}></div>
      ${slice.error ? html`<p class="fm-status" role="alert">${slice.error}</p>` : nothing}
      <span class="fm-yaml-actions">
        <button type="button" class="fm-link" @click=${() => this._applySlice()}>Apply</button>
        <button
          type="button"
          class="fm-link fm-disclosure"
          @click=${() => {
            this._slice = null;
          }}
        >
          Cancel
        </button>
      </span>`;
  }

  private _renderMenu(entry: ModelEntry, kind: WidgetKind, name: string): TemplateResult {
    const key = pathKey(entry.path);
    const open = this._menu === key;
    const isItem = typeof entry.key === 'number';
    // A conversion that writes a new value over a collection would drop the comments inside it.
    const collection = entry.info.kind === 'map' || entry.info.kind === 'seq';
    const conversions = entry.info.needsYaml
      ? []
      : typeConversions(entry.value, kind).filter((c) => !(collection && entry.info.hasComments && c.write !== undefined));
    const item = (label: string, shape: string | null, run: () => void, cls = '') =>
      html`<button type="button" role="menuitem" tabindex="-1" class=${cls} @click=${run}>
        ${shape ? icon(shape, { size: ICON_SM }) : nothing}${label}
      </button>`;
    return html`<button
        type="button"
        class="mbr-icon-btn sm fm-menu-btn"
        aria-label=${`Actions for ${name}`}
        title="Actions"
        aria-haspopup="menu"
        aria-expanded=${open ? 'true' : 'false'}
        @click=${() => (open ? this._closeMenu(false) : this._openMenu(key))}
      >
        ${icon(iconEllipsis, { size: ICON_SM })}
      </button>
      ${open
        ? html`<div class="fm-menu" role="menu" aria-label=${`Actions for ${name}`}>
            ${isItem
              ? nothing
              : item('Rename', iconTextCursorInput, () => {
                  this._closeMenu(false);
                  this._renaming = key;
                  this._focus = { key, target: 'rename' };
                })}
            ${item('Edit as YAML', iconCode, () => this._startSlice(entry))}
            ${conversions.length > 0
              ? html`<hr /><span class="fm-menu-label" aria-hidden="true">Change type to</span>
                  ${conversions.map((c) => item(c.label, KIND_ICONS[c.kind], () => this._convert(entry, c)))}`
              : nothing}
            <hr />
            ${item(
              'Remove',
              iconTrash2,
              () => {
                this._closeMenu(false);
                this._remove(entry);
              },
              'danger',
            )}
          </div>`
        : nothing}`;
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'mbr-fm-editor': MbrFmEditor;
  }
}

