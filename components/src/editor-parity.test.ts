/**
 * Editor parity harness (TODO: "Editor: cover mbr's markdown extensions with
 * Milkdown plugins", item 0). Two checks per fixture in
 * `tests/fixtures/editor-roundtrip/*.md`, both against the real Crepe with the
 * editor's own configuration (`CREPE_FEATURES` + `MARKDOWN_EXTENSION_PLUGINS`,
 * see `editor-test-support.ts`):
 *
 * (a) **Round trip.** Opening the file and saving it untouched writes it back
 *     byte for byte (frontmatter through the YAML textarea, body through
 *     Crepe, `recombine`d — `openEditor`'s own save path).
 * (b) **Structure.** The editor's block outline of the file — built from the
 *     mdast Milkdown parses into (`editor-outline.ts`) — equals the renderer's,
 *     `<fixture>.outline.json`, which mbr-core's `editor_parity` test writes
 *     from pulldown-cmark with the renderer's options. The format is
 *     documented in both `editor-outline.ts` and
 *     `crates/mbr-core/src/editor_parity.rs`: a pre-order array of
 *     `{depth, kind, lines: [first, last], …kind fields}`, lines 1-based,
 *     inclusive, whole-file. Goldens are only ever generated on the Rust side
 *     (`MBR_UPDATE_OUTLINES=1 cargo test -p mbr-core editor_parity`).
 *
 * Fixtures that fail today are listed in `ROUND_TRIP_FAILS` / `OUTLINE_FAILS`
 * with the TODO item that will fix them, and run as `it.fails`. When a plugin
 * fixes one, vitest reports that `it.fails` as a failure — delete its entry
 * (the test becomes a plain `it`). A new fixture needs no registration here:
 * drop `<name>.md` in the directory, regenerate the goldens on the Rust side,
 * and both checks pick it up.
 *
 * # Inventory: what an untouched save changes today (2026-10-15, Crepe 7.22.2)
 *
 * Structural / lossy:
 * - Definition lists (item 1): parsed as one paragraph; an indented nested
 *   list under `: def` becomes a top-level list and the next `: def` is folded
 *   into its last item (T1-A09/T1-B11 damage). **A fenced code block in a
 *   `: def` (`: ```rust`) is worse: the opening fence is escaped
 *   (`\`\`\`rust`), its indented body loses its indent, and the closing fence
 *   then opens a new code block that swallows the rest of the document**
 *   (`flashcards-loose`).
 * - `[-]` / `[>]` tasks (item 2) → `\[-]` / `\[>]`, no longer tasks.
 * - `--- {attrs}` (item 3) → `\--- {attrs}`, no longer a section rule. The
 *   setext-position case (`--- {…}` directly under a paragraph) is a
 *   paragraph continuation to the renderer too, but is escaped the same way.
 * - Math (new, item 5): `$…$`/`$$…$$` are plain text to the editor (Crepe's
 *   LaTeX feature is off), so CommonMark backslash escapes inside them are
 *   consumed: `\,` → `,`, `\;` → `;`, `\{a\}` → `{a}`. Silent LaTeX damage.
 * - GitHub alerts (new, item 5): `> [!NOTE]` → `> \[!NOTE]`, which the
 *   renderer no longer treats as an alert.
 * - Entities (new, item 5): `&amp;` → `&`, `&copy;` → `©`, `&nbsp;` → a
 *   literal U+00A0 (an invisible change).
 * - Reference links (new, item 5/6): `[text][ref]` is inlined to
 *   `[text](url "title")` and the `[ref]: …` definition is deleted.
 * - Line endings (new, item 6): CRLF files are rewritten with LF throughout,
 *   frontmatter included.
 *
 * Cosmetic (same structure, whole-file diff — item 6):
 * - `-` and `+` bullets → `*`; `[X]` → `[x]`.
 * - `1)` → `1.`; and remark alternates delimiters between *adjacent* ordered
 *   lists, so `1)…` then `5.…` saves as `1.…` then `5)…` (swapped).
 * - `---` thematic break → `***`.
 * - Tables re-padded and delimiter rows normalized (`|-|-|` → `| - | - |`).
 * - `>>>` marginalia / `>>` pull quotes → `> > >` / `> >` (same nesting).
 * - Setext headings → ATX; indented code → fenced; `~~~` fences → backticks;
 *   two-space hard breaks → `\`; `2 * 3` → `2 \* 3`.
 *
 * Survive untouched: wikilinks in every form (after `unescapeWikilinks`),
 * `==mark==`, `~~strike~~`, footnotes, `{{ vid(...) }}`, raw HTML blocks and
 * inline HTML, ```` ```chat ```` fences, `TODO:`/`TK`/`FIXME`/`XXX` markers,
 * task annotations (`@due(...)`, `@done(...)`, `#tag`, `!!`, `> date`),
 * heading attributes `{#id .class}` (as text), `__strong__`/`_em_`, flat
 * definition lines (`: x` as paragraph text), and the already-damaged
 * T1-B11 (a fixed point).
 *
 * Structure only (bytes survive, outline differs): heading attributes (no
 * `id`/`classes` in the editor — item 5), flat definition lists and the
 * damaged deck (item 1).
 */
import { afterEach, describe, expect, it } from 'vitest';
import { EDITOR_FIXTURES, destroyEditors, editorOutline, savedFile } from './editor-test-support.js';

/** Fixtures whose untouched save is not byte-identical today, and why. */
const ROUND_TRIP_FAILS: Record<string, string> = {
  'commonmark-variants': 'item 6: setext/indented/tilde/hard-break/`+` normalized; item 5: reference definition dropped',
  crlf: 'item 6: CRLF → LF; `-` → `*`',
  escapes: 'item 5: entities decoded (`&nbsp;` → U+00A0); `*` escaped',
  'flashcards-a09': 'item 1: nested list under `: def` hoisted, next `: def` folded in',
  'flashcards-b11': 'item 1: nested list under `: def` hoisted, next `: def` folded in',
  'flashcards-dash-history': 'item 1: `-` history list hoisted out of its definition',
  'flashcards-loose': 'item 1: fence in a definition escaped; the rest of the file becomes code',
  'kitchen-sink': 'items 1, 2, 3, 6',
  'lists-dash': 'item 6: `-` → `*`',
  'lists-ordered-paren': 'item 6: `1)` → `1.` and adjacent-list delimiters swapped',
  marginalia: 'item 6: `>>>` → `> > >`',
  math: 'item 5: backslash escapes inside math consumed (`\\,` → `,`)',
  rules: 'item 6: `---` → `***`',
  'section-attrs': 'item 3: `--- {attrs}` → `\\--- {attrs}`',
  'section-attrs-setext': 'item 3: `--- {attrs}` under a paragraph → `\\--- {attrs}`',
  tables: 'item 6: tables re-padded',
  tasks: 'item 6: `-` → `*`',
};

/** Fixtures whose editor outline disagrees with the renderer's today, and why. */
const OUTLINE_FAILS: Record<string, string> = {
  crlf: 'item 1: no definition list',
  'flashcards-a09': 'item 1: no definition list',
  'flashcards-b11': 'item 1: no definition list',
  'flashcards-b11-damaged': 'item 1: no definition list',
  'flashcards-dash-history': 'item 1: no definition list',
  'flashcards-flat': 'item 1: no definition list',
  'flashcards-loose': 'item 1: no definition list',
  'heading-attrs': 'item 5: no heading attributes',
  'kitchen-sink': 'items 1, 2, 3, 5',
  'section-attrs': 'item 3: no section break',
};

const names = Object.keys(EDITOR_FIXTURES);

afterEach(destroyEditors);

describe('editor parity harness', () => {
  it('has fixtures, each with a renderer outline golden', () => {
    expect(names.length).toBeGreaterThan(20);
    const missing = names.filter((name) => EDITOR_FIXTURES[name].outline === null);
    expect(missing, 'regenerate with MBR_UPDATE_OUTLINES=1 cargo test -p mbr-core editor_parity').toEqual([]);
  });

  it('lists known failures only for fixtures that exist', () => {
    const stale = [...Object.keys(ROUND_TRIP_FAILS), ...Object.keys(OUTLINE_FAILS)].filter(
      (name) => !(name in EDITOR_FIXTURES),
    );
    expect(stale).toEqual([]);
  });
});

describe('editor round trip: an untouched save is byte-identical', () => {
  for (const name of names) {
    const known = ROUND_TRIP_FAILS[name];
    const test = known ? it.fails : it;
    test(known ? `${name} (fails today — ${known})` : name, async () => {
      const { source } = EDITOR_FIXTURES[name];
      expect(await savedFile(source)).toBe(source);
    });
  }
});

describe('editor structure: the editor outline equals the renderer outline', () => {
  for (const name of names) {
    const known = OUTLINE_FAILS[name];
    const test = known ? it.fails : it;
    test(known ? `${name} (fails today — ${known})` : name, async () => {
      const { source, outline } = EDITOR_FIXTURES[name];
      expect(outline, `no golden for ${name}`).not.toBeNull();
      expect(await editorOutline(source)).toEqual(JSON.parse(outline ?? '[]'));
    });
  }
});
