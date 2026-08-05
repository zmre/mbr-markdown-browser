# TODO

## What's Next

* **Relationships & genealogy** (see [docs](docs/markdown/relationships.md))
  * [ ] Edit-mode support for structured person data: a friendlier way to view/edit the person frontmatter (born, died, born_place, gender, aliases, relationships) than hand-editing raw YAML in the in-browser editor — e.g. a small form for the known fields.
  * [ ] Display the person `image` everywhere a person surfaces (infobox done; consider graph nodes and link/hover previews).
  * [ ] Edit the person `image` from edit mode — pick/replace the portrait. If the in-browser editor can't upload images yet, add image/photo upload to the editor and wire it to the `image` field.

* **Tasks** (see [docs](docs/markdown/tasks.md))
  * [ ] Scan for `TODO:` / `FIXME:` style markers in source files as well as markdown checkboxes, driven by a configurable list of regexes. Deferred from TASKS_SPEC.md's opening paragraph; needs a decision on what "a task" means when there is no checkbox to toggle.
  * [ ] An edit-token prompt that is not the editor. `POST /.mbr/task` needs the same bearer token as `/.mbr/edit`, and the only place to enter one is the editor's footer. The loop is closed enough to be usable — a refused click says so, `edit-token.ts` records the refusal, and the editor opens with its token field already visible — but "press `e` and find the footer" is still a detour to reach a one-field form. The token is per page load by design (it lives in memory, never in web storage), so this is asked once per page on a token-protected server; a small inline prompt on the 401 would remove the detour. Worth doing *only* as a prompt that writes through `setEditToken`; anything that persists the token to make it "convenient" is the thing this design exists to avoid.
  * [ ] Editing a task's *text* (not just its status) still means opening the file. A single-line inline edit in the panel would cover most of what the editor is used for on a daily note.

* [ ] Can we allow cmd-f (ctrl-f elsewhere except that's page down so...) to do in-page search in gui? works fine already in browser so this should be part of the gui shell probably, not in-page javascript.
* [ ] Should we allow tabs for viewing multiple markdown files in one session (gui)?
* [ ] CriticMarkup support?
* [ ] Export to PDF
  * _After research, my options here are pretty ugly. I don't want to compile in chromium or anything and don't want to rely on it being installed in a common place, either. Current browser widget I use doesn't give me a print to pdf option. Need to look for a reasonable way to make this happen cross platform with reliable output._
  * Print stylesheet support and light background default (though I guess we could make a dark background PDF).
  * Start with the current page as an option.
  * Also allow a print to PDF for the whole site (essentially taking a doc site and compiling everything into chapters in a single PDF).
  * All of this to live only in the GUI app via menu bar items with cmd-<key> shortcuts.
  * Allow printing while we're at it
  * And printing of the compiled book, too
  * On MacOS, printing would probably be enough because the user could export to PDF, but because this is cross-platform, it would be nice if we can find a good way to do this anywhere.
  * CLI tool should support direct markdown to PDF options, too, including for the "book" mode compiling all markdown listed in a sidebar into a single document.
  * When building a book, start with a full page title page then a page with the table of contents, then the converted markdown in any specified order or default order. Align with the GUI for ordering and labeling.
  * Make sure to handle edge cases like extra long titles.
* [ ] Copy rendered to clipboard
  * Again GUI mode only and via menu bar items. Export the rendered HTML as whatever rich text format is native so it can be pasted into emails.  In this case, make background and text foreground (aside from links and colored items) neutral so it can be pasted into different environments so we don't get white text pasted into a white background email.
  * Copy should assume whole document unless there's a selection

* **Big repo (goodwiki) issues**
  * [ ] In mbr-browser and index pages, we need some limit on the number of things shown (tags, files, etc.) or some sort of pagination
    * [ ] The home page currently shows all pages on the site, which means processing all files before loading the index, which in dynamic mode sucks.
  * [ ] wikilinks and the link checker: underscore-prefixed files (e.g., _...Baby One More Time Tour.md) - files with special chars were renamed with underscores but internal links weren't updated -- none of those work yet. not sure what to do
    * Need to look into the spaces vs. underscores stuff a bit here too
    * Answer: only if we submit PRs to pagefind or switch to something else
  * [ ] Media scanning / populating media metadata is slow on large repos. Images take 2 to 10ms. PDFs can take a whole minute. Video files 30 to 50ms.  In practice, on the Magic repo, it takes many minutes (10?) to complete a first pass.  I want to research ways to speed this up. I want to make sure we are doing what we can in parallel and I want to see if the libraries we're using have competitors that are faster. I also want to understand what information we're gathering that's slow. Most metadata should be at the front of the file so we shouldn't have to read entire PDFs or video files when processing, but I suspect that's not the case.

* [ ] We should change it so on open of the app without any specified dir (or the root as assumed), we pop up some sort of splash page where the user can select from recents or select open. Maybe give some info on the app.

* **Publish**
  * [ ] Publish to a homebrew cask?
  * [ ] Publish to determinate's flake hub?

* **Windows**
  * [ ] Consider a hybrid CRT: static vcruntime + *dynamic* UCRT. The `+crt-static` in `.cargo/config.toml` fixes the missing-VCRUNTIME140.dll crash on clean installs, but it also freezes the UCRT into the binary, so the UCRT security fixes Microsoft ships via Windows Update never reach mbr.exe. Tauri does the hybrid for this same wry/tao/webview2 stack, and [rust#153568](https://github.com/rust-lang/rust/issues/153568) proposes it as the Windows default. Not free: the link args differ between release (`/nodefaultlib:libucrt.lib` + `ucrt.lib`) and debug (the `...d.lib` variants), and config-file rustflags cannot vary by profile — `cargo test` on the Windows CI legs is a debug build — so doing it properly means a build-script dependency such as `static_vcruntime`. The existing CI import assertions work unchanged either way.

* [ ] Need to produce robots.txt and sitemap.xml files (robots pulled from .mbr so user can override)? We would need some custom frontmatter to cause something to be left out or even ignored. We also need to use last update or date field to push into sitemap too.  But our "everything is relative" idea falls apart since the sitemap needs to know the full URL of the content (hostname, prefix path, etc.) so maybe we'd only build it if that's specified.

* [ ] Components are currently bundled as mbr-components.js and loaded as a single file, which is great, but we want to allow for more fine-grained overrides.  The better behavior here is for us to assemble a mbr-components.js file from a set of individual files allowing for user overrides to those files.  A static build will have a single mbr-comonents.js file and a dynamic one will concatenate each component file in a particular dir together first checking for per-repo or templates dir overrides.
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
    * Did I make a hide from browse feature? Should I?
