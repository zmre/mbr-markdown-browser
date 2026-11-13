# TODO

## What's Next

* **Mobile**
  * [ ] No way to enter review mode on mobile (via text selection or "..." menu)
  * [ ] Device checks still open in `apple/ios/TESTING.md`: an hour in the background then return (§13), Working Copy as a provider (§9), the dark-mode folder picker and whether the Grant Folder Access banner names "Dropbox" (`picker: provider name …` log, §10). Also re-measure the provider refresh on a device now that an unchanged folder stops waiting 300 ms after the coordinated read (was the full 2 s).
  * [ ] First open of a large iCloud vault on a device that has never listed it is slow while folder listings come down (iPad: 16.5 s scan for ~4,450 files; a 705-file vault: 19 s); the page shows at once from what is local, but there is no indication that more is arriving. Consider a "Still listing folders from iCloud…" hint, and Browse showing folders that hold no notes or are still listing.
* **Editor follow-ups** (the Milkdown plugins for mbr's syntax are done — see `components/src/editor-plugins/`, the parity harness, `roundtrip-check` and `docs/features/editing.md`). On a 3,153-note repository no note is rewritten by an ordinary save; accepted as good enough (2026-10-16): when a block *is* edited, a few cosmetic normalizations remain — CR-only line endings become LF, some blank lines between adjacent lists of different kinds or after a list ending in code, trailing blank lines after an unclosed final fence, whitespace-only lines near a quote, a paragraph's indent after an ordered list, a definition's `  : ` indent, lazy lines inside footnotes are indented, an authored unneeded escape in a table cell or term is dropped.
  * [ ] A list item whose first block is a definition list holds an empty filler paragraph in the editor (Milkdown's `list_item` must start with a paragraph); it is dropped on save but visible as an empty line above the term while editing.
  * [ ] Two definition lists separated only by a deleted block become one list (markdown cannot write two adjacent lists apart) — the save writes the merged list; expected, but the editor shows them as two until reload.
  * [ ] No typed way to create a `--- {attrs}` section break in the editor (Crepe's `---` input rule makes a plain rule on the third dash); existing ones are editable.

* **Third-party assets**
  * [ ] Update the vendored highlight.js (`templates/hljs*.js`) once upstream releases past 11.12.0 (still the latest on npm as of 2026-11-12): CodeQL flags `js/bad-tag-filter` in `hljs.11.12.0.js` and `hljs.lang.xml.11.12.0.js` (alerts 88/89, dismissed as third-party on 2026-10-17 pending the update).

* **Relationships & genealogy** (see [docs](docs/features/relationships.md))
  * [ ] Wire the properties editor's `image` field to upload/pick — pick/replace the portrait. It is a plain text field today (`frontmatter/fields.ts` `text('image', …)`); upload exists only for the body (`editor-crepe.ts` `uploadFile` → `POST /.mbr/upload`). Factor `uploadFile` out and give `image` a widget that writes the returned path.

* **Big repo (goodwiki) issues**
  * [ ] Finish pagination — the browse components have it, nothing else does. Done: `mbr-browse` middle pane and `mbr-browse-single` top-level folders/root files (`sidebar_max_items` per page, default 100, + "Show more"), the media browser (200), and tag pills. Still unbounded: the recursive folder tree in both components, `_renderDynamicSections`, the hierarchical tag tree, and every server-rendered template — `home.html`, `section.html`, `tag.html` and `tag_index.html` all loop the complete list with no slice.

* **Windows**
  * [ ] Consider a hybrid CRT: static vcruntime + *dynamic* UCRT. The `+crt-static` in `.cargo/config.toml` fixes the missing-VCRUNTIME140.dll crash on clean installs, but it also freezes the UCRT into the binary, so the UCRT security fixes Microsoft ships via Windows Update never reach mbr.exe. Tauri does the hybrid for this same wry/tao/webview2 stack, and [rust#153568](https://github.com/rust-lang/rust/issues/153568) proposes it as the Windows default. Not free: the link args differ between release (`/nodefaultlib:libucrt.lib` + `ucrt.lib`) and debug (the `...d.lib` variants), and config-file rustflags cannot vary by profile — `cargo test` on the Windows CI legs is a debug build — so doing it properly means a build-script dependency such as `static_vcruntime`. The existing CI import assertions work unchanged either way.

* [ ] Need to produce sitemap.xml file (robots pulled from .mbr so user can override)? We would need some custom frontmatter to cause something to be left out or even ignored. We also need to use last update or date field to push into sitemap too.  But our "everything is relative" idea falls apart since the sitemap needs to know the full URL of the content (hostname, prefix path, etc.) so maybe we'd only build it if that's specified.

* [ ] Pull in lightningcss and auto combine and minify the pico.min.css + theme.css + user.css files.
