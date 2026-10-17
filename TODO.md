# TODO

## What's Next

* **Quicklook**
  * Lets setup the thumbnail generation, too
* **GUI**
  * Right now our icons for things are all different styles.  We don't have a single style language. For example, the i button for info is a heavy solid thing that might be a unicode character. Tasks is a clipboard (which doesnt read as tasks) that's a simple outline.  There are more examples through the interface, like the folders used in the browser.  Also, it's inconcistent in the header how we have a border around the magnifying glass icon but not around other icons.
  * I want to do a full review of buttons that use icons throughout the interface, pick a design language for them, and bring consistency so it looks well designed.
* **Mobile**
  * [ ] No way to enter review mode on mobile (via text selection or "..." menu)
  * [ ] Plain browsers have no visible "New note" control outside the editor (only the editor's ⋯ menu); consider a shortcut or a button on listings.
  * [ ] Device checks still open in `apple/ios/TESTING.md`: an hour in the background then return (§13), Working Copy as a provider (§9), the dark-mode folder picker and whether the Grant Folder Access banner names "Dropbox" (`picker: provider name …` log, §10).
  * [ ] Grant Folder Access: iOS 27 ignores the picker's `directoryURL` for third-party File Provider folders (Dropbox, observed 2026-10-16), so the picker opens at the last folder used; the banner naming the folder is the workaround. Revisit if a later iOS honours it.
  * [ ] Refresh on a cloud-provider vault waits up to 2 s for the provider (`ProviderRefresh`) even when nothing changed; consider a shorter budget or skipping it when the provider reports the folder current.
  * [ ] First open of a large iCloud vault on a device that has never listed it is slow while folder listings come down (iPad: 16.5 s scan for ~4,450 files; a 705-file vault: 19 s); the page shows at once from what is local, but there is no indication that more is arriving. Consider a "Still listing folders from iCloud…" hint, and Browse showing folders that hold no notes or are still listing.
* **Editor follow-ups** (the Milkdown plugins for mbr's syntax are done — see `components/src/editor-plugins/`, the parity harness and `docs/modes/editing.md`)
  * [ ] Re-run `roundtrip-check` on a real repository after the 2026-10-16 fixes (on the anonymized repros of the first run: 2,963 → 271 still change on a full re-serialize, 0 rewritten by any save, the open-time notice on 7% of notes) and check the 2 earlier `TypeError: … reading 'length'` errors are gone.
  * [ ] A nested list that mixes tabs and four spaces within one list prints with 2-space indentation once that list is edited (one indentation is recorded per item; ~80 notes in the run).
  * [ ] Lazy continuation lines inside footnote definitions are indented when the footnote is edited.
  * [ ] Notes with old Mac (CR-only) line endings are saved with LF (same meaning).
  * [ ] Scattered blank-line cases still normalized when the block is edited: an empty item under a quote, an HTML block directly against a paragraph, whitespace-only lines next to an edited block.
  * [ ] An authored unneeded escape in a table cell or a definition term is dropped when that block is edited (paragraphs and headings keep theirs via `mbrStyle.source`); `x\\y` saves as `x\y` (same text).
  * [ ] A list item whose first block is a definition list holds an empty filler paragraph in the editor (Milkdown's `list_item` must start with a paragraph); it is dropped on save but visible as an empty line above the term while editing.
  * [ ] Two definition lists separated only by a deleted block become one list (markdown cannot write two adjacent lists apart) — the save writes the merged list; expected, but the editor shows them as two until reload.
  * [ ] No typed way to create a `--- {attrs}` section break in the editor (Crepe's `---` input rule makes a plain rule on the third dash); existing ones are editable.

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


