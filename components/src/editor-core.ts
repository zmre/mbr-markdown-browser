/**
 * The editor core shared by `<mbr-editor>` (`editor-crepe.ts`, the lazy
 * `mbr-editor.min.js` chunk) and `<mbr-markdown-field>` (`markdown-field/`,
 * the standalone `mbr-markdown-field.min.js` bundle): Crepe configured for
 * mbr's markdown, the serializer and the save safety net, and the stylesheet
 * that themes Crepe.
 *
 * **Imports nothing stateful and touches no page globals** — no
 * `window.frontmatter`, no `shared.ts`, no edit token, no fetch. Both bundles
 * inline it, and `<mbr-markdown-field>` runs on pages mbr did not render
 * (`markdown-field.test.ts` walks this module's imports to keep it that way).
 * Anything that reads the page or the server belongs to the element using it.
 */

import { Crepe, CrepeFeature, type CrepeConfig } from '@milkdown/crepe';
import crepeProsemirrorCss from '@milkdown/crepe/theme/common/prosemirror.css?inline';
import crepeResetCss from '@milkdown/crepe/theme/common/reset.css?inline';
import crepeBlockEditCss from '@milkdown/crepe/theme/common/block-edit.css?inline';
import crepeCursorCss from '@milkdown/crepe/theme/common/cursor.css?inline';
import crepeImageBlockCss from '@milkdown/crepe/theme/common/image-block.css?inline';
import crepeLinkTooltipCss from '@milkdown/crepe/theme/common/link-tooltip.css?inline';
import crepeListItemCss from '@milkdown/crepe/theme/common/list-item.css?inline';
import crepePlaceholderCss from '@milkdown/crepe/theme/common/placeholder.css?inline';
import crepeToolbarCss from '@milkdown/crepe/theme/common/toolbar.css?inline';
import crepeTableCss from '@milkdown/crepe/theme/common/table.css?inline';
import { remarkCtx } from '@milkdown/kit/core';
import type { MilkdownPlugin } from '@milkdown/kit/ctx';
import { remarkPreserveEmptyLinePlugin } from '@milkdown/kit/preset/commonmark';
import { withAuthoredBlankLines } from './editor-frontmatter.js';
import {
  matchSequences,
  minimalDiff,
  prepareBaseline,
  rewrittenLines,
  type LineRange,
  type ParsedRoot,
} from './editor-minimal-diff.js';
import { alerts } from './editor-plugins/alerts.js';
import { authoredStylePlugin } from './editor-plugins/authored-style.js';
import { autolinkLiteralsPlugin } from './editor-plugins/autolink-literals.js';
import { linkEdgesPlugin } from './editor-plugins/link-spaces.js';
import { nestedMarksPlugin } from './editor-plugins/nested-marks.js';
import { nestedListStartPlugin } from './editor-plugins/nested-list-start.js';
import { indentedCodePlugin } from './editor-plugins/indented-code.js';
import { canonicalOf, minimalEscapesPlugin, type Parser } from './editor-plugins/minimal-escapes.js';
import { taskMarkers } from './editor-plugins/task-markers.js';
import { headingAttrsPlugin } from './editor-plugins/heading-attrs.js';
import { sectionAttrsPlugin } from './editor-plugins/section-attrs.js';
import { wikilinksPlugin } from './editor-plugins/wikilinks.js';
import { mathPlugin } from './editor-plugins/math.js';
import { escapesPlugin } from './editor-plugins/escapes.js';
import { referencesPlugin } from './editor-plugins/references.js';
import { imagesPlugin } from './editor-plugins/images.js';
import { definitionListPlugins } from './editor-plugins/definition-list.js';

/**
 * Crepe's structural stylesheet for the features the editor runs with: its
 * `theme/common/style.css` minus the sheets of features {@link CREPE_FEATURES}
 * switches off or Crepe leaves off (CodeMirror, LaTeX, top bar, AI and its
 * diff view). It consumes the `--crepe-*` variables {@link EDITOR_THEME_CSS}
 * maps onto the page's theme.
 *
 * Listed file by file because `style.css` imports `latex.css`, which imports
 * KaTeX's stylesheet, and `?inline` inlines every KaTeX font as base64:
 * ~1.4 MB of each bundle for rules nothing matches (mbr's math nodes are
 * `data-type="mbr-math"`, drawn by `editor-plugins/math.ts`, not KaTeX).
 */
export const CREPE_BASE_CSS = [
  crepeProsemirrorCss,
  crepeResetCss,
  crepeBlockEditCss,
  crepeCursorCss,
  crepeImageBlockCss,
  crepeLinkTooltipCss,
  crepeListItemCss,
  crepePlaceholderCss,
  crepeToolbarCss,
  crepeTableCss,
].join('\n');

/**
 * The Crepe feature switches the editor runs with.
 *
 * CodeMirror (and its dependent LaTeX feature) are disabled. CodeMirror
 * statically pulls in @codemirror/language-data (≈50 lazy language imports) —
 * bloating the bundle and, when inlined into a single chunk, breaking module
 * init order. The LaTeX feature depends on CodeMirror, so it must be disabled
 * too. Neither is essential here: code blocks still save as fenced code and
 * math still saves as `$…$` (rendered on the page by the existing katex
 * component); only the in-editor helpers are dropped.
 *
 * Shared by `openEditor`, `<mbr-markdown-field>` and the round-trip and parity
 * tests, so all of them parse and serialize with exactly the configuration a
 * real save uses.
 */
export const CREPE_FEATURES = {
  [CrepeFeature.CodeMirror]: false,
  [CrepeFeature.Latex]: false,
} as const;

/**
 * Feature settings every editor needs, tests included: Crepe's virtual
 * cursor is off because `references.ts` installs the same plugin configured
 * for its `mbr_link_ref` mark.
 */
export const CREPE_FEATURE_CONFIGS = {
  [CrepeFeature.Cursor]: { virtual: false },
} as const;

/**
 * The Milkdown plugins that teach the editor mbr's markdown extensions
 * (definition lists, `[-]`/`[>]` tasks, `--- {attrs}`, wikilinks, …), one per
 * `editor-plugins/<name>.ts`.
 *
 * **This list is the registration point.** `openEditor`, `<mbr-markdown-field>`
 * and the parity harness (`editor-parity.test.ts`) all install exactly these, so a plugin is
 * held to the byte-identical round trip and to the renderer's block outline
 * the moment it is listed here. Plugins that are editing aids rather than
 * syntax (link autocomplete, media double-click) are passed to
 * {@link createMarkdownCrepe} by the element that wants them.
 */
export const MARKDOWN_EXTENSION_PLUGINS: readonly (MilkdownPlugin | MilkdownPlugin[])[] = [
  sectionAttrsPlugin,
  headingAttrsPlugin,
  wikilinksPlugin,
  taskMarkers,
  nestedListStartPlugin,
  alerts,
  mathPlugin,
  escapesPlugin,
  referencesPlugin,
  imagesPlugin,
  definitionListPlugins,
  autolinkLiteralsPlugin,
  linkEdgesPlugin,
  nestedMarksPlugin,
  // Before authored-style, which records the merged block's spelling.
  indentedCodePlugin,
  authoredStylePlugin,
  minimalEscapesPlugin,
];

/**
 * Installs {@link MARKDOWN_EXTENSION_PLUGINS} on `crepe`, before `create()`,
 * and takes out Milkdown's `remarkPreserveEmptyLine`.
 *
 * That plugin is half of a pair: Milkdown prints an empty paragraph as a
 * `<br />` placeholder, and the plugin folds a lone `<br />` back into one on
 * load. The editor must never write HTML the author did not type, so an empty
 * paragraph prints as nothing (`authored-style.ts`, `paragraphContent`) — and
 * then a `<br />` on load can only be one the author wrote, which must stay
 * HTML: folded, it was rewritten as `<br />`, or dropped outright in a table
 * cell. Without the plugin installed the paragraph's placeholder is off too
 * (Milkdown checks for the plugin before writing one).
 */
export function useMarkdownExtensions(crepe: Crepe): void {
  void crepe.editor.remove(remarkPreserveEmptyLinePlugin);
  for (const plugin of MARKDOWN_EXTENSION_PLUGINS) crepe.editor.use(plugin);
}

/**
 * The body a save writes: Crepe's serialization plus mbr's post-processing.
 * The one definition, shared by the editor's dirty check / save and the
 * round-trip tests, so the tests cannot drift from what a save does.
 */
export function serializeBody(crepe: Crepe): string {
  return crepe.getMarkdown();
}

/** What a save writes, given the body Crepe was opened on (see `editor-minimal-diff.ts`). */
export interface BodySaver {
  /**
   * The body to write: the current serialization with every unedited block
   * as it was loaded. `whole`: that was not possible, and the body is the
   * whole serialization (which rewrites {@link BodySaver.rewritten}'s lines).
   */
  body(): { body: string; whole: boolean };
  /** Lines of the loaded body the editor prints differently (see `rewrittenLines`). */
  rewritten: { ifEdited: LineRange[]; onSave: LineRange[] };
}

/**
 * The save path for a body opened in `crepe`: call right after `create()`,
 * before any edit, with the body `crepe` was given.
 */
export function createBodySaver(crepe: Crepe, original: string): BodySaver {
  // Whitespace-only lines the printer empties are put back where their neighbours are unchanged.
  const printed = () => withAuthoredBlankLines(serializeBody(crepe), original);
  const baseline = printed();
  const whole = () => ({ body: printed(), whole: false });
  // The round trip is exact: every unedited block already prints as loaded.
  if (baseline === original) return { body: whole, rewritten: { ifEdited: [], onSave: [] } };
  const remark = crepe.editor.action((ctx) => ctx.get(remarkCtx)) as unknown as Parser;
  const parse = (markdown: string) => remark.parse(markdown) as ParsedRoot;
  const base = prepareBaseline(original, baseline, parse, canonicalOf(remark));
  if (!base) {
    const lines = changedLines(original, baseline);
    return { body: () => ({ body: printed(), whole: true }), rewritten: { ifEdited: [], onSave: lines } };
  }
  return {
    body: () => {
      const current = printed();
      const body = minimalDiff(base, current, parse);
      return body === null ? { body: current, whole: true } : { body, whole: false };
    },
    rewritten: rewrittenLines(base),
  };
}

/** Line ranges of `original` that are not in `changed` (a line diff). */
export function changedLines(original: string, changed: string): LineRange[] {
  const a = original.split('\n');
  const kept = new Set(matchSequences(a, changed.split('\n')).map(([i]) => i));
  const ranges: LineRange[] = [];
  a.forEach((_, i) => {
    if (kept.has(i)) return;
    const last = ranges[ranges.length - 1];
    if (last && last.last === i) last.last = i + 1;
    else ranges.push({ first: i + 1, last: i + 1 });
  });
  return ranges;
}


/** What {@link createMarkdownCrepe} builds an editor from. */
export interface MarkdownCrepeOptions {
  /** The element Crepe mounts into. */
  root: HTMLElement;
  /** The markdown body to open (LF line endings; no frontmatter). */
  body: string;
  /**
   * Feature settings merged over {@link CREPE_FEATURE_CONFIGS} (uploads,
   * placeholder, the block menu…). The shared ones cannot be switched off here.
   */
  featureConfigs?: CrepeConfig['featureConfigs'];
  /**
   * Editing aids installed after mbr's syntax plugins, so they see its nodes
   * (link autocomplete, media double-click, change tracking).
   */
  plugins?: readonly MilkdownPlugin[];
}

/**
 * Crepe on `body`, configured for mbr's markdown exactly as every editor and
 * test uses it: {@link CREPE_FEATURES}, {@link CREPE_FEATURE_CONFIGS} (with
 * `featureConfigs` merged over), {@link useMarkdownExtensions}, then `plugins`.
 * Not yet created: the caller awaits `crepe.create()`.
 */
export function createMarkdownCrepe(opts: MarkdownCrepeOptions): Crepe {
  const crepe = new Crepe({
    root: opts.root,
    defaultValue: opts.body,
    features: CREPE_FEATURES,
    featureConfigs: { ...opts.featureConfigs, ...CREPE_FEATURE_CONFIGS },
  });
  // mbr's markdown syntax first, so the editing aids see its nodes. Plugins go
  // on the underlying Milkdown editor before create() — Crepe's injection point.
  useMarkdownExtensions(crepe);
  for (const plugin of opts.plugins ?? []) crepe.editor.use(plugin);
  return crepe;
}

/** An async lookup memoized: see {@link memoizeLatest}. */
export interface LatestMemo<T> {
  /** The last result fetched (fetching the first time). */
  (): Promise<T>;
  /** A fresh fetch, unless one is already in flight, which is shared; later plain calls get its answer. */
  fresh(): Promise<T>;
}

/**
 * `fetcher` memoized: plain calls share the last request, `fresh()` starts a
 * new one (concurrent callers share a request in flight). A failed request is
 * forgotten, so the next call retries.
 */
export function memoizeLatest<T>(fetcher: () => Promise<T>): LatestMemo<T> {
  let pending: Promise<T> | null = null;
  let inFlight = false;
  const start = (): Promise<T> => {
    inFlight = true;
    const request: Promise<T> = fetcher().then(
      (value) => {
        if (pending === request) inFlight = false;
        return value;
      },
      (err: unknown) => {
        if (pending === request) {
          pending = null;
          inFlight = false;
        }
        throw err;
      },
    );
    pending = request;
    return request;
  };
  const memo = (): Promise<T> => pending ?? start();
  memo.fresh = (): Promise<T> => (pending && inFlight ? pending : start());
  return memo;
}

// Maps Crepe's theme variables onto Pico's, with the Nord light values as
// fallbacks in case a Pico variable is absent.
export const EDITOR_THEME_CSS = `
.milkdown {
  --crepe-color-background: var(--pico-background-color, #fdfcff);
  --crepe-color-on-background: var(--pico-color, #1b1c1d);
  --crepe-color-surface: var(--pico-card-background-color, #f8f9ff);
  --crepe-color-surface-low: var(--pico-card-sectioning-background-color, #f2f3fa);
  --crepe-color-on-surface: var(--pico-color, #191c20);
  --crepe-color-on-surface-variant: var(--pico-muted-color, #43474e);
  /* Crepe uses --crepe-color-outline for toolbar/menu icon fill as well as for
     borders (the border uses are mostly color-mixed to ~20% opacity). Map it to
     Pico's muted *text* color, not muted-border-color, so the icons stay legible
     in both light and dark — matching Nord's mid-gray outline. */
  --crepe-color-outline: var(--pico-muted-color, #73777f);
  --crepe-color-primary: var(--pico-primary, #37618e);
  --crepe-color-secondary: var(--pico-secondary-background, #d7e3f8);
  --crepe-color-on-secondary: var(--pico-secondary-inverse, #101c2b);
  --crepe-color-inverse: var(--pico-contrast-background, #2e3135);
  --crepe-color-on-inverse: var(--pico-contrast-inverse, #eff0f7);
  --crepe-color-inline-code: var(--pico-code-color, #ba1a1a);
  --crepe-color-error: var(--pico-del-color, #ba1a1a);
  --crepe-color-hover: var(--pico-secondary-hover-background, #eceef4);
  --crepe-color-selected: var(--pico-muted-border-color, #e1e2e8);
  --crepe-color-inline-area: var(--pico-code-background-color, #d8dae0);

  --crepe-font-title: var(--pico-font-family, Rubik, Cambria, 'Times New Roman', Times, serif);
  --crepe-font-default: var(--pico-font-family, Inter, Arial, Helvetica, sans-serif);
  --crepe-font-code: var(--pico-font-family-monospace, 'JetBrains Mono', Menlo, Monaco, 'Courier New', Courier, monospace);

  /* Elevation for floating menus (toolbar, block-edit, link tooltip). Crepe's
     Nord theme defined these; without them box-shadow resolves to nothing, and
     in light mode the menu surface is the same white as the page — leaving the
     icons with no container/edge. These match Crepe's original Nord elevation. */
  --crepe-shadow-1: 0 1px 3px 1px rgba(0, 0, 0, 0.15), 0 1px 2px 0 rgba(0, 0, 0, 0.3);
  --crepe-shadow-2: 0 2px 6px 2px rgba(0, 0, 0, 0.15), 0 1px 2px 0 rgba(0, 0, 0, 0.3);
}

/* Code block / inline code background: match the rendered page. Pico paints
   <pre>/<code> with --pico-code-background-color at full opacity, but Crepe's
   default is a translucent color-mix (≈60%), which looks washed out and lighter
   than the page. Reset's higher-specificity \`pre code { background: transparent }\`
   still keeps the inner <code> clear, so blocks don't double-layer. */
.milkdown .ProseMirror pre,
.milkdown .ProseMirror code {
  background: var(--pico-code-background-color);
}
`;

// Footnote nodes (from the gfm preset) are unstyled in the editor by default.
// Style the inline reference as a raised citation badge and the definition as a
// clearly-delimited note, reusing the same Crepe theme variables as EDITOR_THEME_CSS.
export const FOOTNOTE_CSS = `
.milkdown .ProseMirror sup[data-type="footnote_reference"] {
  font-size: 0.75em;
  line-height: 0;
  vertical-align: super;
  font-weight: 600;
  color: var(--crepe-color-primary);
  padding: 0 0.15em;
  cursor: default;
  user-select: none;
}
.milkdown .ProseMirror dl[data-type="footnote_definition"] {
  margin: 0.4rem 0;
  padding: 0.15rem 0 0.15rem 0.75rem;
  border-left: 2px solid var(--crepe-color-selected, var(--pico-muted-border-color, #e1e2e8));
  font-size: 0.9em;
  color: var(--crepe-color-on-surface-variant, var(--pico-muted-color, #43474e));
}
.milkdown .ProseMirror dl[data-type="footnote_definition"] dt {
  display: inline;
  margin-right: 0.35rem;
  font-weight: 700;
  color: var(--crepe-color-primary);
}
.milkdown .ProseMirror dl[data-type="footnote_definition"] dt::before { content: "["; }
.milkdown .ProseMirror dl[data-type="footnote_definition"] dt::after { content: "]"; }
.milkdown .ProseMirror dl[data-type="footnote_definition"] dd {
  display: inline;
  margin: 0;
}
`;
