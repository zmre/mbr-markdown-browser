/**
 * The value widgets of the properties form, one renderer per widget kind.
 *
 * `<mbr-fm-editor>` resolves a field's definition and widget kind
 * (`form-logic.ts`'s `widgetKindFor`) and looks the kind up in its `widgets`
 * table, which defaults to {@link GENERIC_WIDGETS}. mbr/OKF-specific widgets
 * (relationships, labeled contact values, actors, note links) are added by
 * passing a table that spreads this one, so the form itself never names them.
 * A kind with no renderer falls back to the generic tree (collections) or the
 * text field (scalars).
 *
 * Every renderer edits through `host.apply`, which runs a model operation
 * (`model.ts`) and reports a refusal instead of throwing; nothing here writes
 * text itself.
 */

import { html, noChange, type TemplateResult } from 'lit';
import { Directive, directive, type PropertyPart } from 'lit/directive.js';
import { live } from 'lit/directives/live.js';
import {
  addItem,
  setScalar,
  type FmPath,
  type FmScalar,
  type FmValue,
  type FrontmatterModel,
  type NodeInfo,
} from './model.js';
import {
  applyDatetimeLocal,
  datetimeLocalValue,
  formatOffset,
  localOffsetMinutes,
  parseDateTime,
  type FieldCatalog,
  type FieldDef,
  type Frontmatter,
} from './fields.js';
import { addTag, removeTag, tagItems, type ActorMemory } from './mbr-logic.js';
import {
  parseNumberInput,
  partialDateFields,
  partialDateValue,
  type ComboOption,
  type WidgetKind,
} from './form-logic.js';
import type { ComboCommitDetail } from './fm-combobox.js';
import './fm-combobox.js';
import { ICON_SM, icon, iconPlus, iconX } from '../icons.js';

/** What a widget may ask of the form. */
export interface WidgetHost {
  readonly model: FrontmatterModel;
  /** Runs `op` on the model; a refusal is shown in the form. Returns whether it applied. */
  apply(op: (model: FrontmatterModel) => FrontmatterModel): boolean;
  /** Removes item `index` of the list at `path`; the form's state for later items (collapsed, chosen kind) moves up with them. */
  removeListItem(path: FmPath, index: number): boolean;
  /** Value suggestions for `path` (enum values, then the vault's). */
  suggestions(path: FmPath, def: FieldDef, kind: WidgetKind): ComboOption[];
  /** The scalar widget for one value (a list draws its items with it). */
  renderScalar(ctx: WidgetContext): TemplateResult;
  /** The disclosure for a nested map or list (its rows are drawn by the form). */
  renderTreeSummary(ctx: WidgetContext): TemplateResult;
  /** Focus the value at `path` once the next render is done (`item`: a list's item `index`). */
  focusAfterUpdate(path: FmPath, item?: number): void;
  /** A message under the row at `path` that is not a validation hint (`null` clears it). */
  setLocalHint(path: FmPath, message: string | null): void;
  /** The generic rows for the children of the collection at `path` that `keep` selects. */
  renderChildRows(path: FmPath, keep: (key: string | number) => boolean): TemplateResult;
  /** Built-in field knowledge (tag sources, the relationship registry). */
  readonly catalog: FieldCatalog;
  /** The whole frontmatter value (gating, phrasing). */
  readonly frontmatter: Frontmatter;
  /** The reader's actor id for the "me" shortcut (memory only). */
  readonly actorMemory: ActorMemory;
}

/** One value to draw. */
export interface WidgetContext {
  readonly host: WidgetHost;
  readonly path: FmPath;
  readonly value: FmValue | undefined;
  readonly def: FieldDef;
  readonly kind: WidgetKind;
  readonly info: NodeInfo;
  /** The accessible name of the field (its key, or `key, item N`). */
  readonly label: string;
}

export type WidgetRenderer = (ctx: WidgetContext) => TemplateResult;
export type WidgetTable = Readonly<Partial<Record<WidgetKind, WidgetRenderer>>>;

const str = (v: FmValue | undefined): string => (v === null || v === undefined ? '' : typeof v === 'object' ? '' : String(v));

const inputValue = (e: Event): string => (e.target as HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement).value;

/** Writes a scalar, a cleared field as an empty value (`key:`). */
function writeScalar(ctx: WidgetContext, value: FmScalar): void {
  ctx.host.apply((m) => setScalar(m, ctx.path, value === '' ? null : value));
}

const textWidget: WidgetRenderer = (ctx) =>
  html`<input
    type="text"
    class="fm-in"
    aria-label=${ctx.label}
    .value=${live(str(ctx.value))}
    @input=${(e: Event) => writeScalar(ctx, inputValue(e))}
  />`;

const longtextWidget: WidgetRenderer = (ctx) => {
  const text = str(ctx.value);
  const rows = Math.min(10, Math.max(2, text.split('\n').length));
  return html`<textarea
    class="fm-in"
    rows=${rows}
    aria-label=${ctx.label}
    .value=${live(text)}
    @input=${(e: Event) => writeScalar(ctx, inputValue(e))}
  ></textarea>`;
};

/**
 * A number field's `.value`: the stored number, except while what is in the
 * field already means it (`1.`, `1.0` and `1.00` are all 1, and rewriting
 * them as `1` would make `1.05` untypable) or is not a number yet (`-`, or
 * a browser's `badInput`).
 */
class NumberValue extends Directive {
  render(value: FmValue | undefined): string {
    return str(value);
  }

  override update(part: PropertyPart, [value]: [FmValue | undefined]): unknown {
    const input = part.element as HTMLInputElement;
    const typed = parseNumberInput(input.value);
    const typing = input.ownerDocument.activeElement === input;
    if (typed === value || (typing && (typed === undefined || input.validity?.badInput))) return noChange;
    return this.render(value);
  }
}

const numberValue = directive(NumberValue);

/** Writes on every keystroke that is a number (or a cleared field); a half-typed one writes nothing. */
const numberWidget: WidgetRenderer = (ctx) =>
  html`<input
    type="number"
    step="any"
    class="fm-in"
    aria-label=${ctx.label}
    .value=${numberValue(ctx.value)}
    @input=${(e: Event) => {
      const input = e.target as HTMLInputElement;
      // A browser reports `1.` or `-` as an empty value with `badInput`: not a clear.
      if (input.validity?.badInput) return;
      const n = parseNumberInput(input.value);
      if (n !== undefined) ctx.host.apply((m) => setScalar(m, ctx.path, n));
    }}
  />`;

const boolWidget: WidgetRenderer = (ctx) =>
  html`<input
    type="checkbox"
    class="fm-check"
    role="switch"
    aria-label=${ctx.label}
    .checked=${live(ctx.value === true)}
    @change=${(e: Event) => ctx.host.apply((m) => setScalar(m, ctx.path, (e.target as HTMLInputElement).checked))}
  />`;

const dateWidget: WidgetRenderer = (ctx) => {
  const v = str(ctx.value).trim();
  return html`<input
    type="date"
    class="fm-in"
    aria-label=${ctx.label}
    .value=${live(/^\d{4}-\d{2}-\d{2}$/.test(v) ? v : '')}
    @input=${(e: Event) => writeScalar(ctx, inputValue(e))}
  />`;
};

/**
 * A timestamp: the wall-clock time in its own offset, which is shown beside
 * the field and kept on every edit (a value without one gets the local
 * offset, as OKF requires one). `applyDatetimeLocal` keeps the stored
 * spelling (separator, seconds).
 */
const datetimeWidget: WidgetRenderer = (ctx) => {
  const stored = str(ctx.value);
  const parts = parseDateTime(stored);
  const offset = parts?.offset ?? formatOffset(localOffsetMinutes());
  const shown = offset === 'Z' || offset === 'z' ? 'UTC' : `UTC${offset}`;
  return html`<span class="fm-inline">
    <input
      type="datetime-local"
      class="fm-in"
      step=${parts?.second !== undefined ? '1' : '60'}
      aria-label=${ctx.label}
      .value=${live(stored ? datetimeLocalValue(stored) : '')}
      @input=${(e: Event) => {
        const typed = inputValue(e);
        if (typed === '') {
          writeScalar(ctx, null);
          return;
        }
        const next = applyDatetimeLocal(typed, stored || undefined);
        if (next !== null) writeScalar(ctx, next);
      }}
    />
    <span class="fm-offset" title=${parts?.offset ? 'The offset written with this time' : 'No offset yet: your local one is added'}>${shown}</span>
  </span>`;
};

const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];

/**
 * Year, month and day, each optional (`1927`, `1927-03`, `--03-19`): the
 * contact dates `PartialDate` reads. An impossible combination (a day
 * without a month) writes nothing and says why.
 */
const partialDateWidget: WidgetRenderer = (ctx) => {
  const fields = partialDateFields(ctx.value);
  const update = (e: Event) => {
    const group = (e.target as HTMLElement).closest('.fm-partial');
    if (!group) return;
    const get = (name: string) => (group.querySelector(`[data-part="${name}"]`) as HTMLInputElement | HTMLSelectElement | null)?.value ?? '';
    const typed = { year: get('year'), month: get('month'), day: get('day') };
    // A year being typed is not wrong yet.
    if (/^\d{1,3}$/.test(typed.year.trim())) return;
    const result = partialDateValue(typed, ctx.value);
    if (!result.ok) {
      ctx.host.setLocalHint(ctx.path, result.message);
      return;
    }
    ctx.host.setLocalHint(ctx.path, null);
    ctx.host.apply((m) => setScalar(m, ctx.path, result.value as FmScalar));
  };
  return html`<span class="fm-partial" role="group" aria-label=${ctx.label}>
    <input
      type="text"
      inputmode="numeric"
      maxlength="4"
      placeholder="Year"
      class="fm-in"
      data-part="year"
      aria-label="Year"
      .value=${live(fields.year)}
      @input=${update}
    />
    <select class="fm-in" data-part="month" aria-label="Month" .value=${live(fields.month)} @change=${update}>
      <option value="">Month</option>
      ${MONTHS.map((m, i) => {
        const v = String(i + 1).padStart(2, '0');
        return html`<option value=${v} ?selected=${v === fields.month}>${m}</option>`;
      })}
    </select>
    <select class="fm-in" data-part="day" aria-label="Day" .value=${live(fields.day)} @change=${update}>
      <option value="">Day</option>
      ${Array.from({ length: 31 }, (_, i) => {
        const v = String(i + 1).padStart(2, '0');
        return html`<option value=${v} ?selected=${v === fields.day}>${i + 1}</option>`;
      })}
    </select>
  </span>`;
};

/** A known set of values plus free text: the value is written as typed. */
const enumWidget: WidgetRenderer = (ctx) =>
  html`<mbr-fm-combobox
    openOnFocus
    .value=${str(ctx.value)}
    .options=${ctx.host.suggestions(ctx.path, ctx.def, ctx.kind)}
    label=${ctx.label}
    @fm-combo-input=${(e: CustomEvent<{ value: string }>) => writeScalar(ctx, e.detail.value)}
    @fm-combo-commit=${(e: CustomEvent<ComboCommitDetail>) => writeScalar(ctx, e.detail.value)}
  ></mbr-fm-combobox>`;

/** The definition and kind for item `i` of a scalar list. */
function itemContext(ctx: WidgetContext, item: FmValue, i: number): WidgetContext {
  const def: FieldDef = ctx.def.item ?? { key: '', kind: 'text', label: 'Item', group: ctx.def.group };
  const kind: WidgetKind = typeof item === 'number' ? 'number' : typeof item === 'boolean' ? 'bool' : def.kind === 'enum' ? 'enum' : 'text';
  return { ...ctx, path: [...ctx.path, i], value: item, def, kind, label: `${ctx.label}, item ${i + 1}` };
}

/** A list of scalars: one field per item; Enter in the last one adds the next. */
const listWidget: WidgetRenderer = (ctx) => {
  const items = Array.isArray(ctx.value) ? ctx.value : [];
  const add = () => {
    if (ctx.host.apply((m) => addItem(m, ctx.path, null))) ctx.host.focusAfterUpdate(ctx.path, items.length);
  };
  const onKeydown = (e: KeyboardEvent) => {
    if (e.key !== 'Enter' || e.isComposing) return;
    const target = e.target as HTMLElement;
    if (!(target instanceof HTMLInputElement) || target.type !== 'text') return;
    const li = target.closest('li[data-index]') as HTMLElement | null;
    if (!li) return;
    e.preventDefault();
    const index = Number(li.dataset.index);
    if (index === items.length - 1) add();
    else (li.nextElementSibling?.querySelector('input') as HTMLInputElement | null)?.focus();
  };
  return html`<ol class="fm-list" aria-label=${ctx.label} @keydown=${onKeydown}>
      ${items.map(
        (item, i) =>
          html`<li data-index=${i}>
            ${ctx.host.renderScalar(itemContext(ctx, item, i))}
            <button
              type="button"
              class="mbr-icon-btn sm"
              aria-label=${`Remove item ${i + 1}`}
              title="Remove"
              @click=${() => ctx.host.removeListItem(ctx.path, i)}
            >
              ${icon(iconX, { size: ICON_SM })}
            </button>
          </li>`,
      )}
    </ol>
    <button type="button" class="fm-link" @click=${add}>${icon(iconPlus, { size: ICON_SM })}Add item</button>`;
};

/**
 * Tags: chips, and a field that adds one on Enter or a comma, with the
 * vault's tags as suggestions. A tag field written as one comma-separated
 * string (repo.rs reads both) stays a string: chips are its parts, and an
 * edit rewrites only that scalar.
 */
const tagsWidget: WidgetRenderer = (ctx) => {
  const items = tagItems(ctx.value);
  const present = new Set(items);
  const options = ctx.host.suggestions(ctx.path, ctx.def, ctx.kind).filter((o) => !present.has(o.value));
  const add = (raw: string) => {
    const tag = raw.trim();
    if (!tag || present.has(tag)) return;
    // The field keeps focus (it is not re-created), ready for the next tag.
    ctx.host.apply((m) => addTag(m, ctx.path, ctx.value, tag));
  };
  const remove = (i: number) => ctx.host.apply((m) => removeTag(m, ctx.path, ctx.value, i));
  return html`<div class="fm-chips">
    ${items.map(
      (item, i) =>
        html`<span class="fm-chip"
          ><span>${item}</span
          ><button type="button" class="mbr-icon-btn sm" aria-label=${`Remove ${item}`} title="Remove" @click=${() => remove(i)}>
            ${icon(iconX, { size: 14 })}
          </button></span
        >`,
    )}
    <mbr-fm-combobox
      class="fm-tag-entry"
      .options=${options}
      label=${`Add to ${ctx.label}`}
      placeholder=${items.length === 0 ? 'Add a tag' : ''}
      separators=","
      clearOnCommit
      @fm-combo-commit=${(e: CustomEvent<ComboCommitDetail>) => add(e.detail.value)}
      @fm-combo-backspace-empty=${() => {
        if (items.length > 0) remove(items.length - 1);
      }}
    ></mbr-fm-combobox>
  </div>`;
};

const treeWidget: WidgetRenderer = (ctx) => ctx.host.renderTreeSummary(ctx);

/** The generic widgets, by kind. */
export const GENERIC_WIDGETS: WidgetTable = {
  text: textWidget,
  longtext: longtextWidget,
  number: numberWidget,
  bool: boolWidget,
  date: dateWidget,
  datetime: datetimeWidget,
  partialDate: partialDateWidget,
  enum: enumWidget,
  wikilink: enumWidget,
  actor: enumWidget,
  list: listWidget,
  tags: tagsWidget,
  tree: treeWidget,
};

/** The renderer for `kind`, with the fallbacks: collections draw as a tree, other scalars as text. */
export function widgetFor(table: WidgetTable, kind: WidgetKind, value: FmValue | undefined): WidgetRenderer {
  const own = table[kind];
  if (own) return own;
  const collection = kind === 'map' || kind === 'relationships' || kind === 'labeled' || kind === 'list' || kind === 'tags';
  if (collection || (value !== null && typeof value === 'object')) return table.tree ?? treeWidget;
  return table.text ?? textWidget;
}
