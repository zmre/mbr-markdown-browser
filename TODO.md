# TODO

## Findings

P0 — Security (fix before any non-localhost or untrusted-content use)

  1. Path traversal in the HLS transcode endpoint — src/server.rs:2677-2693. Both Rust agents independently confirmed this. try_serve_hls_content joins config.base_dir.join(&video_path)
  with no canonicalize/containment check, and it's reached precisely when normal resolution returnsNotFound (i.e. .. paths that safe_join rejected). With --transcode on, a client can
  probe/transcode/read .mp4 files anywhere on disk. The sibling handlers (try_serve_video_metadata, try_serve_pdf_cover) already call validate_path_containment — this branch just needs
  the same. Mitigated today only by --transcode defaulting off. Small.
  2. XSS via unescaped remote OpenGraph/favicon metadata — src/oembed.rs:294-324, 73-78. html_escape::encode_text doesn't escape quotes, and html() builds single-quoted <img
  src='{}'>/<a href='{}'> from a remote page's og:image/favicon. A malicious site returning og:image = x' onerror='… injects script. Oembed is on by default in server/GUI mode, so
  browsing a note with a bare URL to a hostile site runs attacker JS. Fix: encode_double_quoted_attribute + double quotes. Small.
  3. SSRF in oembed fetching — src/oembed.rs:229-244. Fetches any bare URL from markdown, follows up to 10 redirects, with no blocking of localhost / private / link-local ranges (e.g.
  169.254.169.254), and reflects results into the page. Reject private/loopback/link-local on every redirect hop, or allowlist. Medium.
  4. Unbounded memory from fetched content — src/oembed.rs:244. response.text().await reads the whole body with no size cap or content-type check; a multi-GB response exhausts memory,
  then Html::parse_document burns CPU. Stream with a ~512KB–1MB cap and require HTML content-type. Small.
  5. No warning when binding non-localhost — src/main.rs:134-146. --host 0.0.0.0 exposes unauthenticated expensive ops (ffmpeg transcode, PDF extraction, repo-wide grep, full-text
  search) plus full repo read. At minimum print a prominent warning; consider a token/rate-limit. Small.

  P1 — Correctness bugs

  6. Lost-wakeup hang on concurrent HLS requests — src/server.rs:2771,2876 + video_transcode_cache.rs:112-141. Notify::notify_waiters() stores no permit; if completion fires between
  start_generation and .await, the waiter hangs forever with no timeout. Create the Notified future before re-checking state (or use watch/oneshot), and add a timeout. Medium.
  7. Temp-file collision corrupts transcoded segments — src/video_transcode.rs:458-463. Temp path is mbr_segment_{pid}_{index}.ts; concurrent transcodes of the same index for different
  videos/resolutions collide, and failures leak the temp file. Use tempfile::NamedTempFile. Small.
  8. build_static_url_path corrupts URLs — src/repo.rs:1185. .replacen(static_folder, "", 1) strips the first occurrence anywhere, so notes/static-analysis/img.png →
  /notes/-analysis/img.png. Use Path::strip_prefix on a leading {static_folder}/. Small.
  9. build_markdown_url_path false index match — src/repo.rs:1161. url.ends_with(index_file) matches docs/myindex.md → truncates to /docs/my. Compare the file-name component. Small.
  10. Markdown files silently dropped without btime — src/repo.rs:1084. metadata.created()? fails on filesystems lacking birth time (older Linux, NFS); the file then vanishes from
  listings/search with only a warn. Fall back to modified. Small.
  11. Handler panic on multi-byte tag source names — src/server.rs:3394,3471. capitalized + &source[1..] byte-slices; a source like "étiquettes" panics the request. Extract one helper
  using len_utf8() (also DRY — duplicated in two handlers). Small.
  12. Cache size-accounting drift on overwrite — video_transcode_cache.rs:174, video_metadata_cache.rs:115, link_index.rs:283, link_grep.rs:113. insert over an existing key fetch_adds
  without subtracting the replaced entry's size, so current_size ratchets up → ever more aggressive spurious eviction. Subtract the old size. Small.
  13. Video metadata cache never invalidated — video_metadata_cache.rs:185. Key is URL+type, no mtime, no watcher hookup; edited videos/PDFs serve stale covers/chapters until restart,
  and NotAvailable negatives are permanent. Include mtime or hook the watcher. Medium.
  14. Failed HLS entries poisoned forever — video_transcode_cache.rs:196-239. fail_generation stores Failed permanently; clear_failed is #[allow(dead_code)] and never called. One
  transient error 422s that playlist until restart. Give Failed a TTL. Small.
  15. Watcher ignores ignore_globs — src/watcher.rs:88. _ignore_globs is bound and never used, so ignored files still trigger reloads/invalidations. Apply should_ignore in the callback.
  Small.

  P1 — Performance (contradicts the "sub-second on 10k+ files" goal)

  16. Blocking I/O and CPU work in async handlers — repo-wide grep for links.json (server.rs:2352), fs::read_to_string in render_with_cache (markdown.rs:601), directory scans built
  inline (server.rs:3218), and full ffmpeg decode in try_serve_video_metadata (server.rs:1892-1936, no spawn_blocking — unlike the exemplary PDF path). These stall tokio workers for
  seconds under load. Wrap in spawn_blocking / use tokio::fs. Medium.
  17. Per-request ffmpeg probe before cache lookup — server.rs:2696. probe_video_resolution runs on every HLS request before the cache is consulted, so cached hits still pay a blocking
  demux. Move behind the cache miss and cache resolution per path+mtime. Small.
  18. O(n) full-repo scans per page render — server.rs:3131-3147 iterates all markdown files to find siblings on every render; directory_to_html re-scans from disk per request;
  build.rs:1172-1198 is O(dirs×files) for section pages. The build path already has the right pattern (sibling_index, build.rs:493) — reuse it. Medium.
  19. New reqwest Client + rustls config per oembed URL — oembed.rs:209 + lib.rs:13-25 rebuilds the full webpki root store per bare URL per render and discards connection pooling. Share
  a LazyLock<Client>. Small.
  20. No single-flight for video metadata — server.rs:1890. A gallery with N videos fires N concurrent ffmpeg decoders (thundering herd, compounding #16). Reuse the HLS single-flight
  mechanism. Medium.
  tag/SHA. Small.
  22. Dependabot breaks the Nix component build — dependabot.yml:20-23 bumps components/ but nothing updates package-lock.json or the flake's npmDepsHash, so every frontend dependabot
  PR breaks mbr-components. Add a CI sync-or-fail step, and consolidate the dual bun/npm lockfiles (finding also flagged separately). Medium.
  23. Darwin-only flake outputs break nix flake check on Linux — flake.nix:356,380,565,578. optionalAttrs isDarwin (...) yields an empty set, not a derivation (verified via nix eval).
  Build the package set with {...} // lib.optionalAttrs isDarwin {...}. Small.
  24. Impure host-tool refs in derivations — flake.nix:501-508,634-667 call /usr/bin/codesign and /usr/bin/install_name_tool; works only because sandboxing is off. Use
  pkgs.darwin.sigtool/cctools. Medium.
  25. Release profile drift — Cargo.toml:132: codegen-units = 16 vs the skill's 1 for this perf-critical tool. Also panic = 'abort' means any background-thread panic hard-kills the GUI
  and any .appex panic aborts the host QuickLook process — document as deliberate or use unwind for gui/ffi. Small.
  26. Pre-commit clippy misses --all-targets — .githooks/pre-commit:71 diverges from CI and CLAUDE.md, so test-code lints only surface in CI. Small.
  27. Cache-hostile source filter + committed .cargo/config.toml jobs=8 — flake.nix:149-170 invalidates src on any *.md edit (docs PRs force full rebuilds); the committed jobs=8 imposes
  a machine-specific cap on all contributors. Small each.

  P2 — Tests / Docs / Frontend

  28. Fresh checkout fails the mandatory quality gates — cargo test/build fail at server.rs:3773 because templates/components-js/mbr-components.min.js is gitignored and only produced by
  cd components && bun run build. CLAUDE.md's "MANDATORY" checks never mention the prerequisite. Add it (or a build.rs stub with a clear error). Small.
  29. Docs drift vs code (violates repo's own mandatory policy) — docs/reference/cli.md omits -p/--port, --host, --theme, -o/--stdout; configuration.md omits build_concurrency,
  template_folder, theme, sort. Small.
  30. CLAUDE.md is stale — test counts self-contradict ("~462"/"~354"/"~274"; actual 1082) and the module table is missing ~16 of 40 modules, giving agents a wrong map. Small.
  31. Timing-based test startup is a latent CI flake — server_integration.rs:69 uses fixed sleep(100ms) + a TOCTOU find_available_port; 139 concurrent servers can collide. Bind the
  listener in-test or poll readiness. Medium.
  32. Largest frontend components untested + as any coupling — the 6 test files cover pure helpers; mbr-search.ts (1208 LOC), mbr-keys.ts, mbr-media-browser.ts etc. have zero tests, and
  mbr-keys.ts reaches into siblings' private state via as any (a rename silently breaks shortcuts). Start with mbr-keys/mbr-search; expose typed interfaces. Large.

  Lower-priority quality items (Rust unwrap/assert/Box<dyn Error> in a few spots vs the no-panic/thiserror standard, per-call regex compile in quicklook.rs:234, the vid.rs
  apostrophe-in-path regex edge case, DRY: five copy-pasted cache modules and duplicated template-context assembly, non-hermetic oembed unit test, noisy test output, single 230KB JS

## What's Next

* [ ] Should we allow tabs for viewing multiple markdown files in one session?
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
  * Demo tags
  * Demo customizations
  * Demo search and browse
    * Show advanced things like ordering
    * Did I make a hide from browse feature? Should I?
