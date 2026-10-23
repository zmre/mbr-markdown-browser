/**
 * The properties form's widgets for mbr's and OKF's own vocabulary:
 *
 * - `relationships`: one row per edge — type (from the registry), the note
 *   (`to`: "X is my <type>"), an optional label — plus a "Flip relationship
 *   direction" checkbox (checked = `from`; none for a symmetric type) and the
 *   edge read back as a sentence, which says what the direction means. An edge naming two
 *   other notes shows both endpoints, labelled from/to. Other keys on an
 *   edge (`married:`, `place:`) stay generic rows under it.
 * - `labeled`: contact values with a label (`emails`, `phones`, `dates`,
 *   `addresses`, `aliases`, …) in whichever of contact.rs's three shapes the
 *   note uses; new entries follow that shape.
 * - `wikilink`: a note picker writing `"[[Title]]"`, free text allowed.
 * - `actor`: OKF actors (`human:`, `process:`, `agent/version`) with a "me"
 *   shortcut.
 *
 * Registered by spreading {@link MBR_WIDGETS} over the generic table; the
 * form never names them. Every edit is a `model.ts` operation on the node
 * that changes (see `mbr-logic.ts`).
 */

import { html, nothing, type TemplateResult } from 'lit';
import { live } from 'lit/directives/live.js';
import { addItem, addKey, getIn, nodeInfo, setScalar, type FmPath, type FmValue } from './model.js';
import { CONTACT_LABELS, type FieldDef } from './fields.js';
import { widgetKindFor } from './form-logic.js';
import {
  ADDRESS_PARTS,
  DEFAULT_RELATIONSHIP_TYPES,
  FLIP_DIRECTION_LABEL,
  RESERVED_EDGE_KEYS,
  addLabeledEntry,
  canFlipDirection,
  edgeDirection,
  edgeOf,
  freshLabel,
  humanActorId,
  labeledEntries,
  newEdge,
  relationTypeOptions,
  relationType,
  relationshipSentence,
  removeEntry,
  setEdgeField,
  setEntryLabel,
  toggleEdgeDirection,
  type EdgeView,
  type LabeledEntry,
} from './mbr-logic.js';
import { GENERIC_WIDGETS, type WidgetContext, type WidgetRenderer, type WidgetTable } from './fm-widgets.js';
import type { ComboCommitDetail, MbrFmCombobox } from './fm-combobox.js';
import './fm-combobox.js';
import type { RelationTypeConfig } from '../graph/relationship-graph.js';
import { ICON_SM, icon, iconPlus, iconTriangleAlert, iconUser, iconX } from '../icons.js';

const str = (v: FmValue | undefined): string => (v === null || v === undefined || typeof v === 'object' ? '' : String(v));

/** The text in a combobox when focus leaves it (its value is only committed on Enter/pick). */
const comboText = (e: FocusEvent): string => ((e.currentTarget as MbrFmCombobox).querySelector('input')?.value ?? '').trim();

/** Whether focus left `e.currentTarget` for good (not for its own list). */
const leftFor = (e: FocusEvent): boolean => {
  const to = e.relatedTarget as Node | null;
  return !to || !(e.currentTarget as HTMLElement).contains(to);
};

const removeButton = (label: string, run: () => void): TemplateResult =>
  html`<button type="button" class="mbr-icon-btn sm" aria-label=${label} title="Remove" @click=${run}>${icon(iconX, { size: ICON_SM })}</button>`;

// ============================================================================
// Note link
// ============================================================================

/** A note picker: suggestions are every note as `[[Title]]` (path as detail); free text stays allowed. */
const wikilinkWidget: WidgetRenderer = (ctx) =>
  html`<mbr-fm-combobox
    class="fm-wikilink"
    openOnFocus
    placeholder="[[Note]]"
    .value=${str(ctx.value)}
    .options=${ctx.host.suggestions(ctx.path, ctx.def, 'wikilink')}
    label=${ctx.label}
    @fm-combo-input=${(e: CustomEvent<{ value: string }>) => ctx.host.apply((m) => setScalar(m, ctx.path, e.detail.value || null))}
    @fm-combo-commit=${(e: CustomEvent<ComboCommitDetail>) => ctx.host.apply((m) => setScalar(m, ctx.path, e.detail.value || null))}
  ></mbr-fm-combobox>`;

// ============================================================================
// Actor
// ============================================================================

/**
 * An OKF actor. "Me" writes `human:<id>` once an id is known; the first
 * `human:<id>` the reader commits (Enter, a pick, or leaving the field)
 * becomes it, for this page only.
 */
const actorWidget: WidgetRenderer = (ctx) => {
  const memory = ctx.host.actorMemory;
  const write = (value: string) => ctx.host.apply((m) => setScalar(m, ctx.path, value || null));
  const remember = (value: string) => {
    const id = humanActorId(value);
    if (id && !memory.id) memory.id = id;
  };
  const me = (e: Event) => {
    if (memory.id) {
      write(`human:${memory.id}`);
      ctx.host.setLocalHint(ctx.path, null);
      return;
    }
    // No id yet: prime the field (`human:` alone is not worth writing) and ask for it.
    const combo = (e.currentTarget as HTMLElement).parentElement?.querySelector<MbrFmCombobox>('mbr-fm-combobox');
    if (combo) {
      combo.value = 'human:';
      combo.focus();
    }
    ctx.host.setLocalHint(ctx.path, 'Type your id after human: — it is remembered on this page for the next “Me”.');
  };
  return html`<span class="fm-inline">
    <mbr-fm-combobox
      openOnFocus
      placeholder="human:<id>, process:<id> or agent/version"
      .value=${str(ctx.value)}
      .options=${ctx.host.suggestions(ctx.path, ctx.def, 'actor')}
      label=${ctx.label}
      @fm-combo-input=${(e: CustomEvent<{ value: string }>) => write(e.detail.value)}
      @fm-combo-commit=${(e: CustomEvent<ComboCommitDetail>) => {
        write(e.detail.value);
        remember(e.detail.value);
        if (memory.id) ctx.host.setLocalHint(ctx.path, null);
      }}
      @focusout=${(e: FocusEvent) => {
        if (!leftFor(e)) return;
        remember(comboText(e));
        if (memory.id) ctx.host.setLocalHint(ctx.path, null);
      }}
    ></mbr-fm-combobox>
    <button
      type="button"
      class="fm-link fm-me"
      title=${memory.id ? `Fill in human:${memory.id}` : 'Fill in human: and your id'}
      @click=${me}
    >
      ${icon(iconUser, { size: ICON_SM })}Me
    </button>
  </span>`;
};

// ============================================================================
// Relationships
// ============================================================================

/** Attribute names worth suggesting on an edge (docs/markdown/relationships.md). */
const EDGE_ATTRIBUTES = ['married', 'divorced', 'since', 'until', 'place', 'note'];

interface Registry {
  readonly types: readonly RelationTypeConfig[];
  /** Whether the vault has its own (else these are mbr's defaults, and nothing is "not in" them). */
  readonly authored: boolean;
}

function renderEdge(ctx: WidgetContext, edge: EdgeView, index: number, registry: Registry, me: string): TemplateResult {
  const host = ctx.host;
  const edgePath = [...ctx.path, index];
  const dir = edgeDirection(edge);
  const known = relationType(registry.types, edge.type);
  const sentence = relationshipSentence(edge, registry.types, me);
  const flippable = canFlipDirection(edge, registry.types);
  const n = index + 1;
  const endpoint = (key: 'to' | 'from', label: string, extra = '') =>
    html`<mbr-fm-combobox
      class=${`fm-wikilink${extra}`}
      openOnFocus
      placeholder="[[Note]]"
      .value=${key === 'to' ? edge.to : edge.from}
      .options=${host.suggestions([...edgePath, key], { key, kind: 'wikilink', label, group: 'mbr' }, 'wikilink')}
      label=${`Relationship ${n}, ${label}`}
      @fm-combo-input=${(e: CustomEvent<{ value: string }>) => host.apply((m) => setEdgeField(m, edgePath, key, e.detail.value))}
      @fm-combo-commit=${(e: CustomEvent<ComboCommitDetail>) => host.apply((m) => setEdgeField(m, edgePath, key, e.detail.value))}
    ></mbr-fm-combobox>`;
  return html`<li class="fm-edge" data-index=${index}>
    <div class=${dir === 'both' ? 'fm-edge-main fm-edge-both' : 'fm-edge-main'}>
      <mbr-fm-combobox
        class="fm-edge-type"
        openOnFocus
        placeholder="type"
        .value=${edge.type}
        .options=${relationTypeOptions(registry.types)}
        label=${`Relationship ${n}, type`}
        @fm-combo-input=${(e: CustomEvent<{ value: string }>) => host.apply((m) => setEdgeField(m, edgePath, 'type', e.detail.value))}
        @fm-combo-commit=${(e: CustomEvent<ComboCommitDetail>) => host.apply((m) => setEdgeField(m, edgePath, 'type', e.detail.value))}
      ></mbr-fm-combobox>
      ${dir === 'both'
        ? html`<span class="fm-edge-dir-text fm-edge-from-text">from</span>${endpoint('from', 'from', ' fm-edge-from')}<span
              class="fm-edge-dir-text fm-edge-to-text"
              >to</span
            >${endpoint('to', 'to', ' fm-edge-to')}`
        : endpoint(dir, 'note')}
      <input
        type="text"
        class="fm-in fm-edge-label"
        placeholder="label"
        aria-label=${`Relationship ${n}, label (optional)`}
        .value=${live(edge.label)}
        @input=${(e: Event) => host.apply((m) => setEdgeField(m, edgePath, 'label', (e.target as HTMLInputElement).value))}
      />
      ${removeButton(`Remove relationship ${n}`, () => host.removeListItem(ctx.path, index))}
    </div>
    ${flippable
      ? html`<label class="fm-edge-invert">
          <input
            type="checkbox"
            aria-label=${`Relationship ${n}: flip relationship direction`}
            .checked=${live(dir === 'from')}
            @change=${(e: Event) => {
              if ((e.target as HTMLInputElement).checked !== (dir === 'from')) host.apply((m) => toggleEdgeDirection(m, edgePath, edge));
            }}
          />${FLIP_DIRECTION_LABEL}
        </label>`
      : nothing}
    ${sentence ? html`<p class="fm-edge-sentence">${sentence}</p>` : nothing}
    ${edge.type && registry.authored && !known
      ? html`<p class="fm-hint">${icon(iconTriangleAlert, { size: 14 })}“${edge.type}” is not in relationship_types: no reverse edge is derived.</p>`
      : nothing}
    ${edge.attributes.length > 0
      ? html`<div class="fm-children" role="list">${host.renderChildRows(edgePath, (k) => typeof k === 'string' && !RESERVED_EDGE_KEYS.includes(k))}</div>`
      : nothing}
    <div class="fm-edge-add">
      <mbr-fm-combobox
        placeholder="Add attribute"
        clearOnCommit
        .options=${EDGE_ATTRIBUTES.filter((a) => !edge.attributes.includes(a)).map((value) => ({ value }))}
        label=${`Relationship ${n}, add attribute`}
        @fm-combo-commit=${(e: CustomEvent<ComboCommitDetail>) => {
          const key = e.detail.value.trim();
          if (key && !RESERVED_EDGE_KEYS.includes(key)) host.apply((m) => addKey(m, edgePath, key, null));
        }}
      ></mbr-fm-combobox>
    </div>
  </li>`;
}

/** The registry to offer: the vault's, else mbr's defaults (`authored` says which). */
function registryOf(ctx: WidgetContext): Registry {
  const own = ctx.host.catalog.relationshipTypes;
  return { types: own.length > 0 ? own : DEFAULT_RELATIONSHIP_TYPES, authored: own.length > 0 };
}

const relationshipsWidget: WidgetRenderer = (ctx) => {
  const items = Array.isArray(ctx.value) ? ctx.value : [];
  const registry = registryOf(ctx);
  const title = ctx.host.frontmatter.title;
  const me = typeof title === 'string' && title.trim() ? title.trim() : 'this note';
  const add = () => {
    if (ctx.host.apply((m) => addItem(m, ctx.path, newEdge()))) ctx.host.focusAfterUpdate(ctx.path, items.length);
  };
  return html`<ol class="fm-edges" aria-label=${ctx.label}>
      ${items.map((item, i) => {
        const edge = edgeOf(item);
        if (edge) return renderEdge(ctx, edge, i, registry, me);
        return html`<li class="fm-edge" data-index=${i}>
          <div class="fm-children" role="list">${ctx.host.renderChildRows(ctx.path, (k) => k === i)}</div>
        </li>`;
      })}
    </ol>
    <button type="button" class="fm-link" @click=${add}>${icon(iconPlus, { size: ICON_SM })}Add relationship</button>`;
};

// ============================================================================
// Labeled values
// ============================================================================

/** The key a labeled field is filed under (`emails`, `dates`, …). */
const fieldName = (path: FmPath): string => {
  const last = [...path].reverse().find((s): s is string => typeof s === 'string');
  return last ?? '';
};

/** A structured address: one field per part, a missing part added when typed. */
function renderAddress(ctx: WidgetContext, entry: LabeledEntry, n: number): TemplateResult {
  const value = entry.value;
  if (typeof value === 'string') {
    return html`<textarea
      class="fm-in"
      rows=${Math.min(6, Math.max(2, value.split('\n').length))}
      aria-label=${`${ctx.label} ${n}`}
      .value=${live(value)}
      @input=${(e: Event) => ctx.host.apply((m) => setScalar(m, entry.valuePath, (e.target as HTMLTextAreaElement).value || null))}
    ></textarea>`;
  }
  const parts = value && typeof value === 'object' && !Array.isArray(value) ? value : {};
  const placeholder: Record<string, string> = { street: 'Street', city: 'City', region: 'Region', postcode: 'Postcode', country: 'Country', country_code: 'Code' };
  const write = (part: string, text: string) => {
    // A cleared part stays as `part:` so the address keeps its layout.
    ctx.host.apply((m) => setScalar(m, [...entry.valuePath, part], text === '' ? null : text));
  };
  return html`<span class="fm-address" role="group" aria-label=${`${ctx.label} ${n}`}>
    ${ADDRESS_PARTS.map((part) =>
      part === 'street'
        ? html`<textarea
            class="fm-in fm-address-street"
            rows="2"
            placeholder=${placeholder[part]}
            aria-label=${placeholder[part]}
            .value=${live(str(parts[part]))}
            @input=${(e: Event) => write(part, (e.target as HTMLTextAreaElement).value)}
          ></textarea>`
        : html`<input
            type="text"
            class=${`fm-in fm-address-${part}`}
            placeholder=${placeholder[part]}
            aria-label=${placeholder[part]}
            .value=${live(str(parts[part]))}
            @input=${(e: Event) => write(part, (e.target as HTMLInputElement).value)}
          />`,
    )}
  </span>`;
}

const labeledWidget: WidgetRenderer = (ctx) => {
  const host = ctx.host;
  const field = fieldName(ctx.path);
  const entries = labeledEntries(ctx.value ?? null, ctx.path, field);
  const suggested = ctx.def.labels ?? CONTACT_LABELS[field] ?? [];
  const itemDef: FieldDef = ctx.def.item ?? { key: '', kind: 'text', label: 'Value', group: ctx.def.group };
  const used = entries.filter((e): e is LabeledEntry => e !== null).map((e) => e.label);
  const relabel = (i: number, label: string) =>
    host.apply((m) => {
      // Re-read: an earlier relabel (commit, then focus leaving) moved the entry.
      const current = labeledEntries(getIn(m, ctx.path) ?? null, ctx.path, field)[i];
      return current ? setEntryLabel(m, ctx.path, current, label) : m;
    });
  const add = () => {
    const label = freshLabel(used, suggested);
    const value: FmValue = null;
    if (host.apply((m) => addLabeledEntry(m, ctx.path, getIn(m, ctx.path), field, label, value))) {
      host.focusAfterUpdate(ctx.path, entries.length);
    }
  };
  const valueWidget = (entry: LabeledEntry, i: number): TemplateResult => {
    if (field === 'addresses' && itemDef.kind === 'map' && (entry.value === null || typeof entry.value === 'object' || typeof entry.value === 'string')) {
      return renderAddress(ctx, entry, i + 1);
    }
    if (entry.value !== null && typeof entry.value === 'object') {
      return host.renderChildRows(entry.valuePath.slice(0, -1), (k) => k === entry.valuePath[entry.valuePath.length - 1]);
    }
    const kind = widgetKindFor(itemDef, entry.value);
    return host.renderScalar({
      host,
      path: entry.valuePath,
      value: entry.value,
      def: itemDef,
      kind,
      info: nodeInfo(host.model, entry.valuePath),
      label: `${ctx.label}, ${entry.label || `item ${i + 1}`}`,
    });
  };
  return html`<ol class="fm-labeled" aria-label=${ctx.label}>
      ${entries.map((entry, i) => {
        if (!entry) {
          return html`<li data-index=${i} class="fm-labeled-raw">
            <div class="fm-children" role="list">${host.renderChildRows(ctx.path, (k) => k === i)}</div>
          </li>`;
        }
        const n = i + 1;
        return html`<li data-index=${i}>
          <mbr-fm-combobox
            class="fm-label"
            openOnFocus
            placeholder="label"
            .value=${entry.label}
            .options=${suggested.filter((l) => l === entry.label || !used.includes(l)).map((value) => ({ value }))}
            label=${`${ctx.label} ${n}, label`}
            @fm-combo-commit=${(e: CustomEvent<ComboCommitDetail>) => relabel(i, e.detail.value)}
            @focusout=${(e: FocusEvent) => {
              if (leftFor(e)) relabel(i, comboText(e));
            }}
          ></mbr-fm-combobox>
          <div class="fm-labeled-value">${valueWidget(entry, i)}</div>
          ${removeButton(`Remove ${entry.label || `item ${n}`}`, () =>
            host.apply((m) => {
              const current = labeledEntries(getIn(m, ctx.path) ?? null, ctx.path, field)[i];
              return current ? removeEntry(m, ctx.path, current) : m;
            }),
          )}
        </li>`;
      })}
    </ol>
    <button type="button" class="fm-link" @click=${add}>${icon(iconPlus, { size: ICON_SM })}Add ${itemDef.label.toLowerCase()}</button>`;
};

/** The generic widgets plus mbr's and OKF's. */
export const MBR_WIDGETS: WidgetTable = {
  ...GENERIC_WIDGETS,
  wikilink: wikilinkWidget,
  actor: actorWidget,
  relationships: relationshipsWidget,
  labeled: labeledWidget,
};
