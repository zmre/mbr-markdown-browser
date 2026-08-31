# TODO

## What's Next

* [ ] Reveal.js has a new major version, 6.x, we need to update to it
  * API stays the same
  * The HTML and CSS are *not* wildly changed: v6 removes zero CSS classes (adds two), and `.reveal > .slides > section` plus every `Reveal.initialize()` option are unchanged. The real work is our three `reveal.theme.*.css`, which are patched forks (Source Sans Pro `@import` stripped, globals removed so they inherit from Pico; `blank` has no upstream counterpart at all) and would need re-deriving against v6 — where themes now inline their fonts as base64, taking `black.css` from 7 KB to 575 KB, so the strip is worth keeping.
  * `scripts/update-assets.sh --reveal 6.0.1` handles the mechanical part (core + `dist/reveal.css` + the notes plugin, which moved to `dist/plugin/notes.js`) and deliberately leaves the themes alone.
* **Relationships & genealogy** (see [docs](docs/markdown/relationships.md))
  * [ ] Edit-mode support for structured person data: a friendlier way to view/edit the person frontmatter (born, died, born_place, gender, aliases, relationships) than hand-editing raw YAML in the in-browser editor — e.g. a small form for the known fields.
  * [ ] Wire the editor to the person `image` frontmatter field — pick/replace the portrait. Image upload itself is done (`editor-crepe.ts` `uploadFile` → `POST /.mbr/upload`, reachable from the upload button, drag-drop and paste), but every result path targets a ProseMirror body node; nothing writes a frontmatter key, so the portrait is still set by hand-typing `image:` into the raw YAML textarea.

* [ ] Prose critique support
  * There are two use cases here: one, someone sends me something to review and I want a way to make edits inline.  CriticMarkup is a reasonable approach although usually when I'm in this scenario, the markdown is in github and we're commenting on diffs.
  * Most of the time though I don't really want or need critic markup (viewing or generating). It doesn't really come up for me.  However, I would like to be able to indicate suggested changes and comments in the GUI.  Moreover, I want to be able to add notes and suggestions.
    * The primary use case for this these days is to generate feedback to AI that captures file, line number, and type of change.
    * To that end, take a look at github:zmre/pwnvim and what I'm doing there.  It can pull review comments from GitHub, but can also generate them on specific lines.  Then I can export the review info as markdown that looks like this:

    ```markdown
    # Code Review

    1. **[SUGGESTION]** `flake.nix:26`
      Did you know that numtide's llm-agents repo https://github.com/numtide/llm-agents.nix has omp?  And they build into binary caches, which would speed this up (probably) if you used that as the input.

    2. **[NOTE]** `flake.nix:35`
      If you use the numtide version, this update stuff can go away - you just need to update the flake lock

    3. **[QUESTION]** `PLAN.md:7`
      Not sure what it means by the four-cli toolchain dead weight. The priv options? The other codex and gemini stuff?

    4. **[NOTE]** `PLAN.md:15`
      I dunno, I kind of like the predictable wrapup. When I use claude without it I just have a wall of text to sort through.
    ```

    * And in the UI it has little markers showing when there's a comment (and we already have a facility for this with `>>>` blocks, I think).
    * So ideally I'd be able to add a bunch of feedback in a doc and then copy it out as markdown, but also view it inline. If suggesting changes, it would show the diffs inline and work sort of like how github comments work in that case.  But instead of showing diffs inline in a unified style, something more like criticmarkup styles (cross outs for delete, green text for additions, etc.).  But for now we don't need to use criticmarkup in the markdown file or to be able to read or save that.
    * We'd want to store the pending comments in some sort of localstorage so a reload of the file when it changes doesn't obliterate what we already have.
    * Need to work out how to adjust line numbers of comments if we edit the file (assuming edit mode is on).  If it is edited on the server, that might be tougher / unsupported.
    * pwnvim doesn't yet handle pushing things back up to github review comments, nor adding change suggestions as diff recommendations, but we will want to support those things.
    * I think no icon in the header is necessary. We'll make this trigger in one of two ways: by pressing `r` to make a generic note (no specific line), or by selecting text and then we show a floating menu item about adding a note, which can be triggered also with `r` but this time with an anchoring spot. Capital `R` should bring up a list of everything and when there is anything, a floating button icon of a chat bubble in the bottom right should do the same.  From the list of all there should be a copy button/icon to copy out the markdown.  When adding, it should work similarly to github reviews so in our case we'll have a list of types of notes in a dropdown and if "suggestion" is chosen, we'll give a code block with the selected line(s) so it can be modified.
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
  * [ ] Publish to a homebrew cask?
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
  * Demo relationships/genealogy
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
