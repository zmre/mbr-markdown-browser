/**
 * `<mbr-markdown-field>`: mbr's markdown editor as a form field, for pages mbr
 * did not render (`mbr-markdown-field.min.js`, entry `index.ts`).
 *
 * The same editor core as `<mbr-editor>` (`editor-core.ts`: Crepe, mbr's
 * syntax plugins, the minimal-diff save path), so wikilinks, alerts, math,
 * definition lists and the rest round-trip exactly as they do there, and it
 * never writes HTML the author did not type. What it leaves out is everything
 * that belongs to a page or a file: no `window.frontmatter`, `shared.ts`, edit
 * token, live reload or save endpoint, no frontmatter form. **The host owns
 * the value**: it sets `value`, listens for `change`, and saves however it
 * likes. The one network request it can make is the optional wikilink catalog
 * (`catalog-url`), and only when the reader types `[[`.
 *
 * Light DOM, like `<mbr-editor>`: ProseMirror's selection handling inside a
 * shadow root is unreliable in WebKit, and Crepe's stylesheet is document
 * scoped. Styles are injected once into `<head>`, every selector starting at
 * `mbr-markdown-field`; theming is `--mbr-field-*` (see `styles.ts`).
 *
 * A plain `HTMLElement` rather than Lit: it renders one wrapper and hands it
 * to Crepe, so a template engine would add weight and nothing else.
 */

import type { Crepe } from '@milkdown/crepe';
import { CrepeFeature } from '@milkdown/crepe';
import { editorViewCtx } from '@milkdown/kit/core';
import type { MilkdownPlugin } from '@milkdown/kit/ctx';
import { Plugin, PluginKey } from '@milkdown/kit/prose/state';
import type { EditorView } from '@milkdown/kit/prose/view';
import { $prose } from '@milkdown/kit/utils';
import {
  CREPE_BASE_CSS,
  EDITOR_THEME_CSS,
  FOOTNOTE_CSS,
  createBodySaver,
  createMarkdownCrepe,
  memoizeLatest,
  type BodySaver,
} from '../editor-core.js';
import { lineEndingOf, toLf, withAuthoredEnd, withLineEnding, type LineEnding } from '../editor-frontmatter.js';
import {
  AUTOCOMPLETE_CSS,
  createLinkAutocompletePlugin,
  type LinkAutocompleteSite,
} from '../editor-link-autocomplete.js';
import { DEFINITION_LIST_CSS, addDefinitionListMenuItem } from '../editor-plugins/definition-list.js';
import { POPOVER_HOST_ATTR } from '../editor-plugins/popover-host.js';
import { FIELD_CSS } from './styles.js';

/** The element's tag name. */
export const MARKDOWN_FIELD_TAG = 'mbr-markdown-field';

/** `change`'s detail: the field's markdown, as `value` would return it. */
export interface MarkdownFieldChangeDetail {
  value: string;
}

/** A host-supplied image uploader: stores `file` somewhere and returns the URL to write. */
export type MarkdownFieldUploader = (file: File) => Promise<string>;

/**
 * How long typing may run before `change` fires (ms). Reading `value` is
 * always current; this only spaces the events out. A pending `change` is
 * always sent before `blur`.
 */
export const CHANGE_DEBOUNCE_MS = 120;

const STYLE_ID = 'mbr-markdown-field-styles';

/** Attributes mirrored onto properties. `value` sets the value; it is never written back. */
const OBSERVED = ['value', 'readonly', 'placeholder', 'catalog-url', 'link-base'] as const;

/** Properties a framework may set on the element before its class is defined. */
const UPGRADED = ['value', 'readonly', 'placeholder', 'catalogUrl', 'linkBase', 'uploadImage'] as const;

function injectStyles(): void {
  if (document.getElementById(STYLE_ID)) return;
  const style = document.createElement('style');
  style.id = STYLE_ID;
  // Crepe's structural CSS, the Crepe→Pico mapping `<mbr-editor>` uses, the
  // node styles the plugins need, then the field's own rules last so they win.
  style.textContent = [
    CREPE_BASE_CSS,
    EDITOR_THEME_CSS,
    FOOTNOTE_CSS,
    DEFINITION_LIST_CSS,
    AUTOCOMPLETE_CSS,
    FIELD_CSS,
  ].join('\n');
  document.head.appendChild(style);
}

/**
 * The note list a catalog URL returns: site.json (`{markdown_files: […]}`,
 * e.g. mbr's own `/.mbr/site.json`) or a bare array of the same entries
 * (`{url_path, frontmatter?: {title?}}`).
 */
async function fetchCatalog(url: string): Promise<LinkAutocompleteSite[]> {
  const resp = await fetch(url, { credentials: 'same-origin' });
  if (!resp.ok) throw new Error(`wikilink catalog ${resp.status}`);
  const data = (await resp.json()) as unknown;
  if (Array.isArray(data)) return data as LinkAutocompleteSite[];
  const files = (data as { markdown_files?: unknown } | null)?.markdown_files;
  return Array.isArray(files) ? (files as LinkAutocompleteSite[]) : [];
}

/** A ProseMirror plugin calling `onChange` after every transaction that changes the document. */
function docChangePlugin(onChange: () => void): MilkdownPlugin {
  return $prose(
    () =>
      new Plugin({
        key: new PluginKey('mbr-markdown-field-change'),
        view: () => ({
          update: (view, prev) => {
            if (!view.state.doc.eq(prev.doc)) onChange();
          },
        }),
      }),
  );
}

/** Test seam: the live editor view of a field (or `null` before it mounts). Not part of the element's API. */
const viewOfField = new WeakMap<MbrMarkdownField, () => EditorView | null>();

/** The ProseMirror view behind `field`, for tests and diagnostics. */
export function fieldEditorView(field: MbrMarkdownField): EditorView | null {
  return viewOfField.get(field)?.() ?? null;
}

/**
 * The editor as a form field. See the module comment and
 * `docs/customization/markdown-field.md`.
 *
 * @fires change - `CustomEvent<{value: string}>`, after the reader edits
 *   (spaced by {@link CHANGE_DEBOUNCE_MS}, always before `blur`); never for a
 *   `value` the host set.
 * @fires blur - `Event`, when focus leaves the field (its menus and popovers
 *   count as inside).
 */
export class MbrMarkdownField extends HTMLElement {
  static get observedAttributes(): readonly string[] {
    return OBSERVED;
  }

  /** The value as the host set it or as last computed from the editor. */
  #value = '';
  /** The editor's document changed since {@link #value} was computed. */
  #dirty = false;
  /** The value the last `change` carried (or the host set): no event repeats it. */
  #reported = '';
  #readonly = false;
  #placeholder = '';
  #catalogUrl = '';
  #linkBase = '/';
  #uploadImage: MarkdownFieldUploader | null = null;

  /** What the current editor was opened on: the body (LF) and the value's line ending. */
  #opened: { body: string; ending: LineEnding } = { body: '', ending: '\n' };
  #crepe: Crepe | null = null;
  #saver: BodySaver | null = null;
  #wrapper: HTMLDivElement | null = null;
  /** Bumped by every (re)mount and teardown; a mount that lost the race destroys its editor. */
  #generation = 0;
  #mountQueued = false;
  #ready: Promise<void> = Promise.resolve();
  #changeTimer: ReturnType<typeof setTimeout> | null = null;

  constructor() {
    super();
    viewOfField.set(this, () => this.#crepe?.editor.action((ctx) => ctx.get(editorViewCtx)) ?? null);
  }

  // ---------------------------------------------------------------------------
  // Properties
  // ---------------------------------------------------------------------------

  /**
   * The markdown. Reading it is always current. An edit the reader has not
   * touched comes back exactly as set; edited, the parts the reader did not
   * change are still written as they were set (`createBodySaver`). Line
   * endings follow the value that was set (CRLF in, CRLF out).
   *
   * Setting it replaces the document (and its undo history) unless it equals
   * the current value — so a two-way binding that writes back each `change`
   * never resets the caret. It fires no `change`.
   */
  get value(): string {
    if (this.#dirty && this.#saver) {
      this.#value = this.#serialize(this.#saver);
      this.#dirty = false;
    }
    return this.#value;
  }

  set value(next: string) {
    const text = next == null ? '' : String(next);
    if (text === this.value) return;
    this.#cancelChange();
    this.#value = text;
    this.#reported = text;
    this.#dirty = false;
    this.#scheduleMount();
  }

  /** Whether the reader may edit. Reflects the `readonly` attribute. */
  get readonly(): boolean {
    return this.#readonly;
  }

  set readonly(next: boolean) {
    const on = Boolean(next);
    if (on !== this.hasAttribute('readonly')) this.toggleAttribute('readonly', on);
    this.#applyReadonly(on);
  }

  /** Shown while the field is empty. Reflects the `placeholder` attribute. */
  get placeholder(): string {
    return this.#placeholder;
  }

  set placeholder(next: string) {
    const text = next == null ? '' : String(next);
    if (text !== (this.getAttribute('placeholder') ?? '')) this.setAttribute('placeholder', text);
    if (text === this.#placeholder) return;
    this.#placeholder = text;
    // Crepe reads its placeholder once, at create: re-open (keeping the value).
    if (this.#crepe) this.#remount();
  }

  /**
   * Where the `[[` autocomplete gets its notes: a URL returning site.json
   * (e.g. `/.mbr/site.json`) or a bare array of `{url_path, frontmatter: {title}}`.
   * Fetched (with same-origin credentials) each time the reader types `[[`.
   * Empty: no autocomplete, and the field makes no request at all.
   * Reflects the `catalog-url` attribute.
   */
  get catalogUrl(): string {
    return this.#catalogUrl;
  }

  set catalogUrl(next: string) {
    const url = next == null ? '' : String(next);
    if (url !== (this.getAttribute('catalog-url') ?? '')) {
      if (url) this.setAttribute('catalog-url', url);
      else this.removeAttribute('catalog-url');
    }
    if (url === this.#catalogUrl) return;
    const toggled = !url !== !this.#catalogUrl;
    this.#catalogUrl = url;
    // The autocomplete plugin is installed only with a catalog; the URL itself is read per `[[`.
    if (toggled && this.#crepe) this.#remount();
  }

  /**
   * The URL of the note being edited, which the autocomplete's relative-link
   * mode (Tab in the `[[` popup) links from. Default `/`. Reflects `link-base`.
   */
  get linkBase(): string {
    return this.#linkBase;
  }

  set linkBase(next: string) {
    const base = next == null || next === '' ? '/' : String(next);
    if (base !== (this.getAttribute('link-base') ?? '/')) this.setAttribute('link-base', base);
    this.#linkBase = base;
  }

  /**
   * Stores a pasted, dropped or picked image and returns the URL to write
   * (property only). Without one, image uploads are refused: the field never
   * writes a `blob:` URL into the host's markdown.
   */
  get uploadImage(): MarkdownFieldUploader | null {
    return this.#uploadImage;
  }

  set uploadImage(next: MarkdownFieldUploader | null) {
    this.#uploadImage = typeof next === 'function' ? next : null;
  }

  /** Resolves once the editor for the current value, placeholder and catalog is mounted. */
  get ready(): Promise<void> {
    return this.#ready;
  }

  /** Puts the caret in the editor. */
  override focus(options?: FocusOptions): void {
    const view = fieldEditorView(this);
    if (view) view.focus();
    else super.focus(options);
  }

  // ---------------------------------------------------------------------------
  // Lifecycle
  // ---------------------------------------------------------------------------

  connectedCallback(): void {
    // A framework may have set properties before this class was defined: they
    // sit on the instance and hide the accessors. Re-set them through them.
    for (const key of UPGRADED) {
      if (Object.prototype.hasOwnProperty.call(this, key)) {
        const own = (this as unknown as Record<string, unknown>)[key];
        delete (this as unknown as Record<string, unknown>)[key];
        (this as unknown as Record<string, unknown>)[key] = own;
      }
    }
    injectStyles();
    if (!this.#wrapper) {
      const wrapper = document.createElement('div');
      wrapper.className = 'mbr-markdown-field';
      wrapper.setAttribute(POPOVER_HOST_ATTR, '');
      // Native `change` events from Crepe's own inputs (the image block's URL
      // field, its file picker) must not reach the host as if they were ours.
      wrapper.addEventListener('change', (event) => event.stopPropagation());
      this.#wrapper = wrapper;
    }
    if (this.#wrapper.parentNode !== this) this.replaceChildren(this.#wrapper);
    this.addEventListener('focusout', this.#onFocusOut);
    this.#scheduleMount();
  }

  disconnectedCallback(): void {
    this.removeEventListener('focusout', this.#onFocusOut);
    this.#flushChange();
    // Keep what was typed: a field moved in the DOM re-opens on it.
    void this.value;
    this.#teardown();
  }

  attributeChangedCallback(name: (typeof OBSERVED)[number], _old: string | null, next: string | null): void {
    switch (name) {
      case 'value':
        this.value = next ?? '';
        break;
      case 'readonly':
        this.#applyReadonly(next !== null);
        break;
      case 'placeholder':
        this.placeholder = next ?? '';
        break;
      case 'catalog-url':
        this.catalogUrl = next ?? '';
        break;
      case 'link-base':
        this.linkBase = next ?? '/';
        break;
    }
  }

  // ---------------------------------------------------------------------------
  // The editor
  // ---------------------------------------------------------------------------

  #applyReadonly(on: boolean): void {
    this.#readonly = on;
    this.setAttribute('aria-readonly', String(on));
    this.#crepe?.setReadonly(on);
  }

  /** Coalesces the property sets of one task into one mount. */
  #scheduleMount(): void {
    if (!this.isConnected || this.#mountQueued) return;
    this.#mountQueued = true;
    const ready = new Promise<void>((resolve, reject) => {
      queueMicrotask(() => {
        this.#mountQueued = false;
        this.#mount().then(resolve, reject);
      });
    });
    // A failed mount rejects `ready` for whoever awaits it, and is logged, not unhandled.
    ready.catch((err: unknown) => console.error('mbr-markdown-field: the editor failed to start', err));
    this.#ready = ready;
  }

  /** Re-opens the editor on the current value (placeholder or catalog changed). */
  #remount(): void {
    void this.value; // fold unsaved typing into #value first
    this.#flushChange();
    this.#scheduleMount();
  }

  async #mount(): Promise<void> {
    this.#teardown();
    const wrapper = this.#wrapper;
    if (!this.isConnected || !wrapper) return;
    const generation = this.#generation;
    const root = document.createElement('div');
    wrapper.replaceChildren(root);

    const opened = { body: toLf(this.#value), ending: lineEndingOf(this.#value) };
    const { body } = opened;
    const plugins: MilkdownPlugin[] = [docChangePlugin(() => this.#onDocChanged())];
    const catalogUrl = this.#catalogUrl;
    if (catalogUrl) {
      const catalog = memoizeLatest(() => fetchCatalog(catalogUrl));
      plugins.push(
        createLinkAutocompletePlugin({
          // Fresh per `[[` session, so a note created since shows.
          fetchSiteFiles: () => catalog.fresh(),
          currentUrl: () => this.#linkBase,
        }),
      );
    }
    const crepe = createMarkdownCrepe({
      root,
      body,
      featureConfigs: {
        [CrepeFeature.Placeholder]: { text: this.#placeholder, mode: 'doc' },
        [CrepeFeature.ImageBlock]: {
          onUpload: (file: File) =>
            this.#uploadImage
              ? this.#uploadImage(file)
              : Promise.reject(new Error('mbr-markdown-field: image upload needs an uploadImage function')),
        },
        [CrepeFeature.BlockEdit]: { buildMenu: addDefinitionListMenuItem },
      },
      plugins,
    });
    await crepe.create();
    if (generation !== this.#generation) {
      // Torn down or re-mounted while Crepe was starting.
      void crepe.destroy();
      return;
    }
    this.#crepe = crepe;
    this.#opened = opened;
    this.#saver = createBodySaver(crepe, body);
    crepe.setReadonly(this.#readonly);
  }

  #teardown(): void {
    this.#generation += 1;
    const crepe = this.#crepe;
    this.#crepe = null;
    this.#saver = null;
    if (crepe) {
      try {
        void crepe.destroy();
      } catch {
        /* already gone */
      }
    }
    this.#wrapper?.replaceChildren();
  }

  /** The markdown a save would write: unedited blocks as set, the value's own end and line ending. */
  #serialize(saver: BodySaver): string {
    const { body: original, ending } = this.#opened;
    return withLineEnding(withAuthoredEnd(saver.body().body, original), ending);
  }

  // ---------------------------------------------------------------------------
  // Events
  // ---------------------------------------------------------------------------

  #onDocChanged(): void {
    this.#dirty = true;
    if (this.#changeTimer !== null) clearTimeout(this.#changeTimer);
    this.#changeTimer = setTimeout(() => this.#flushChange(), CHANGE_DEBOUNCE_MS);
  }

  #cancelChange(): void {
    if (this.#changeTimer !== null) clearTimeout(this.#changeTimer);
    this.#changeTimer = null;
  }

  /** Sends the pending `change`, if the value moved since the last one. */
  #flushChange(): void {
    this.#cancelChange();
    const value = this.value;
    if (value === this.#reported) return;
    this.#reported = value;
    this.dispatchEvent(
      new CustomEvent<MarkdownFieldChangeDetail>('change', { detail: { value }, bubbles: true, composed: true }),
    );
  }

  #onFocusOut = (event: FocusEvent): void => {
    const next = event.relatedTarget;
    if (next instanceof Node && this.contains(next)) return;
    this.#flushChange();
    this.dispatchEvent(new Event('blur'));
  };
}

declare global {
  interface HTMLElementTagNameMap {
    'mbr-markdown-field': MbrMarkdownField;
  }
}

/** Defines `<mbr-markdown-field>` once (a second bundle on the page is harmless). */
export function defineMarkdownField(): void {
  if (!customElements.get(MARKDOWN_FIELD_TAG)) customElements.define(MARKDOWN_FIELD_TAG, MbrMarkdownField);
}
