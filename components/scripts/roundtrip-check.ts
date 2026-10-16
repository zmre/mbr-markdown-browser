#!/usr/bin/env bun
/**
 * Checks your own notes against the editor's round trip, without sharing them.
 *
 *     cd components
 *     bun run roundtrip-check ~/path/to/notes --out /tmp/mbr-roundtrip
 *
 * Every `.md`/`.markdown` note under the folder (hidden folders such as
 * `.git`, `.obsidian` and `.mbr` skipped; symlinked folders not entered) is
 * opened and saved untouched exactly as the in-browser editor does it — the
 * same frontmatter split and CRLF handling, Crepe with `CREPE_FEATURES`,
 * `CREPE_FEATURE_CONFIGS` and `MARKDOWN_EXTENSION_PLUGINS`, and the open-time
 * check behind the editor's "lines that use markdown the editor writes
 * differently" notice. The code is the editor's own (`editor-crepe.ts`,
 * `editor-frontmatter.ts`, `editor-test-support.ts`), run in vitest under
 * happy-dom, so the tool cannot disagree with the editor.
 *
 * The terminal shows **counts only** (notes checked, unchanged, would change,
 * errors, time) and the report's path — never a note's name or content. The
 * report folder holds `index.md` (id → note, changed lines, a guess at the
 * construct, whether the anonymized reproduction still reproduces) and, per
 * note that would change, `NNNN/`:
 *
 * - `diff.patch` — the note → what a full re-serialize writes, preceded by
 *   the lines the editor's notice lists;
 * - `repro.md` — a minimal reproduction: the changed blocks (plus the
 *   frontmatter and definitions they need), then shrunk block by block and
 *   line by line while the editor still rewrites it the same way;
 * - `repro.anon.md` — the reproduction with letters → `x`/`X` and digits →
 *   `0`, markdown syntax and mbr's keywords kept, re-checked: the index says
 *   `reproduces` or `differs (review repro.md manually)`;
 * - `repro.diff.patch`, `repro.anon.diff.patch` — what the editor does to each.
 *
 * Turning one into a test fixture: read `repro.anon.md` (anonymization is
 * automatic — redact anything it left that you would not publish), copy it to
 * `tests/fixtures/editor-roundtrip/<name>.md`, run
 * `MBR_UPDATE_OUTLINES=1 cargo test -p mbr-core editor_parity` from the
 * repository root for its renderer outline, and review the `.outline.json`.
 * Until the editor handles it, list it in `editor-parity.test.ts`'s
 * `ROUND_TRIP_FAILS`. See docs/modes/editing.md, "Checking your own notes".
 *
 * Exit status: 0 when the check ran (whatever it found), 2 on bad arguments
 * or a failed run (details in `<report>/run.log`).
 */
import { main } from '../src/roundtrip-check/cli.ts';

process.exitCode = await main(process.argv.slice(2), {
  stdout: (text) => process.stdout.write(text),
  stderr: (text) => process.stderr.write(text),
  isTTY: process.stderr.isTTY,
});
