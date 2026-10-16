# TODO

## What's Next

* **Mobile**
  * [ ] Device check (TESTING.md): play FAB vs. review FAB stacking, slides exit ✕ and bar hiding in landscape, editor with the native bar hidden, Info panel ✕ below the bar.
  * [ ] Plain browsers have no visible "New note" control outside the editor (only the editor's ⋯ menu); consider a shortcut or a button on listings.
  * [ ] On a token-protected server a 401 from `/.mbr/raw` while the editor loads shows only text; the token field is never built. And a new note opened via `#mbr-edit` loses the in-memory token (by design of `edit-token.ts`), so the first save asks again.
  * [ ] Desktop with docked sidebar: pages are 10px too wide (Pico's `margin-right: -10px` on the last header `nav > ul` in an unpadded `header.container`).
* **Editor: cover mbr's markdown extensions with Milkdown plugins** (separate PR)
  * _Why:_ Crepe never saves the text it loaded — every save is `crepe.getMarkdown()`, i.e. the body parsed by remark (CommonMark + GFM) and re-serialized. Anything remark doesn't know is rewritten: definition lists lose their structure (`: Memory aids:` + `  * bullet` → blank line + column-0 bullets, the next `: ___Review History___` folded into the last bullet — this damaged real flashcard decks), `--- {attrs}` → `\--- {attrs}`, `[-]`/`[>]` → `\[-]`/`\[>]`, `-` bullets → `*`, `1)` → `1.`, tables re-padded. Pinned by `components/src/editor-roundtrip.test.ts` (`it.fails` cases flip to `it` as each lands). Stopgap already shipped: a save with no changes writes nothing. The flashcard writer is not involved (`flashcards.rs` `ham_deck_*`/`b11_*` tests).
  * _Mechanism (the supported path, not a hack):_ Crepe is a Milkdown preset and Milkdown's own commonmark/gfm support is built from the same pieces. Per construct: (1) a micromark syntax extension + mdast util for parse and to-markdown, registered with `$remark(...)`; (2) a ProseMirror node via `$nodeSchema(...)` with `parseMarkdown` (mdast → node) and `toMarkdown` (node → mdast); (3) a to-markdown handler that prints mbr's house style; plus keymaps / `$inputRule` / `$command` / `$view` where editing needs them. Attach with `crepe.editor.use(...)` before `create()`. Keep each plugin in its own `components/src/editor-plugins/<name>.ts` inside the editor chunk.
  * [x] **0. Parity harness first.** _Done: `components/src/editor-parity.test.ts` (+ `editor-outline.ts`, `editor-test-support.ts`) and mbr-core `editor_parity.rs`; register syntax plugins in `MARKDOWN_EXTENSION_PLUGINS` (`editor-crepe.ts`), remove a fixture's `ROUND_TRIP_FAILS`/`OUTLINE_FAILS` entry when it starts passing; the inventory is the comment at the top of the parity test._ A shared fixture corpus (`tests/fixtures/editor-roundtrip/*.md`: flashcard decks incl. T1-A09/T1-B11, tasks with every marker and annotation, section attrs, wikilinks in all forms, heading attrs, `==mark==`, footnotes, math, chat fences, `{{ vid(...) }}`, raw HTML, marginalia `>>>`, tables, `1)` lists, `-` bullets, CRLF). Two checks per fixture: (a) vitest: Crepe round trip of the untouched document is byte-identical (the target; failures listed as `it.fails` until fixed); (b) structural parity with pulldown-cmark: a small Rust test emits a block-structure outline (e.g. dl/dt/dd nesting + line ranges, list items + task status) and the TS side emits the same outline from the editor's mdast, compared in CI — so the editor and the renderer/`flashcards.rs` can never disagree about where a card or definition ends. Also use it to inventory anything not listed here that the round trip still changes.
  * [x] **1. Definition lists** (flashcards). _Done: `editor-plugins/definition-list(-syntax).ts`, an own micromark container with pulldown-cmark's rules (the package's dialect differed), `: ` + two-space print, tight/loose as authored; Enter/Backspace/`: `/slash menu; `deflist-edge` fixture. T1-A09/T1-B11 round-trip byte for byte._
  * [x] **2. Extended task markers.** _Done: `task-markers.ts` (forked tokenizer, `status`/`marker` on mdast, four-state checkbox view; Enter starts an open `[ ]` task; a task whose first block is a definition list keeps its box)._
  * [x] **3. Section attributes.** _Done: `section-attrs.ts` (atom node + attrs popover, both the paragraph and the setext position)._
  * [x] **4. Wikilinks, properly.** _Done: `wikilinks.ts` (micromark construct matching `wikilink.rs`, chip view, autocomplete inserts the node); `unescapeWikilinks` deleted._
  * [x] **5. Remaining extensions.** _Done: `alerts.ts`, `heading-attrs.ts`, `math.ts` (pulldown's math rules), `escapes.ts` (entities, `\$`), `references.ts` (references and definitions kept), `images.ts`, CRLF kept; alerts whose body opens with a definition list included._
  * [x] **6. Cosmetic serializer settings.** _Done: `authored-style.ts` carries each construct's spelling on the ProseMirror node (bullets, `)`, `1. 1.`, gaps, rules, setext/closing `#`, indented/`~~~`/long fences + info meta, two-space breaks, `>>>`, table rows) with `-`/`---` house style for new content; `minimal-escapes.ts` drops escapes the re-parse proves unneeded and keeps the author's. Every fixture round-trips byte for byte except `deflist-edge`._
  * [x] **7. Fallback/safety net.** _Done: minimal-diff save (`editor-minimal-diff.ts`): unedited top-level blocks are written as their original source, checked by re-splitting, else a confirm before a whole rewrite; a notice at open names lines the round trip changes. Property-tested over the corpus._
  * [x] Docs: `docs/modes/editing.md` (syntax in the editor, what a save writes), CLAUDE.md (editor plugin layout, parity harness rule, `$useKeymap` gotcha).
  * [ ] Remaining cosmetics, now only in a block the reader edits (`deflist-edge` in `ROUND_TRIP_FAILS`): `:` followed by no space or several spaces → `: `; a lazy continuation line of a definition/item/quote is indented (and a `:` there escaped); `:` alone then content on the next line is joined; a paragraph directly under a heading gets a blank line. Carry the gap after `:` per definition like `authored-style.ts` does for lists, if anyone writes these.
  * [ ] An authored unneeded escape in a table cell or a definition term is dropped when that block is edited (paragraphs and headings keep theirs via `mbrStyle.source`); `x\\y` saves as `x\y` (same text).
  * [ ] The outline check still cannot place a standalone image (`images` in `OUTLINE_FAILS`): Crepe's image-block carries no source position.
  * [ ] A list item whose first block is a definition list holds an empty filler paragraph in the editor (Milkdown's `list_item` must start with a paragraph); it is dropped on save but visible as an empty line above the term while editing.
  * [ ] Two definition lists separated only by a deleted block become one list (markdown cannot write two adjacent lists apart) — the save writes the merged list; expected, but the editor shows them as two until reload.

* **Review follow-ups (2026-10-07 review of #323–#329)**
  * [ ] Search modal: typing clears the selection (`mbr-search.ts` `_handleInput` sets `_selectedIndex = -1`) until results land, so a quick Enter is dropped and the highlight blinks; `_setResults` already resets to 0, so delete that line (optionally replay a pending Enter).
  * [ ] Search modal: `_lastPointer` is only recorded by `mousemove` on rows, so a pointer resting outside the rows can still hijack the selection when a longer list renders under it. Track pointer position on the whole `.modal`.
  * [ ] `type:` facet: `TYPE_TOKEN` in `search-extras/facets.ts` lets an unclosed quote swallow the rest of the query (`type:"Meeting rust` drops `rust`); use `"[^"]*"|\S*`. Server `facet_matches` is substring, so `type:person` also matches `salesperson` — match `type:` exactly or label the options.
  * [ ] Flashcard deck: mode/filter/resize keys still work while a rating write is pending, so the in-memory rating can land on a different card or session. Capture mode + index before the `await` in `_rate` and skip the session update if they changed (or ignore those keys while `_pending`).
  * [ ] Flashcard deck: its window capture-phase keydown swallows keys for the GUI find bar opened over it (Enter does nothing, Esc closes the deck, Tab is trapped). Let events whose composed path is in `mbr-find-bar` (or outside the deck) through.
  * [ ] Relationships: an inverse pair with different `category` values is published as-is with no warning (docs say they must match); reconcile first-declared-wins with a warning like hierarchy.
  * [ ] Contacts: map-form `aliases` leave stale `aliases.<label>` dot keys next to the flat list in `site.json` (duplicate info-panel rows); remove them in `normalize_simplified`. Also decide whether singular `email`/`phone`/`address`/`website` keys should be private.
  * [ ] Contacts: `Contact::from_yaml` reads only lowercase keys, so `Emails:` is (safely) not shown on the card; consider case-insensitive reading.
  * [ ] Family chart: a person's `image` reaches family-chart's `CardImage` unescaped (`<image href="${image}">` via d3 `.html()`); escape or URL-validate `avatar` in `genealogy/family-chart-view.ts`.
  * [ ] Org chart: after an in-place "+N more" expansion, Reset returns to the expanded view at the current zoom rather than a fresh fit; add a `showView()` to `graph/viewport-controller.ts`. "+N more" cards below the focus still navigate (would need a per-parent expanded set in `buildOrgTree`).
  * [ ] Chat blocks: `docs/markdown/chat.md` says move/rename skips links inside chat blocks — wrong, `link_rewrite` rewrites them (and inside every code fence; decide whether plain fences should be rewritten at all). Markers inside a chat bubble are highlighted but not listed by the task browser (`scan_source_tasks` skips fences); guard `mark_incomplete_blocks` or document it in CLAUDE.md. A `[^1]:` footnote definition inside a bubble yields a duplicate id.
  * [ ] Flashcard writer: files with lone `\r` line endings put every card on "line 1"; ordered history lists copy the last item's number. Add an integration test for 401 / symlink escape on `/.mbr/flashcard-review`.
  * [ ] `templates/_head.html` hard-codes the concentric threshold default `0.7` alongside `DEFAULT_FLASHCARDS_CONCENTRIC_THRESHOLD`; pass it from Rust.
  * [ ] `scripts/update-assets.sh` example version strings (hljs 11.11.2, mermaid 11.16.1) are stale.

* **Relationships & genealogy** (see [docs](docs/markdown/relationships.md))
  * [ ] Edit-mode support for structured person data: a friendlier way to view/edit the person frontmatter (born, died, born_place, gender, aliases, relationships) than hand-editing raw YAML in the in-browser editor — e.g. a small form for the known fields.
  * [ ] Wire the editor to the person `image` frontmatter field — pick/replace the portrait. Image upload itself is done (`editor-crepe.ts` `uploadFile` → `POST /.mbr/upload`, reachable from the upload button, drag-drop and paste), but every result path targets a ProseMirror body node; nothing writes a frontmatter key, so the portrait is still set by hand-typing `image:` into the raw YAML textarea.

* [ ] Export to PDF
  * _After research, my options here are pretty ugly. I don't want to compile in chromium or anything and don't want to rely on it being installed in a common place, either. Current browser widget I use doesn't give me a print to pdf option. Need to look for a reasonable way to make this happen cross platform with reliable output._
  * Start with the current page as an option.
  * Also allow a print to PDF for the whole site (essentially taking a doc site and compiling everything into chapters in a single PDF).
  * All of this to live only in the GUI app via menu bar items with cmd-<key> shortcuts.
  * Printing of the compiled book too (plain per-page printing is already there: File → Print, cmd/ctrl+P, `webview.print()`)
  * On MacOS, printing would probably be enough because the user could export to PDF, but because this is cross-platform, it would be nice if we can find a good way to do this anywhere.
  * CLI tool should support direct markdown to PDF options, too, including for the "book" mode compiling all markdown listed in a sidebar into a single document.
  * When building a book, start with a full page title page then a page with the table of contents, then the converted markdown in any specified order or default order. Align with the GUI for ordering and labeling.
  * Make sure to handle edge cases like extra long titles.

* **Big repo (goodwiki) issues**
  * [ ] Finish pagination — the browse components have it, nothing else does. Done: `mbr-browse` middle pane and `mbr-browse-single` top-level folders/root files (100 per page + "Show more"), the media browser (200), and tag pills. Still unbounded: the recursive folder tree in both components, `_renderDynamicSections`, the hierarchical tag tree, and every server-rendered template — `home.html`, `section.html`, `tag.html` and `tag_index.html` all loop the complete list with no slice.
    * [ ] `sidebar_max_items` is dead config. It's validated in `config.rs` and injected into every template context, but no template ever emits a `max-items` attribute, so the components always fall back to their hardcoded 100.
    * [ ] The home page no longer enumerates the whole site to render — it's scoped to direct children and falls back to a non-recursive scan while the background scan runs. What still blocks is the sidebar: `shared.ts` fetches `/.mbr/site.json` on every page, and that handler awaits `wait_for_scan()`.
  * [ ] wikilinks and the link checker: underscore-prefixed files (e.g., _...Baby One More Time Tour.md) - files with special chars were renamed with underscores but internal links weren't updated -- none of those work yet. not sure what to do
    * Need to look into the spaces vs. underscores stuff a bit here too
    * Answer: only if we submit PRs to pagefind or switch to something else
  * [ ] Media scanning / populating media metadata is slow on large repos. Images take 2 to 10ms. PDFs can take a whole minute. Video files 30 to 50ms.  In practice, on the Magic repo, it takes many minutes (10?) to complete a first pass. The parallelism half is done — population is rayon-parallel and runs as a later phase of the background scan, so `site.json` no longer waits on ffmpeg/lopdf. What's left is the actual cost, and the suspicion in this item was right: nothing reads headers only.
    * [ ] PDFs get fully parsed **twice** by `lopdf::Document::load` — once in `pdf_metadata.rs` for `Info` + page count, again in `repo.rs` for search text. Wants a trailer/incremental read and a single pass.
    * [ ] Images go through the same ffmpeg-backed `MediaFileMetadata::new` as video, even though the `image` crate is already a dependency and could read dimensions from the header.
    * [ ] Nothing is lazy: `populate_media_metadata` walks every entry in `other_files` up front rather than on demand.
    * [ ] No faster-library evaluation has happened yet (`metadata`, `ffmpeg-next`, `lopdf` all unchanged).
    * [ ] `/.mbr/media.json` still hard-blocks on `wait_for_media()`, so the media browser is unusable for the entire ~10 minutes rather than filling in progressively.

* [ ] We should change it so on open of the app without any specified dir (or the root as assumed), we pop up some sort of splash page where the user can select from recents or select open. Maybe give some info on the app. Today that case shows a bare `rfd` native folder picker and exits on cancel (`main.rs` `needs_folder_picker`/`show_folder_picker`) — that's the stopgap to replace. Nothing tracks recently-opened *folders*; the only "recent" list is recently-viewed files within a repo, in localStorage.

* **Publish**
  * [ ] Publish to determinate's flake hub?
  * [ ] Any publishing to linux repos?

* **Windows**
  * [ ] Consider a hybrid CRT: static vcruntime + *dynamic* UCRT. The `+crt-static` in `.cargo/config.toml` fixes the missing-VCRUNTIME140.dll crash on clean installs, but it also freezes the UCRT into the binary, so the UCRT security fixes Microsoft ships via Windows Update never reach mbr.exe. Tauri does the hybrid for this same wry/tao/webview2 stack, and [rust#153568](https://github.com/rust-lang/rust/issues/153568) proposes it as the Windows default. Not free: the link args differ between release (`/nodefaultlib:libucrt.lib` + `ucrt.lib`) and debug (the `...d.lib` variants), and config-file rustflags cannot vary by profile — `cargo test` on the Windows CI legs is a debug build — so doing it properly means a build-script dependency such as `static_vcruntime`. The existing CI import assertions work unchanged either way.

* [ ] Need to produce robots.txt and/or sitemap.xml files (robots pulled from .mbr so user can override)? We would need some custom frontmatter to cause something to be left out or even ignored. We also need to use last update or date field to push into sitemap too.  But our "everything is relative" idea falls apart since the sitemap needs to know the full URL of the content (hostname, prefix path, etc.) so maybe we'd only build it if that's specified.

* [ ] Pull in lightningcss and auto combine and minify the pico.min.css + theme.css + user.css files.

* [ ] Make demo videos
  * Quick highlight reel
  * Demo quicklook
  * Demo simple preview
    * Show live updates
    * Show how it finds images
    * Show how links are fixed automatically
  * Demo markdown supported extensions
  * Demo rich media -- media browser, video, inline pdf
    * Covers, dynamic chapters and captions, and dynamic downscaling, too
  * Demo oembed bare links
  * Demo speed
  * Demo slides
  * Demo flashcards
  * Demo relationships/genealogy
  * Demo contacts
  * Demo tags
  * Demo customizing look/feel/frontend
  * Demo search and browse
    * Show advanced things like ordering
    * No, there is no hide-from-browse feature — no frontmatter key (`hidden`/`draft`/`private`/`unlisted`) is read anywhere; only dotfiles are skipped, by the scanner. Should there be one? (Would also unblock the sitemap exclusion in the robots.txt item below.)
  * Long video 1: Basics
    * Explain why, show Marked app, demonstrate links working in different contexts
    * Show navigation working
    * Show browse and search
    * Show media search
    * Keyboard oriented
    * Fast
    * Cross-platform
  * Long video 2: Major features
    * Slides
    * Kanban
    * Tasks
    * Relationships
  * Long video 3: Customizability
    * Templates
    * Themes
    * Added custom themes


