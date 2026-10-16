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
 * Structural / lossy: none known.
 *
 * Fixed by a plugin in `editor-plugins/` (kept here so a regression is
 * recognisable): definition lists were one paragraph, an indented nested list
 * under `: def` was hoisted and the next `: def` folded into its last item
 * (T1-A09/T1-B11 damage), and a fenced code block in a `: def` swallowed the
 * rest of the document (`definition-list.ts`; `deflist-edge` covers the
 * renderer's start/continue/end rules); `[-]`/`[>]` tasks were escaped
 * (`task-markers.ts`); `> [!NOTE]` alerts became `> \[!NOTE]` (`alerts.ts`);
 * `--- {attrs}` became `\--- {attrs}` in both positions (`section-attrs.ts`);
 * heading attributes had no structure (`heading-attrs.ts`); wikilinks were
 * escaped (`wikilinks.ts`); backslash escapes inside `$…$`/`$$…$$` were
 * consumed (`math.ts`); `&amp;`/`&nbsp;` were decoded and `\$` lost its
 * backslash (`escapes.ts` — still decoded, same meaning, in link destinations
 * and alt text); `[text][ref]` was inlined and its definition deleted
 * (`references.ts`); untitled images were deleted and a standalone image's
 * alt became Crepe's ratio `![1.00](src)` (`images.ts`). CRLF files were
 * rewritten as LF (`openEditor`/`savedFile` now keep the file's ending).
 *
 * Cosmetic (same structure — item 6), still normalized:
 * - Definition lists: `:` followed by no space or several → `: `; a lazy
 *   continuation line is indented; `:` alone then content on the next line
 *   joins them (`deflist-edge`).
 * - A paragraph directly under a heading gets a blank line between.
 * - An unneeded escape the author wrote in a paragraph or heading is kept
 *   (its source rides along); one in a table cell or term may be dropped.
 *   `x\\y` → `x\y` (the same text: `\y` is no escape).
 *
 * Fixed by `authored-style.ts` / `minimal-escapes.ts` (item 6): bullets
 * (`-`/`+` were `*`), `1)` delimiters (and the swap between adjacent ordered
 * lists), `1. 1. 1.` numbering, `-   x` gaps, `---`/`___`/`* * *` rules (were
 * `***`), setext headings and ATX closing `#`s, indented code, `~~~` and long
 * fences, a fence's info-string meta (was dropped), two-space hard breaks
 * (were `\`), `>>>` marginalia (were `> > >`), table spacing (was re-padded),
 * and escapes remark adds where nothing needs one (`2 \* 3`, `see \[a]`,
 * `AT\&T`, `\[\[x]]`, `* \[-]`).
 *
 * Survive untouched: `==mark==`, `~~strike~~`, footnotes, `{{ vid(...) }}`,
 * raw HTML blocks and inline HTML, ```` ```chat ```` fences,
 * `TODO:`/`TK`/`FIXME`/`XXX` markers, task annotations (`@due(...)`,
 * `@done(...)`, `#tag`, `!!`, `> date`), `__strong__`/`_em_`, and definition
 * lists in house style (`: ` + two-space continuations), the damaged T1-B11
 * included.
 *
 * Structure only (bytes survive, outline differs): a standalone image
 * (Crepe's image-block has no source position).
 */
import { afterEach, describe, expect, it } from 'vitest';
import { EDITOR_FIXTURES, destroyEditors, editorOutline, savedFile } from './editor-test-support.js';

/** Fixtures whose untouched save is not byte-identical today, and why. */
const ROUND_TRIP_FAILS: Record<string, string> = {
  'deflist-edge': 'item 6: `:` spacing and lazy continuation lines normalized, a blank line added after a heading',
};

/** Fixtures whose editor outline disagrees with the renderer's today, and why. */
const OUTLINE_FAILS: Record<string, string> = {
  images: "Crepe's image-block (a lone image) carries no source position, so the outline cannot place it",
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
