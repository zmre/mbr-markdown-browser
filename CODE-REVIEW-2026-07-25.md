# mbr — Full Code Review

**Date:** 2026-07-25 · **Branch:** 2026-07-25-code-review · **Scope:** all of `src/` (~42k lines Rust), `components/src/` (~26k lines TS), `templates/`, `tests/`, `benches/`, `.github/workflows/`, `flake.nix`

**Method:** 15 scoped reviewer agents → 88 findings → each finding adversarially verified against the real code by an independent agent (critical/high got a second verifier on an impact/exploitability lens) → 1 refuted and dropped, 87 survived. 144 agents total.

| Severity | Count |
|----------|------:|
| Critical | 0 |
| High | 5 |
| Medium | 48 |
| Low | 34 |

---

## Security (20 findings)

### [HIGH] safe_join fallback returns symlinks that escape the repo root (arbitrary file read)
`src/path_resolver.rs:49`

When `candidate.canonicalize()` succeeds but resolves *outside* `canonical_base`, `safe_join` does not return `None` — it falls through to the "path doesn't exist yet" branch, which validates only the *parent* and re-joins the raw file name, handing back the unresolved symlink. `resolve_request_path` then classifies it with `is_file()`/`is_dir()` (both follow symlinks) and `Server::handle` serves it via `ServeFile` with no containment re-check. Verified live against the debug binary: a repo containing `passwd -> /etc/passwd` returns the real file on `GET /passwd` (200, full contents); relative symlinks like `../../tmp/secret.txt` work too, so an attacker needs no knowledge of the victim's paths. Only the *final* path component can escape (nested `/etclink/hosts` correctly 404s), which makes the behavior incoherent rather than an intentional feature — the codebase enforces this exact boundary in `find_in_static_folder`, `safe_join_asset`, and `resolve_editable_markdown`, and even has a test named `test_symlink_escape_blocked` that never exercises `safe_join`.

```rust
// For paths that don't exist yet (checking markdown extensions),
// we need to verify the parent is safe and construct the full path
if let Some(parent) = candidate.parent()
    && let Ok(canonical_parent) = parent.canonicalize()
    && canonical_parent.starts_with(canonical_base)
    && let Some(filename) = candidate.file_name()
{
    return Some(canonical_parent.join(filename));
}
```

**Fix:** Bind the canonicalize result first; if it returned `Ok(c)` and `!c.starts_with(canonical_base)`, return `None` immediately instead of falling through. Add a containment re-check on `StaticFile`/`MarkdownFile`/`DirectoryListing` in `Server::handle`, mirroring `resolve_editable_markdown` (server.rs:1852-1858).

### [HIGH] Edit-token auth is bypassed entirely behind the documented reverse proxy
`src/server.rs:1786`

`check_edit_access` derives `require_token` from the raw TCP peer IP, and there is no `X-Forwarded-For` handling anywhere (`grep forwarded src/` returns zero hits). In the deployment `docs/modes/editing.md:130-137` recommends for remote editing — mbr on loopback behind a TLS-terminating proxy — every proxied request arrives from 127.0.0.1, so the configured Argon2 `edit_token_hash` is never checked. `Config::validate()` (config.rs:673) only demands a token when the *bind host* is non-loopback, so this config passes validation and the operator gets false assurance.

```rust
let caller_is_loopback = peer_ip.is_loopback();
let require_token = !caller_is_loopback || config.edit_require_token_on_loopback;
```

The other gates do not help: the CSRF check only requires a literal `X-MBR-Edit: 1` header, and `is_same_origin` returns `true` when no `Origin` header is present — both trivial for `curl`. `test_edit_roundtrip_loopback_no_token` (tests/server_integration.rs:3940) already demonstrates the exact attacker request succeeding. Blast radius: `/.mbr/raw` (read any markdown), `/edit`, `/create`, `/move`, `/mkdir`, `/upload`.

**Fix:** `let require_token = config.edit_token_hash.is_some() || !caller_is_loopback || config.edit_require_token_on_loopback;` — if an operator configured a token, always enforce it. Update `docs/modes/editing.md` accordingly.

### [HIGH] No Host-header validation: DNS rebinding defeats the documented CSRF protection
`src/server.rs:1813`

`is_same_origin` compares `Origin`'s host to the request's own `Host` header, and nothing validates that `Host` names the bound address — `grep -rn "header::HOST"` over `src/` returns exactly one hit, this comparison. The only router layers are `CompressionLayer` and `TraceLayer`. Under DNS rebinding the attacker's document genuinely *is* same-origin (browser sends `Sec-Fetch-Site: same-origin`, custom `X-MBR-Edit` header needs no preflight), so both gates pass and `peer_ip` is loopback so no token is required — directly contradicting the claim at server.rs:1754 that this "defeats cross-origin/DNS-rebinding writes even for loopback callers". The optimistic-concurrency `base_hash` is no obstacle: the attacker reads it from `GET /.mbr/raw/{path}` through the same gate.

Escalation beyond note tampering: `do_upload` accepts an attacker-chosen `dir`, and `serve_mbr_assets` prefers on-disk `<repo>/.mbr/...` over compiled-in defaults, so `dir=.mbr/components&name=mbr-components.min.js` yields persistent script execution in every mbr page and in the wry GUI webview.

**Fix:** Add a Host allowlist to `check_edit_access` (better: a tower layer over all routes) — reject unless the `Host` hostname is `localhost`, the configured bind IP, or an explicitly configured `allowed_hosts` entry.

### [MEDIUM] Tag-page output path escapes `--output` on Windows; containment guard is lexical
`src/build.rs:1447`

`sanitize_path_component` (wikilink.rs:98-110) splits only on `/`, so a backslash traversal survives verbatim; `normalize_tag_value` only lowercases and trims. The follow-up guard is lexical — `Path::Components` never collapses `ParentDir`, verified empirically: `Path::new("build/../../evil/index.html").starts_with("build") == true`. On Windows `\` *is* a separator, so frontmatter `tags: [..\..\..\Users\victim\pwned]` produces a real traversal and `render_single_tag_page_sync` does `create_dir_all` + `fs::write` outside `--output`. `build_tag_pages` defaults to `true`; existing regression tests (build_integration.rs:1178, 1217) use only `/`-separated payloads so they cannot catch it. Constrained primitive: the filename is always `index.html` and existing files are skipped, so it is create-only — arbitrary directory creation plus dropping attacker-controlled `index.html` (e.g. into an IIS docroot or a Startup folder). `TagSource::url_source()` (config.rs:155) is a second injection point into the same joins. Same weak guard at build.rs:1512.

**Fix:** Split `sanitize_path_component` on `['/', '\\']` and drop `..`/`.`/drive-prefixed segments. Replace the `starts_with` guard with a component scan rejecting `Component::ParentDir`/`RootDir`/`Prefix` in the joined suffix.

### [MEDIUM] Build writes pages outside `--output` when a repo directory symlinks outside the root
`src/build.rs:955`

`repo::scan_folder` canonicalizes each queued subdirectory (repo.rs:607-616, `follow_links(true)`), so a symlinked directory is re-rooted at its resolved target; `pathdiff::diff_paths` then yields `../…` and `url_path::path_to_url` deliberately preserves `ParentDir` (url_path.rs:46, with a test asserting it). The build joins these onto `output_dir` with no containment check.

```rust
let url_path = info.url_path.trim_start_matches('/');
let output_path = if url_path.is_empty() || url_path == "/" {
    self.output_dir.join("index.html")
} else {
    self.output_dir.join(url_path).join("index.html")
};
```

Reproduced: repo with `work -> /tmp/.../work/docs`, `mbr -b --output /tmp/.../deep/out` wrote `index.html`, `links.json`, and a placed asset to `/tmp/.../deep/work/docs/a/` — outside the requested output dir. The only `starts_with(&self.output_dir)` guards in build.rs (1447, 1512) are on tag pages. Also affects `links.json` (build.rs:773-779) and `place_assets` (build.rs:1535).

**Fix:** Reject or clamp any scanned file whose repo-relative path contains a `ParentDir` component in `repo::scan_folder`, and add a component-wise containment assertion (not `Path::starts_with`) before every `fs::write`/`place_asset` in build.rs.

### [MEDIUM] `static_folder` from repo-supplied `.mbr/config.toml` is unvalidated and can point outside the root
`src/path_resolver.rs:303`

`find_in_static_folder` joins the configured `static_folder` onto the canonical base and uses the *result* as the containment root, so escaping the root makes the escaped directory the root. `Config::validate` (config.rs:645-681) checks port, sidebar_max_items, graph_depth, build_concurrency and the edit token — never `static_folder` — and `.mbr/config.toml` is merged from the repository itself (config.rs:627). Verified by replicating the function: `static_folder="../.."` + request `…/secret/id_rsa` resolves and passes; `static_folder="/etc"` + `hosts` serves `/etc/hosts`. The 4b fallback at path_resolver.rs:246 runs even when `safe_join` rejects the path, so out-of-repo request paths reach it. Symlink escape is separately blocked, making this currently the only way a hostile repo makes the server read outside the root.

**Fix:** Validate `static_folder` in `Config::validate`: reject absolute paths and any value whose canonicalized form leaves `root_dir`. If upward static folders are genuinely needed (server.rs:345 mentions `../static`), gate them behind a CLI flag rather than repo-supplied config.

### [MEDIUM] `.mbr/config.toml` is served over HTTP and copied into static builds, leaking `edit_token_hash`
`src/server.rs:2762`

`serve_mbr_assets` has no extension or basename allowlist — `safe_join_asset` checks only `..`, canonical containment, and `is_file()`. Reproduced against the real binary: `GET /.mbr/config.toml` returned 200 with the full `edit_token_hash`, no credential required; `mbr -b` wrote the same hash into `build/.mbr/config.toml`, which `build.rs:1672-1678` deliberately makes GitHub-Pages-servable via `.nojekyll`. This violates the documented invariant at configuration.md:212 ("Never sent to the frontend"). Severity is medium rather than high because what leaks is an Argon2id PHC hash with a random salt — dangerous only if the user typed a human password at the `--generate-edit-token` prompt (main.rs:69-79 accepts it verbatim) rather than accepting the 32-byte random default, and no CORS headers exist so a drive-by page cannot read it from a default loopback instance.

**Fix:** Return 404 for `config.toml` (or restrict `serve_mbr_assets` to an asset extension allowlist), skip it in `copy_dir_recursive`, and ideally move `edit_token_hash` out of the web-served folder entirely.

### [MEDIUM] Upload endpoint can write executable site assets into `.mbr/`, hijacking every page
`src/server.rs:2209`

`sanitize_upload_name` rejects only path separators and *markdown* extensions — no allowlist — and `resolve_new_target_path` treats `.mbr` as an ordinary `Component::Normal`. Reproduced end-to-end: `POST /.mbr/upload?dir=.mbr&name=index.html` with `X-MBR-Edit: 1` returned 200, and with no restart `GET /notes/note/` rendered the attacker's Tera template (the watcher at server.rs:1024-1044 hot-reloads on `.html` changes under `.mbr`). `dir=.mbr/components&name=mbr-components.min.js` likewise shadowed the compiled-in bundle. Existing tests (server_integration.rs:4659-4686) cover only `..` traversal and markdown extensions.

**Fix:** Reject uploads whose resolved target lies inside `.mbr/` (or the configured `template_folder`), and restrict extensions to a media allowlist — `.html`/`.js`/`.css` must never be creatable by the asset uploader.

### [MEDIUM] Live-reload WebSocket accepts cross-origin handshakes and streams absolute repo paths
`src/server.rs:1418`

*(Merged: two findings, same root cause — no origin check on the WS upgrade plus an absolute path in the payload.)*

```rust
pub async fn websocket_handler(
    ws: WebSocketUpgrade,
    State(config): State<ServerState>,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| Self::handle_websocket(socket, config))
}
```

No `HeaderMap` is extracted and the route (server.rs:1282) is unconditional with only Compression/Trace layers; axum's `WebSocketUpgrade` validates no `Origin` (zero hits for "origin" in its ws.rs). WebSocket handshakes are exempt from SOP, so any page the victim visits can connect to `ws://127.0.0.1:5200/.mbr/ws/changes` and receive `FileChangeEvent`, whose `path` is documented as the absolute filesystem path (watcher.rs:24-26) — leaking OS username, home layout, and private note filenames in real time. The project already applies exactly this defense to the edit routes via `is_same_origin` (server.rs:1813, sole caller server.rs:1782), so this is an omission. Bounded impact: `subscribe()` runs after upgrade so there is no backlog replay (attacker sees only concurrent edits), the payload is metadata only, and inbound frames other than Close/Ping are ignored.

**Fix:** Extract `HeaderMap` in `websocket_handler` and reject non-same-origin upgrades with 403 before `ws.on_upgrade`. Independently, drop the absolute `path` field from the broadcast payload — `mbr-live-reload.ts` reads only `relative_path` (verified: zero `.path` references).

### [MEDIUM] Third-party GitHub Action pinned to mutable `@main` alongside cache-push secret
`.github/workflows/ci.yml:27`

`DeterminateSystems/determinate-nix-action@main` appears 10 times (ci.yml 27/55/108/141/174/212/307/355, codeql.yml:70, docs.yml:28), and in 9 of those jobs the very next step passes `secrets.CACHIX_AUTH_TOKEN` to `cachix/cachix-action@v17`. A Nix *installer* action runs with effective root on the runner, so it can trivially intercept the later step. `scripts/cachix-push.sh` uses the auth token alone (server-side signing) and `flake.nix:12` lists `zmre.cachix.org-1:…` under `extra-trusted-public-keys`, so a stolen token yields NARs signed by a key this flake auto-trusts — and `release.yml:189-196` opts into that substituter before `nix build .#release`, whose output is published via `softprops/action-gh-release`. `docs.yml` additionally grants `pages: write` / `id-token: write` to a job running the unpinned action. Note `dependabot.yml` configures github-actions updates but cannot pin a branch ref; also note the other actions are on mutable *major-version tags*, so this is the worst instance of a repo-wide pattern rather than a unique weak link.

**Fix:** Pin to a full 40-char commit SHA with a version comment in all 10 occurrences. Drop `authToken` from jobs that only need to pull from the cache.

### [LOW] Unbounded parallel oembed fan-out lets one markdown file flood an arbitrary host
`src/markdown.rs:1068`

`prefetch_oembed_urls` builds one future per distinct bare URL and drives them all with `futures::future::join_all` — no semaphore, no cap on `uncached.len()`, and `collect_bare_urls` has no limit either. Server/GUI mode enables oembed by default (500 ms), so a page with tens of thousands of distinct bare URLs opens that many outbound requests at once, pressuring the fd limit and making the user an unwitting traffic source. The cache is bounded at 10k entries and 2 MB, so the flood repeats on reload. Mitigating: `check_url_target` routes through `lookup_host` on tokio's blocking pool and each request has a 500 ms timeout, so the ramp self-limits; `is_public_ip` already blocks private/loopback targets, so this is outbound amplification only, not SSRF. The codebase already uses bounded concurrency elsewhere (`PDFIUM_SEMAPHORE`, `inbound_grep_semaphore`).

**Fix:** Replace `join_all` with `futures::stream::iter(...).buffer_unordered(N)` and cap URLs fetched per document (e.g. first 100, rest as plain links).

### [LOW] Stored XSS: raw frontmatter JSON injected into an inline `<script>` with `| safe`
`templates/_head_markdown.html:33`

```html
<script>
  // Frontmatter available to page scripts (excludes rendered markdown for efficiency)
  window.frontmatter = {{ frontmatter_json | safe }};
```

`serde_json::to_string` (templates.rs:161) does not escape `<`, `>` or `/`, so a frontmatter value containing `</script>` terminates the block — reproduced in a standalone serde_json + tera harness. Severity is LOW, not critical, because it crosses no trust boundary: `markdown.rs:39` uses `Options::all()` and `html.rs:245` writes `Html | InlineHtml` events verbatim, so any author who can set frontmatter can already run `<script>` from the body (a test comment at markdown.rs:2563 acknowledges this). The concrete cost is a functional bug — a benign title like `Avoiding </script> in templates` silently kills the block, leaving `window.frontmatter`, `window.headings` and `window.extendedMeta` undefined and `<mbr-info>` empty — plus a defense-in-depth hole that would matter the moment a CSP or body sanitizer is added. Note no CSP exists anywhere.

**Fix:** In `render_markdown_with_tera` post-process the serialized string with `.replace('<', "\\u003c").replace('>', "\\u003e").replace('&', "\\u0026")`, or move the payload into `<script type="application/json">` and `JSON.parse(el.textContent)`. Same for `window.headings` / `window.extendedMeta`.

### [LOW] XSS: frontmatter `title` written into `<title>` with `| safe`, bypassing Tera autoescape
`templates/_head_markdown.html:27`

```jinja
<title>{{title_prefix | default(value="")}}{{title | default(value="") | safe}}{{title_suffix | default(value="")}}</title>
```

Verified empirically against tera 1.20.1 (autoescape is on for `.html`; `is_marked_safe()` triggers because `safe` is the last filter): a title of `</title><img src=x onerror=alert(1)>` renders the `<img>` as live markup, while the neighboring `<meta name="title">` on line 28 correctly escapes. Same LOW rationale as above — the markdown body is an unsanitized HTML passthrough, so the same author already has script execution. What survives is a real correctness bug: a benign title like `Tips & Tricks` or `R&D <Q3> Plan` produces malformed markup and a mangled tab title.

**Fix:** Drop `| safe` — `{{ title | default(value="") }}`. While there, note line 28's explicit `| escape` double-escapes (`&amp;lt;`) since autoescape runs after it; drop that too.

### [LOW] Unescaped `file_path` / `prev_page.url` inside JS string literals in the head script
`templates/_head_markdown.html:46`

```jinja
filePath: "{{ file_path | default(value='') | safe }}",
{% if prev_page %}prevPage: { url: "{{ prev_page.url | safe }}", ... },{% endif %}
```

These are raw on-disk relative paths (`to_string_lossy()`, server.rs:4534 / build.rs:930 → page_context.rs:325), and `"` and `\` are legal filename characters. Verified in a Tera harness with the same autoescape setup: a file named `He said "hi".md` renders `filePath: "docs/He said "hi".md",` which bun rejects as a syntax error — killing the whole inline block so `window.frontmatter`/`headings`/`extendedMeta` never define and `<mbr-info>` renders empty. Two corrections to the reported scenario: the example filename containing `/` cannot exist, and the injection point is inside an object literal, so a working payload must be object-shaped (e.g. `x", evil: alert(1), y: ".md`, which I confirmed executes). Rated LOW for the same trust-boundary reason as the two findings above; the correctness half is what justifies the fix.

**Fix:** Serialize these values server-side as a single JSON blob escaped for script context (same fix as the `frontmatter_json` finding) rather than hand-built JS string literals. Removing `| safe` alone is wrong — HTML entities are not decoded inside `<script>`.

### [LOW] `javascript:` link destinations are rendered as clickable hrefs in the info panel and fuzzy nav
`components/src/mbr-info.ts:658`

`link.to` is the raw markdown destination from `links.json`; `is_internal_link()` (link_index.rs:79) explicitly classifies `javascript:`/`data:` as external, `link_transform.rs:113-116` passes them through unchanged, and `escape_href` (pulldown-cmark-escape) percent-escapes but treats `:` as safe and filters no schemes. Both `<mbr-info>` and `<mbr-fuzzy-nav>` (mbr-fuzzy-nav.ts:709) bind that value straight into `href`; Lit does not sanitize attribute bindings and `setSanitizer` is never called. LOW because raw `<script>` in the markdown body already executes without a click (html.rs:245), so this adds no capability — and both cited sinks use `target="_blank"`, where browsers drop `javascript:` navigations anyway; the genuinely clickable path is the rendered body `<a>` from html.rs:498. Note `test_validate_links_skips_external` (build.rs:2752) deliberately asserts `javascript:void(0)` is not flagged.

**Fix:** Add a scheme allowlist at both ends — neutralize `javascript:`/`data:`/`vbscript:` destinations in `html.rs`'s link writer, and route `link.to` through a `safeHref()` helper in `mbr-info.ts` and `mbr-fuzzy-nav.ts`.

### [LOW] Vendored CDN JavaScript fetched with no integrity check and no HTTP error detection
`scripts/update-assets.sh:80`

⚠️ contested — one verifier disagreed: the script is a manual maintainer-only tool never invoked by CI, `build.rs`, or `flake.nix`, so no in-threat-model actor can reach it, and `curl -sL` still does full TLS verification (no `-k`), which refutes the MITM leg.

All five `curl -sL … -o` calls (lines 80, 83, 88, 94, 117) omit `-f`, so `set -euo pipefail` and `|| error` never fire on HTTP errors. Empirically confirmed: `curl -sL "https://cdn.jsdelivr.net/npm/mermaid@11.99.0/dist/mermaid.min.js" -o …` exits 0 and writes a 52-byte error body, after which the script prints "downloaded successfully!". There is no checksum manifest anywhere in the repo, and the results are `include_bytes!`-embedded (server.rs:5298, embedded_hljs.rs:7-35) with zero validation. Note the proposed self-written checksum file provides no integrity against a compromised fetch (it hashes whatever the CDN just served) — only an out-of-band pinned hash would.

**Fix:** Add `-f --proto '=https' --tlsv1.2` to every curl call so HTTP errors fail loudly. For real integrity, pull these from the pinned npm tree in `components/` (already hash-verified by `npmDepsHash`) or add fixed-output Nix derivations with upstream-published hashes.

### [LOW] Release derivation silently strips code signatures from macOS tarball artifacts
`flake.nix:686`

```nix
/usr/bin/codesign --force --sign - staging/MBR.app 2>/dev/null || \
  /usr/bin/codesign --remove-signature staging/MBR.app 2>/dev/null || true
```

Same pattern at lines 677-680 (libpdfium.dylib) and 681-685 (MBRPreview.appex with `--entitlements`), with no `codesign --verify` anywhere on this path — the tarball goes straight to `$out/mbr-${archString}.tar.gz` and is uploaded as a GitHub Release asset via the documented `tar xzf … && cp -r MBR.app /Applications/` install path. The release derivation is *more* likely than `packages.mbr` to hit this, since it copies from the read-only store and chmods only `Contents/MacOS`, leaving `Frameworks/`, `PlugIns/` and `_CodeSignature/` unwritable. Signatures here are ad-hoc (`--sign -`) so no authenticity is lost and SHA256SUMS covers integrity separately — the consequence is a broken app (`Killed: 9` on Apple Silicon, or a stale seal-broken signature) shipped by a green build. `scripts/make-universal-dmg.sh:118` already does `codesign --verify --deep --strict` under `set -euo pipefail`, so only the tarball path has this gap.

**Fix:** Drop the `--remove-signature`/`|| true` fallbacks, let the derivation fail, and add `/usr/bin/codesign --verify --deep --strict staging/MBR.app` before the `tar`. If signing truly cannot work in the sandbox, move it into the release workflow which already runs on a real macOS runner.

### [LOW] crates.io publish ships unreviewable minified JS with `--allow-dirty --no-verify`
`.github/workflows/release.yml:435`

`publish-crate` is checkout → `download-artifact` into `templates/components-js/` → `cargo publish --allow-dirty --no-verify`, with no digest check (unlike the `test` job's `test -f` guards). `.gitignore:15` excludes `templates/components-js`, `Cargo.toml:18-25` omits `components/` from `include`, and all four vite configs set `sourcemap: false` with terser — so `cargo install` users get four minified bundles with no corresponding source in the tarball and no git object to diff. `--allow-dirty` is exactly the guard being suppressed (verified: without it cargo errors on the gitignored-but-included file). LOW because the bytes provably come from the same workflow run's `nix build .#mbr-components` with a pinned `npmDepsHash`, so provenance is unpinned rather than absent; the failure scenario is conditional on future edits to the workflow. Also note dropping `--no-verify` does not address substitution — `include_bytes!` accepts any bytes.

**Fix:** Record each bundle's SHA-256 in `build-components`, re-verify after `download-artifact` and before `cargo publish`, and add `components/**` to `Cargo.toml`'s `include` so the crate carries the source for the JS it embeds.

### [LOW] Media embed URLs interpolated into HTML attributes without escaping
`src/media.rs:177`

`pdf_to_html`, `Audio::to_html` (audio.rs:53-67) and `vid.rs:159-186` splice the link destination straight into double- and single-quoted attributes, and the result is emitted as `Event::Html` which `html.rs:245` writes verbatim. Confirmed against pulldown-cmark 0.13.4 with `Options::all()` that `![x](a"onerror="alert(1)"b.mp3)` yields `dest_url = a"onerror="alert(1)"b.mp3` (pulldown-cmark's own writer escapes this; mbr's custom path does not), and `link_transform.rs` does no escaping. LOW rather than high: the actor is whoever authors the .md file, who already has unrestricted script execution via the raw-HTML passthrough — `tests/server_integration.rs:3706-3717` depends on that passthrough. The remote-content path is clean (`oembed.rs:520/534/541` already escape OpenGraph fields). What remains is real output corruption: `![](Q&A-report.pdf)` emits a bare `&` in `data-pdf-url`, and a filename with `"` breaks the tag.

**Fix:** Wrap every interpolation in `html_escape::encode_double_quoted_attribute` (and `encode_quoted_attribute` for the single-quoted `<source src='…'>` in vid.rs), as `oembed.rs::html()` already does. Escape `caption` too, and add a regression test asserting `&quot;` appears and `onerror=` does not.

---

## Bugs & Correctness (34 findings)

### [HIGH] Panic (process abort) on comment-only YAML frontmatter
`src/markdown.rs:1189`

`YamlLoader::load_from_str(...).map(|ys| ys[0].clone()).ok()` indexes element 0 inside the `map` closure, so `.ok()` cannot catch it — and yaml-rust2 0.11 returns `Ok(vec![])` for a metadata block whose body is only comments. Verified end-to-end: a file containing `---\n# tags: [draft]\n---` makes `mbr -b` exit 101 with `index out of bounds: the len is 0 but the index is 0`, and in server mode the rayon scan worker dies so `Repo::wait_for_scan()` blocks forever (site.json, browse, and search hang indefinitely). Release builds set `panic = 'abort'` (Cargo.toml:159), so the process SIGABRTs. One ordinary user file — commented-out frontmatter — kills the whole repo.

```rust
let metadata_parsed = YamlLoader::load_from_str(text).map(|ys| ys[0].clone()).ok();
```

**Fix:** Use the non-panicking form already present at src/markdown.rs:1427 at both call sites (src/markdown.rs:106 and :1189): `YamlLoader::load_from_str(text).ok().and_then(|docs| docs.into_iter().next())`.

### [MEDIUM] vid shortcode, bare-URL oembed, and `[-] ` transforms fire inside code blocks
`src/markdown.rs:1438`

`process_event` tracks `state.in_code_block` and consults it for word counting at line 1410, but the three text-rewriting branches (1438, 1445, 1461) have no code-block guard. A fence containing only `{{ vid(path="demo.mp4") }}` renders as a raw `<figure><video>` inside `<pre><code>`; a fence containing only a URL becomes an oembed card; a fence starting with `[-] ` becomes a disabled checkbox. `collect_bare_urls` (src/markdown.rs:985) has the same gap, so mbr issues a real outbound HTTP GET for URLs that appear only in code samples — confirmed by log output (`oembed fetch start: https://example.com/x`). This repo's own docs (CLAUDE.md:419, docs/markdown/media.md) trigger it.

**Fix:** Insert a code-block arm ahead of the rewrite chain — `if state.in_metadata { … } else if state.in_code_block { (event, state) } else if …` (do **not** fold it into the metadata arm, which would run YamlLoader on code text) — and track `Tag::CodeBlock`/`TagEnd::CodeBlock` in `collect_bare_urls` the same way `in_link` is tracked.

*Merged:* the "Bare URL inside a fenced code block is replaced with oembed HTML and triggers a network fetch" finding (src/markdown.rs:1445) is the same root cause and is covered by this fix.

### [MEDIUM] transform_wikilinks rewrites `[[Tags:x]]` inside code fences and inline code
`src/markdown.rs:622`

Tag wikilinks are substituted by raw string search-and-replace over the file text *before* pulldown-cmark parses it (src/wikilink.rs:179-213 is a plain `find("[[")` scanner with zero code awareness). `tag_sources` defaults to `["tags"]` (src/config.rs:185), so this always runs. Reproduced with the built binary: a ```markdown fence, an inline code span, and an indented block all render `[[Tags:rust]]` as `[rust](/tags/rust/)`. The project's own docs/reference/tags.md:80 and :95 are corrupted — the "source" example renders identically to the "Renders as:" line.

**Fix:** Move tag-wikilink resolution into the event stream (handle it in `process_event` for `Event::Text` when `!state.in_code_block`, alongside the existing `parse_tag_link` handling for `Tag::Link`), or make `transform_wikilinks` skip fenced/indented blocks and backtick spans.

### [MEDIUM] Section wrapping emits `</section>` for thematic breaks nested in blockquotes/lists
`src/html.rs:255`

The `Rule` arm unconditionally writes `</section>\n<hr />\n<section>` with no container-depth check; `HtmlWriter` has no depth tracking at all. `enable_sections` is always true on the render path (`finalize_render` → `mbr_with_section_attrs`). Verified by running the real `html.rs` against pulldown-cmark 0.13: `> before\n>\n> ---\n>\n> after` yields `<section><blockquote><p>before</p></section><hr /><section><p>after</p></blockquote></section>`; parsing that with happy-dom shows the stray `</section>` pops the blockquote, "after" escapes the quote, and the trailing `</blockquote>` is discarded. `- a\n\n  ---\n\n- b` tears open the `<ul>` the same way.

**Fix:** Add a container-depth counter to `HtmlWriter` (increment on `Start(BlockQuote|List|Item|TableCell|FootnoteDefinition)`, decrement on matching ends) and only use the section-splitting form at depth 0; otherwise emit a plain `<hr />`.

### [MEDIUM] UTF-8 BOM silently disables frontmatter and renders it as a visible heading
`src/markdown.rs:1174`

File contents go to pulldown-cmark verbatim with no BOM stripping. Verified against pinned pulldown-cmark 0.13.4: `\u{FEFF}---\ntitle: …\n---` emits no `MetadataBlock` event at all, so `extract_metadata_from_file`'s handler never fires and title/tags/type/relationships are silently dropped from site.json, search, and the relationship graph — while the page visibly renders `<h2>—title: My Page</h2>`. `rg -i -e bom -e feff src/` finds handling only in `pdf_metadata.rs` (a different path). Windows CI was just added (995634f), making BOM-prefixed files realistic.

```rust
let markdown_input = String::from_utf8_lossy(&buffer);
let parser = MDParser::new_ext(&markdown_input, Options::ENABLE_YAML_STYLE_METADATA_BLOCKS);
```

**Fix:** Strip a leading U+FEFF in one shared helper (`s.strip_prefix('\u{feff}').unwrap_or(s)`) and apply it in `extract_metadata_from_file` (:1174), `render_with_cache` (:610), `render_sync` (:891), and `parse` (:81).

### [MEDIUM] Anchor-only links (`[Top](#top)`) resolve to the parent page, creating phantom backlinks
`src/link_index.rs:206`

The renderer splits `#anchor` off before storing, so an anchor-only link becomes `OutboundLink { to: "", anchor: Some("#top") }` and the `!link.to.starts_with('#')` guard never fires. `resolve_relative_url(base, "", …)` then returns the parent directory. Reproduced with the built binary: `build/docs/links.json` gains `{"from":"/docs/guide/","text":"Overview","anchor":"#overview"}` and the root `links.json` gains one from `/README/`; in server mode `GET /docs/guide/links.json` reports an outbound link to `/docs/` that the author never wrote. Dedup by `to` limits it to one phantom per page. Note `src/page_errors.rs:102` already special-cases exactly this shape.

**Fix:** In `resolve_outbound_links` add `!link.to.is_empty()` to the guard, and skip empty-`to` links in `build.rs::write_link_files` before inverting into `inbound_index`.

### [MEDIUM] transform_link mangles any markdown link whose stem merely ends with "index"
`src/link_transform.rs:161`

The index-collapse branch tests `base_path.ends_with(index_stem)` instead of checking the final path segment. Executing the real (dependency-free) module confirms: `site-index.md -> ../site-/`, `myindex.md -> ../my/`, `reindex.md -> ../re/`. The page is served at `/docs/site-index/`, so every such link 404s in server and build mode. `src/repo.rs:1374-1377` deliberately guards against this exact substring mistake (with a regression test at :1627), so the two implementations disagree. `OutboundLink.to` keeps the untransformed dest, so backlinks look fine while the rendered href is dead. Worst variant: with a directory `docs/sub/` present, `subindex.md -> ../sub/` silently navigates to a *different real page*, which the link checker will not flag.

**Fix:** Mirror `repo::build_markdown_url_path`: `let is_index_target = base_path == index_stem || base_path.ends_with(&format!("/{}", index_stem));` and branch on that.

### [MEDIUM] Server-mode backlinks silently miss every wikilink resolved by title/alias/global fallback
`src/link_grep.rs:405`

`find_inbound_links` derives its Aho-Corasick and regex patterns solely from the target's URL path, so it only sees `[[…]]` whose literal text is a path form of the target. Body wikilinks the renderer resolves via `WikilinkIndex` (title, alias, or global stem fallback) are invisible. Reproduced: repo with `people/pw.md` (`title: Patrick Walsh`, `aliases: [PW]`) and `notes/family.md` containing `See [[Patrick Walsh]] and also [[PW]].` — `mbr -b` writes `{"inbound":[{"from":"/notes/family/","text":"Patrick Walsh"}]}` while `GET /people/pw/links.json` returns `{"inbound":[]}`. The gap is wider than a path mismatch: from another folder even a bare stem `[[pw]]` is missed.

**Fix:** Add the target's title, aliases, and filename stem (from `WikilinkIndex`) to the pattern set and `build_wiki_extraction_regex`; or drop the grep in server mode and invert `LinkCache` outbound entries the way build mode does.

### [MEDIUM] Case-insensitive wikilink resolution emits the author's casing, dropping backlinks in static builds
`src/wikilink_index.rs:128`

`resolve_wikilink` matches the current-folder stem case-insensitively but returns `None` ("no rewrite needed") on a hit, so the raw differently-cased text flows through `transform_link`. Reproduced with the built binary on `notes/Japan.md` + `notes/other.md` containing `See [[japan]]`: the build emits `href="../japan"`, prints `Broken links detected (1 total)`, and `notes/Japan/links.json` is `{"inbound":[],"outbound":[]}` — while server mode's case-insensitive grep *does* report the backlink. Server and build disagree, and the href 404s on any case-sensitive filesystem.

**Fix:** On a same-folder `by_dir_stem` hit, compare the matched note's URL last segment against the raw wikilink text and return `Some(url)` when the casing differs, instead of unconditionally returning `None`.

### [MEDIUM] Live-reload WebSocket silently stops forwarding events after broadcast lag
`src/server.rs:1462`

The handler consumes the file-change broadcast with a refutable `Ok(...)` pattern in `tokio::select!`. Confirmed against vendored tokio 1.53.1 (`src/macros/select.rs:719-729`): a pattern mismatch sets `disabled |= mask` and the branch stays disabled for that select! invocation. On `RecvError::Lagged` (capacity 100, `src/watcher.rs:20`, one event per path with no coalescing), the rx branch dies and the task parks forever in `receiver.next()` — the browser client never sends anything, the server never pings, so the socket stays open and no reconnect fires. Live reload is dead for that tab until a manual reload, with no log line.

```rust
tokio::select! {
    Ok(change_event) = rx.recv() => {
```

**Fix:** Bind the whole result and match explicitly, as the two sibling subscribers already do at src/server.rs:1065 and :1078: `result = rx.recv() => match result { Ok(e) => …, Err(Lagged(n)) => { warn!(…); continue }, Err(Closed) => break }`.

### [MEDIUM] find_root_dir aborts the marker search entirely when any marker matches `$HOME`
`src/config.rs:597`

On a `$HOME` match the code `break`s out of the whole `DIR_MARKERS` loop instead of skipping to the next marker, contradicting the function's own doc comment ("Skips matches at `$HOME`"). Verified by compiling config.rs:562-616 verbatim: with `~/.git` present and an Obsidian vault at `~/notes` (marked only by `~/notes/.obsidian`), `mbr -s ~/notes/projects/plan.md` resolves the root to `~/notes/projects` instead of `~/notes`. Sidebar/site.json cover only that subfolder, breadcrumbs are wrong, repo-wide wikilinks stop resolving, and `~/notes/static` is not found. Removing `~/.git` makes it correct, proving the abort.

**Fix:** Replace `break` with `continue` in both the DIR and FILE marker loops.

### [MEDIUM] Build leaves orphaned `build.old.<pid>` directories that the repo scanner then indexes
`src/build.rs:513`

`prepare_output_dir` renames the previous output to `<output>.old.<pid>` and deletes it on a detached, never-joined thread; the stale check only matches the *current* PID, so leftovers from an interrupted run are never swept (no signal handling exists). `build.old.<pid>` is not in `ignore_dirs` and `should_ignore` only skips dot-names/exact matches/globs, and the scan runs (build.rs:380) before `prepare_output_dir` (build.rs:399). Reproduced: with a leftover `build.old.88888/`, media.json and site.json list `/build.old.88888/images/pic.png` *and* `/images/pic.png` — the image indexed twice — and `place_assets` recreates the whole orphan inside the new output.

**Fix:** Use a hidden, PID-independent name (`.<name>.mbr-old`) so the scanner ignores it, sweep any pre-existing `*.mbr-old` siblings before renaming, and join the delete thread (or re-check and delete at the end of `build()`).

### [MEDIUM] Cancelled HLS/metadata request leaves a permanent `InProgress` entry, wedging that key forever
`src/server.rs:4270`

The single-flight winner claims the cache slot via `start_generation`, then awaits `spawn_blocking`. Client disconnect drops the handler future at that await, so `complete_generation`/`fail_generation` never runs. `InProgress` has no TTL (only `Failed` does, `video_transcode_cache.rs:25`) and `evict_until_freed` refuses to evict non-`Complete` entries, so the slot is unrecoverable: every later request for that segment blocks the full 60 s `HLS_WAIT_TIMEOUT` then 404s, until restart. Cancellation was confirmed empirically against the pinned axum 0.8.9/hyper 1.11.0 (`h1_half_close=false` errors the connection on read EOF and drops the in-flight service future). The `Err(JoinError)` arms at :4293 also `return None` without `fail_generation`, so a panicking transcode wedges the key with no cancellation involved. `try_serve_video_metadata` has the identical defect at src/server.rs:3230 — and unlike HLS, that path is default-on.

**Fix:** Wrap the claim in a guard struct whose `Drop` calls `fail_generation` / `metadata_inflight.remove(&key)` + `notify_waiters()`, so every exit path releases; add a test that drops the producing future mid-flight and asserts the next `start_generation` returns `Started`.

### [MEDIUM] Global `?` shortcut preventDefaults inside inputs — question mark cannot be typed anywhere
`components/src/mbr-keys.ts:244`

The `?` help-overlay branch is the *first* thing in `_handleKeydown` and calls `e.preventDefault()` unconditionally, 29 lines ahead of the `isInputTarget(e)` guard at :273. `<mbr-keys>` is on every page via templates/_footer.html:1 and listens on `document`. Nothing intercepts `?` first: mbr-search's input handler never stopPropagations it, and the Crepe/link-autocomplete handlers stop only Arrow/Tab/Enter/Escape. Typing `Why?` in the editor saves `Why` and pops the Keyboard Shortcuts modal over the editor; same in the search box and every rename/picker input.

```ts
if (e.key === '?' && !e.ctrlKey && !e.metaKey) {
  e.preventDefault();
```

**Fix:** Gate the branch on the existing helper: `if (e.key === '?' && !e.ctrlKey && !e.metaKey && !isInputTarget(e))`. Leave the Escape-closes-help branch unguarded.

### [MEDIUM] mbr-nav compares percent-encoded pathname against decoded `url_path` and emits unresolved hrefs
`components/src/mbr-nav.ts:64`

`_computeNavigation` reads raw `window.location.pathname` and matches it against site.json `url_path`, which Rust serializes decoded (`src/url_path.rs::path_to_url` copies names verbatim; no `utf8_percent_encode` on that field). For `/Walsh/Patrick%20Joseph%20Walsh%20b.1977-10-01/` the `findIndex` returns -1, the function returns early, and both buttons stay disabled — which also kills the `H`/`L` shortcuts (mbr-keys.ts:427-451 clicks `a.nav-button.prev/next`). The same mismatch fires for every page of a static build deployed under a subdirectory, where `basePath` prefixes the pathname. `shared.ts:141` `getCanonicalPath()` exists for exactly this and shared.test.ts:67-72 pins this exact path shape; mbr-nav is the only component that uses neither it nor `resolveUrl()`. Lines 107/118 additionally emit raw `href="${file.url_path}"`, which would 404 under a subdirectory prefix if a match ever did occur.

**Fix:** Import both helpers from `./shared.js`: use `getCanonicalPath()` at line 64 and wrap the hrefs as `resolveUrl(this._prevFile.url_path)` / `resolveUrl(this._nextFile.url_path)` at lines 107 and 118.

*Merged:* "mbr-nav emits raw absolute url_path hrefs, ignoring resolveUrl/getCanonicalPath" is the same root cause; both halves are fixed together.

### [MEDIUM] mbr-browse stores percent-encoded pathnames, so Recent silently drops entries and the current page is never highlighted
`components/src/mbr-browse.ts:121`

`connectedCallback` writes raw `window.location.pathname` into `localStorage['mbr_recent_files']`, but `_getBlendedRecent` (line 516) resolves those strings with `this._allFiles.find(f => f.url_path === url)` against decoded `url_path` values and `.filter(f => f !== undefined)` drops the misses. Visiting `/My Notes/Idea/` stores `/My%20Notes/Idea/`, which never resolves — the page never appears in Recent no matter how often it is visited. `_isCurrentPath` (line 632) has the identical mismatch, so the current row is never highlighted, and line 149 feeds the same encoded value to `_autoExpandCurrentPath`. In a static build under a subdirectory, every stored path carries the deployment prefix. `<mbr-browse>` is the default sidebar (`default_sidebar_style()` = "panel", src/config.rs:65); `mbr-browse-single.ts:152` already uses `getCanonicalPath()`.

**Fix:** Import `getCanonicalPath` from `./shared.js` and use it at lines 121 and 632 (transitively fixing 149); optionally decode existing stored values on read.

*Merged:* the two separately-reported instances of this (Recent-files drop and current-path highlight) are one root cause.

### [MEDIUM] mbr-browse auto-expand keys omit the trailing slash the folder tree uses
`components/src/mbr-browse.ts:561`

`_autoExpandCurrentPath` adds `'/docs'`, `'/docs/guide'` to `_expandedFolders`, but `buildFolderTree` (sorting.ts:206, :231) mints every node with `path = '/docs/'`, and `_renderFolderTree` looks up `this._expandedFolders.has(node.path)` at line 899. Verified with a throwaway `bun test` against the real `buildFolderTree`: `expanded.has('/docs/') === false`. Only root survives (via `|| isRoot`), so no level below root is ever auto-expanded and the user drills down manually every time — the exact behaviour the function exists to prevent. `mbr-browse-single.ts:623` gets it right; `_toggleFolder` is called with the slash form, so the two writers use different key shapes.

**Fix:** `this._expandedFolders.add(accumulated + '/');` at line 561, and factor the shared `_autoExpandCurrentPath`/`_toggleFolder`/`_isCurrentPath` trio into one module used by both browsers. Note `_currentSelection` (line 583) is set from the page URL and still won't match a FolderNode, so the selection highlight needs a separate look.

### [MEDIUM] Async render path skips no-network embeds when `oembed_timeout_ms == 0`; sync path does not
`src/markdown.rs:636`

`render_with_cache` short-circuits the entire oembed prefetch at timeout 0, but `PageInfo::new_from_url` (src/oembed.rs:311-325) returns `local_embed(url)` *before* its own `timeout_ms == 0` early return — so the guard suppresses only the no-network embeds (YouTube, Giphy, gist, bare media). `render_sync` unconditionally calls `collect_local_embeds` at :914 and produces them. `src/quicklook.rs:166` hardcodes 0 into the async path, so QuickLook previews never embed; docs/markdown/media.md:278 and docs/reference/configuration.md:318 explicitly promise these still work at 0. Commit 12f317e rewrote the sync branch and left the async one behind.

**Fix:** Mirror `render_sync`: always call `collect_local_embeds`, merging cached network results only when `oembed_timeout_ms > 0` (a blind unconditional `prefetch_oembed_urls` would pollute the cache with empty `PageInfo`s). Better, extract the shared pipeline body into one function parameterized by an embed-source closure.

### [MEDIUM] Four divergent copies of the external-URL scheme predicate flag ftp/magnet/sms links as broken
`src/link_index.rs:72`

The "is this URL external" check exists four times with four different scheme lists (link_index.rs:72, link_transform.rs:203 + :113, build.rs:2110, page_errors.rs:73 — plus a fifth in wikilink.rs:132). `is_internal_link` lacks `ftp://`/`magnet:`/`sms:`, so `OutboundLink.internal = true`. `page_errors::validate_internal_links` never calls its own `is_external_url` (grep shows it is used only by `validate_media_references`), so the link reaches `resolve_request_path`, returns `NotFound`, and the page-errors panel shows a false `BrokenInternalLink`. Build mode joins `ftp://example.com/data.zip` onto the source dir and reports a broken link; `--fail-on-broken-links` then exits non-zero on a valid link. `magnet:` is additionally rewritten to `../magnet:?…`, corrupting an otherwise valid href.

**Fix:** Collapse all copies into one `pub fn is_external_url(url: &str) -> bool` in a shared module (ideally "has a URL scheme other than the site-relative ones" rather than an enumerated allowlist) and call it from link_index, link_transform, page_errors and `build.rs::validate_links`. Add a regression test covering `ftp://`, `ftps://`, `magnet:`, `sms:`, `callto:`, `blob:`.

### [MEDIUM] link_grep::compute_url_path is a fourth path→URL implementation that ignores `index_file`
`src/link_grep.rs:713`

`compute_url_path` takes no `index_file` parameter, so `docs/index.md` becomes `/docs/index/` instead of the canonical `/docs/` produced by `repo::build_markdown_url_path`. That non-canonical string is stored as `InboundLink.from` (link_grep.rs:609/643/667) and used for the self-link skip at :519. Consequences: the backlink href in mbr-info.ts:697 points at a URL that `path_resolver.rs:179-217` treats as needing a redirect, `try_serve_links_json` has no `Redirect` arm so the graph BFS gets a 404 and dead-ends the node, and `graph/build.ts:64` keys on `link.from` so one page becomes two disconnected nodes. Build mode uses the canonical value, so the two modes disagree. It also diverges on case: the file filter lowercases the extension but line 733 compares `.md/` case-sensitively, so `NOTES.MD` yields `/NOTES.MD/`.

**Fix:** Canonicalize the stored `from` via `crate::repo::build_markdown_url_path(path, root_dir, index_file)`, threading `index_file` through `find_inbound_links` and link_rewrite.rs:440. Keep the existing index-aware folder computation — `get_folder_url_path("/docs/index/")` = `/docs/` is what relative-link matching needs, so a blind swap would regress it.

### [LOW] Grep-based backlinks count links inside fenced code blocks
`src/link_grep.rs:604`

`find_inbound_links` runs Aho-Corasick + regex over raw file text with no markdown-block awareness (grep for `fence`/`code_block` in the module returns nothing). Confirmed with a temp unit test: a file whose only mention of the target is inside a ```markdown fence, and another whose only mention is inside a backtick span, are both reported as inbound links, while pulldown-cmark's event stream — what build mode inverts — sees none. Repos that document their own linking conventions (mbr's docs/markdown/index.md:25, docs/modes/editing.md:80) accumulate false backlinks in the info panel and mini graph that `mbr -b` does not produce.

**Fix:** Strip fenced/indented code blocks and inline code spans from `content` with a cheap line scan tracking ``` / ~~~ fences before regex extraction, or parse with pulldown-cmark and collect `Tag::Link` destinations to match the renderer exactly.

### [LOW] Inverse relationship derivation mislabels both sides when only one half of an inverse pair is configured
`src/relationships.rs:168`

`predicate_object` looks up the inverse only via the declared type's own `inverse` field and falls back to the declared name; `RelationTypeRegistry::from_types` never registers the reciprocal and `Config::validate()` performs no consistency check. Reproduced by compiling the registry verbatim: with `[{name="employer", inverse="employee"}, {name="spouse", symmetric}]`, `predicate_object("employee")` returns `"employee"`, so B's page lists A under "Employees" instead of "Employers" (mbr-info.ts:569 groups purely by `predicate`). Reciprocal declarations also fail to collapse into one edge, contradicting the dedup promise at docs/markdown/relationships.md:185. Setting `relationship_types` replaces the total defaults wholesale, and docs/markdown/relationships.md:198 tells users to add exactly this kind of half-declared entry.

**Fix:** In `RelationTypeRegistry::from_types`, for every type `t` with `inverse = Some(i)`, auto-register or verify the reciprocal `i.inverse = t.name`, and log a warning when a declared inverse names an undeclared type.

### [LOW] Outbound link cache is never invalidated on file change, so links.json serves stale links
`src/server.rs:1189`

The watcher-driven invalidation task clears only the repo caches and `sibling_nav_cache` — it cannot touch `link_cache`/`inbound_link_cache`, which are constructed afterwards at :1193 and never cloned into that spawn. `LinkCache::get` has no TTL and `invalidate_all()` is called from exactly one place (`do_move`, :2453); `save_markdown_handler` invalidates nothing. The mini-graph BFS fetches `links.json` for *neighbour* pages, and `try_serve_links_json` returns a cache hit with no mtime check, so an externally edited neighbour serves its pre-edit link list indefinitely. The `!outbound_links.is_empty()` guard at :4472 additionally prevents a page that lost all its links from ever overwriting its entry. The comment at src/server.rs:86 claiming the watcher invalidates this cache is false.

**Fix:** Drop both link caches in the debounced invalidation block (as `do_move` already does), and remove the `!outbound_links.is_empty()` guard at :4472 so an emptied page overwrites with an empty list.

### [LOW] `media_populated` flag is set before population runs, so media.json is served without metadata
`src/repo.rs:952`

`populate_media_metadata` uses `media_populated.swap(true)` as an "already started" guard, and there is no later `store(true)` — but `is_media_populated()`/`wait_for_media()` read the same flag as "finished". A `GET /.mbr/media.json` arriving during the ffmpeg/pdfium probing window passes the check at server.rs:1587, skips `wait_for_media()`, and returns entries with no duration/dimensions, contradicting the handler's documented contract. `populate_basic_metadata` does not fill media fields, so pre-population entries genuinely have `None`.

**Fix:** Add a separate `media_population_started: AtomicBool` for the run-once guard and only `media_populated.store(true, Release)` immediately before `notify_media_populated()`. Note `Notify::notify_waiters()` stores no permit, so the waiter must create its `notified()` future *before* re-checking the flag (or switch to `tokio::sync::watch`).

### [LOW] `wait_for_scan` / `wait_for_media` can miss the notification and hang the request forever
`src/repo.rs:937`

Both waiters check the flag and only then `await` `notified()`. Reading vendored tokio 1.53.1 narrows the window: `notified()` snapshots `notify_waiters_calls` at creation, so the race is the few instructions between the flag load (repo.rs:910/934) and the state load inside `notified()` — real but small. There is no timeout on either path and no router timeout layer, and `mark_scan_complete()` fires exactly once, so a miss hangs `/.mbr/site.json` permanently. A larger non-racy path exists: the full-rescan task (server.rs:1165-1181) calls `repo.clear()` (resetting `media_populated`) and returns early if `scan_all()` errors, never reaching `notify_media_populated()` — every subsequent media.json request then blocks forever.

**Fix:** Build and enable the future before re-checking — `let n = self.scan_notify.notified(); tokio::pin!(n); n.as_mut().enable(); if self.is_scan_complete() { return; } n.await;` — matching the pattern already used at src/server.rs:3237 and :3686, and wrap both waits in `tokio::time::timeout` as those call sites do.

### [LOW] HLS URL round-trip hardcodes `.mp4`, so non-mp4 videos can serve another file's transcode
`src/video_transcode.rs:219`

`Vid::to_html` strips whatever extension is present and emits `{base}-720p.m3u8` for all of mp4/mov/mkv/avi/m4v/ogv, but `find_original_video_path` reconstructs the source as `format!("{base}.mp4")` and the server does no extension probing. With `videos/interview.mov` and `videos/interview.mp4` both present, a `.mov` embed is served the `.mp4`'s transcode (the HLS cache key is the resolved `.mp4` path); with no sibling, every HLS request 404s and transcoding never works for those formats. Narrow reach: `--transcode` is opt-in and EXPERIMENTAL, only Safari requests the `.m3u8` sources, and vid.rs:139 always emits a final fallback `<source>` with the real URL.

```rust
fn find_original_video_path(base: &str) -> String {
    format!("{base}.mp4")
}
```

**Fix:** Keep the real extension in the HLS URL (emit `interview.mov-720p.m3u8` and have `parse_hls_request` return the base verbatim), or probe the `is_supported_video` extension list against the filesystem. Add tests for a `.mov` source with and without a same-stem `.mp4` sibling.

### [LOW] `evict_until_freed` subtracts a stale entry size, which can wrap `current_size`
`src/cache.rs:209`

The eviction pass snapshots `(key, priority, e.size_bytes)` while iterating, then calls `guard.remove(&key)` and subtracts the *snapshotted* size, discarding the returned entry — papaya's `remove` does return `Option<&V>`, so the correct value is available. A concurrent replace of that key between snapshot and removal over-subtracts, and `fetch_sub` wraps silently. `try_serve_pdf_cover`/`try_serve_pdf_cover_sidecar` bypass `claim_inflight` (unlike the video-metadata handler at server.rs:3214), so duplicate same-key inserts are reachable. Note the finding's "wraps to 2^64 and permanently kills the cache" arithmetic is wrong: eviction only runs when `current_size > max_size`, so an over-subtraction smaller than the current value just leaves drift. A wrap needs a single stale entry larger than roughly half the budget — conceivable for a full-resolution video-frame JPEG in the 2 MB default, not for PDF covers (capped at 1200 px).

**Fix:** Subtract what was actually removed: `if let Some(removed) = guard.remove(&key) { self.current_size.fetch_sub(removed.size_bytes, …) }`. Add a saturating `fetch_update` as defence in depth, and route the PDF-cover handlers through `claim_inflight`.

### [LOW] Mini link graph renders empty when site.json resolves after the graph chunk, and never rebuilds
`components/src/mbr-info.ts:141`

`_noteMeta` is a plain field (not `@state`) and `_isKnownNote` is a stable-identity class-field arrow. If site.json is still in flight when the panel opens (large repo, tiny links.json, 57 kB chunk), `build.ts:121 if (!isKnownNote(neighbor)) continue` rejects every neighbour and `mbr-mini-graph.render()` returns `nothing` for `nodes.length < 2`. When site.json arrives, `_noteMeta` is replaced but `_isKnownNote` keeps its identity, so `MbrMiniGraphElement.willUpdate`'s `changed.has('isKnownNote')` never fires. Self-heals only if the user closes and reopens the panel (which destroys and reconstructs the child). `mbr-genealogy.ts:84-92` handles the same late-arrival case correctly.

**Fix:** Declare `@state() private _noteMeta` and rebuild `_isKnownNote`/`_getMeta` as *new* closures inside the `subscribeSiteNav` callback, so Lit re-sets the property and the child restarts its BFS.

### [LOW] Pagefind search has no request-generation guard — a stale response can overwrite newer results
`components/src/mbr-search.ts:519`

`_performPagefindSearch` awaits `pagefind.search()` plus up to 20 parallel `data()` fetches and then writes `_results`/`_totalMatches`/`_durationMs` with no captured-query check, request id, or AbortController — while `_performServerSearch` (line 445) *does* abort the previous request. The 150 ms debounce only prevents per-keystroke starts; any search exceeding it can overlap the next and land last, leaving the input reading `abc` while the list shows `ab` results, so Enter navigates to a result that does not match the visible query. `Pagefind.debouncedSearch` is declared in the interface at line 74 but never called. Related: `_closeSearch` clears `_results` but a stale in-flight response still writes them, and `_openSearch` does not reset, so reopening can show the old query's results over an empty input. (The finding's claim that undebounced scope/filter toggles widen the window does not apply here — those controls render only when `serverMode` is true.)

**Fix:** Capture `const q = this._query;` before the first await and `if (q !== this._query) return;` before every state write, or switch to `pagefind.debouncedSearch`, which returns `null` for superseded calls.

### [LOW] editor-path-picker re-derives fs paths instead of reading site.json's `raw_path`
`components/src/editor-path-picker.ts:399`

`buildSuggestions` reconstructs each file's repo-relative path from its directory-style `url_path`, guessing the non-index interpretation and hardcoding `ext = exts[0] ?? 'md'`, even though site.json ships the authoritative `raw_path` (src/repo.rs:152-154, serialized and asserted by tests/server_integration.rs:1240). `docs/guide/index.md` (`url_path: "/docs/guide/"`) is suggested as `docs/guide.md`, which does not exist; picking it fills the input with a path that `validationError` then rejects as "A file already exists at that location" because `fsPathToApproxUrl` collides back to `/docs/guide/`. `.markdown` files are mis-rendered the same way.

```ts
// Reconstruct a plausible fs path for the file (non-index interpretation).
const fsPath = (segs.length ? segs.join('/') : 'index') + `.${ext}`;
```

**Fix:** Add `raw_path: string` to `SiteMarkdownFile` (line 27) and use `f.raw_path` directly, falling back to the current derivation only when it is absent.

### [LOW] Static build output is non-deterministic (site.json ordering) and no test checks it
`src/build.rs:1762`

`.mbr/site.json` and `.mbr/media.json` serialize `Repo`, whose `MarkdownFiles`/`OtherFiles` `Serialize` impls iterate a `papaya::HashMap` with the default randomly-seeded `RandomState` — a standalone probe showed a different order on all 5 runs. The `tags_data` object at build.rs:1773 is a std `HashMap` and is *also* order-unstable because Cargo.toml:81 enables tera's `preserve_order`, which turns on `serde_json/indexmap` and makes object key order equal insertion order. Two builds of an identical repo produce different `sha256sum`s: committed `build/` directories churn in `git diff`, and content-hash/ETag caching invalidates on no-op rebuilds. No rendered output changes (consumers key on unique `url_path`), and no test builds twice.

**Fix:** Collect into a `Vec` and sort by `url_path` before `serialize_element` in `MarkdownFiles::serialize` (src/repo.rs:105) and `OtherFiles::serialize` (:126); use a `BTreeMap` for `tags_data`. Add `tests/build_integration.rs::test_build_is_byte_deterministic` building one fixture into two dirs and `assert_eq!`ing the bytes of both JSON files.

---

## Maintainability & DRY (3 findings)

### [LOW] Render pipeline duplicated across two 11-argument entry points

`src/markdown.rs:595`

`render_with_cache` (async) and `render_sync` (blocking) each spell out the same ~33-line orchestration — wikilink transform, `collect_events_and_headings`, `has_h1`, `process_all_events`, `mark_incomplete_blocks`, `finalize_render` — differing only in the file read and the oembed source. Both hide 11 positional parameters behind `#[allow(clippy::too_many_arguments)]`. This is the structural cause of the oembed divergence reported separately: `collect_local_embeds` was added to `render_sync:914` but not to the matching branch at `render_with_cache:636`, and nothing detects the omission (`render_sync` has one caller and no tests; no parity test exists).

```rust
#[allow(clippy::too_many_arguments)]
pub async fn render_with_cache(
    file: PathBuf,
    root_path: &Path,
    oembed_timeout_ms: u64,
    link_transform_config: LinkTransformConfig,
    oembed_cache: Option<Arc<OembedCache>>,
    server_mode: bool,
    transcode_enabled: bool,
    valid_tag_sources: HashSet<String>,
    mark_incomplete: bool,
    incomplete_markers: &[String],
    wikilink_index: Option<Arc<WikilinkIndex>>,
) -> Result<MarkdownRenderResult, MarkdownError> {
```

Note: the individual stages *are* already extracted into shared functions (`finalize_render`'s doc comment even says so) — what is duplicated is the call sequence, which is why this is low rather than medium. Adjacent bare bools (`server_mode`/`transcode_enabled`) are also silently transposable.

**Fix:** Add a `RenderOptions` struct (mirroring the existing `MarkdownPageParams`/`MarkdownContextOptions` pattern in `src/page_context.rs:238`) and factor the shared sequence into `render_pipeline(markdown_input, prefetched_oembed, &opts)`, so both entry points reduce to "get text, get embeds, call pipeline." Removes all four `too_many_arguments` allows in the module.

### [LOW] `mbr-keys` drives four other components through their private fields via `as any`

`components/src/mbr-keys.ts:44`

`isModalOpen()` detects open overlays by reading TS-private fields off other elements, and the global handler invokes `_openSearch()`/`_openMediaBrowser()` the same way. `document.querySelector('mbr-search')` is already correctly typed via `HTMLElementTagNameMap`, so the `as any` discards real type information under `strict: true` — renaming `_isOpen` in `mbr-search.ts` compiles cleanly and `isModalOpen()` silently returns `false`, letting bare-letter shortcuts hijack keys while the search modal is open. `mbr-fuzzy-nav.ts:192` even exposes `public get isOpen()` under a `// Public Methods (called from mbr-keys)` banner, which `mbr-keys.ts:56` ignores in favor of `_isOpen`.

```ts
const search = document.querySelector('mbr-search');
if (search && (search as any)._isOpen) return true;
// ...
const browseSingle = document.querySelector('mbr-browse-single');
if (browseSingle && (browseSingle as any)._isDrawerOpen) return true;
```

There is no eslint config in `components/` to flag this, and the existing test is not a guard: `mbr-keys.test.ts:75` sets `(search as any)._isOpen = true` on an un-upgraded element (`test-setup.ts` registers no custom elements), asserting the same untyped string from both sides. Blast radius includes `mbr-slides.ts:194` and `mbr-editor.ts:54`. The copy-paste divergence `_isOpen` (`mbr-browse`) vs `_isDrawerOpen` (`mbr-browse-single`) is hard-coded here as two branches.

**Fix:** Declare `interface MbrOverlay { readonly isOpen: boolean; open(): void; close(): void }`, implement it as public members on `mbr-search`, `mbr-browse`, `mbr-browse-single` and `mbr-fuzzy-nav`, and have `isModalOpen()` iterate a selector list against that typed interface.

### [LOW] `mbr-fuzzy-nav` re-declares `PageLinks` and re-implements the `links.json` cache

`components/src/mbr-fuzzy-nav.ts:207`

`mbr-fuzzy-nav.ts:29-51` declares module-local `OutboundLink`/`InboundLink`/`PageLinks` and `_loadLinks()` does its own fetch into a per-element `_linksCache`, duplicating `graph/relationship-graph.ts:48-68` and `graph/links-cache.ts` — both in the same main bundle and already used by `mbr-info.ts:22,237`. Opening both the info panel and fuzzy nav on a page fetches `links.json` twice, since `fetchPageLinks`' module-level shared-promise cache is bypassed. The local `PageLinks` also omits the shared type's `relationships?: SiteRelationship[]`, so relationship work lands in two places.

```ts
const currentPath = window.location.pathname;
const normalizedPath = currentPath.endsWith('/') ? currentPath : currentPath + '/';
const linksUrl = normalizedPath + 'links.json';

const response = await fetch(linksUrl);
```

The extraction precedent already exists — `components/src/fuzzy.ts:4` notes the scoring algorithm "was extracted verbatim from `mbr-fuzzy-nav.ts`". One caveat on the swap: `fetchPageLinks` collapses 404 and network failure into `null`, while fuzzy-nav distinguishes them to render `_linksError` at `mbr-fuzzy-nav.ts:638`.

**Fix:** Delete the local interfaces and the `_loadLinks` fetch body; import `type PageLinks` from `./graph/relationship-graph.js` and call `fetchPageLinks(getCanonicalPath())` from `./graph/links-cache.js` as `mbr-info.ts` does, extending the shared cache to surface an error/404 distinction if `_linksError` is worth keeping.

---

## Testing Gaps (12 findings)

### [MEDIUM] `transform_wikilinks` rewrites `[[Source:value]]` inside code fences and code spans; no test covers it
`src/markdown.rs:622`

`transform_wikilinks` is a raw-string prepass applied to the whole file before pulldown-cmark parses, and `src/wikilink.rs:179-212` is a plain `find("[[")`/`find("]]")` loop with zero backtick or fence tracking. Default config has `tag_sources = ["tags"]` (src/config.rs:185), so the rewrite always fires: this repo's own `docs/reference/tags.md:80` (fenced ```` ```markdown ````), `:95` (inline spans), and `:320` render as `Check out [rust](/tags/rust/)` instead of showing the syntax being documented. Sibling pre-parse extensions each have an explicit "ignored in code block" test (`test_remark_hint_ignored_in_code_block`, src/markdown.rs:2771); wikilinks have none across src/markdown.rs:2942-3160 or src/wikilink.rs's 31 tests. Both verifiers reproduced the corruption end-to-end.

```rust
let markdown_input = if valid_tag_sources.is_empty() {
    raw_markdown_input
} else {
    transform_wikilinks(&raw_markdown_input, &valid_tag_sources)  // whole file, code included
};
```

**Fix:** Add `test_wikilink_not_transformed_in_code_block()` asserting `html.contains("[[Tags:rust]]")` for both a fenced block and an inline span; then move the transform into the event stream guarded by `state.in_code_block` (src/markdown.rs:1385-1392), or mask fenced/inline-code ranges before rewriting. The same prepass at src/markdown.rs:900 needs the same treatment.

### [MEDIUM] Full-text content search path has zero assertions; a silent regression already shipped and was fixed blind
`src/search.rs:645`

`search_file_content` (path rejoin, ripgrep scan, line numbers, snippet truncation, `is_content_match`) executes under the default `scope: All` but nothing asserts on its output — every search test's query term also appears in a filename or frontmatter title, so metadata search alone satisfies them. `rg 'snippet|is_content_match' tests/` returns no relevant hits, and `test_search_with_folder_scope` (tests/server_integration.rs:891) has an explicit `|| all_results == 1` escape clause that passes vacuously when content search returns nothing. This is not hypothetical: HEAD commit 995634f changed `raw_path` from absolute to repo-relative and hand-patched this line in the same commit — had it been missed, `path.exists()` would be false for every file, content search would return nothing, and the whole suite would still be green.

```rust
let path = self.root_dir.join(&info.raw_path);
if !path.exists() {
    return Ok(None);   // silent; no log
}
```

**Fix:** Add `test_search_content_scope_finds_body_only_term()` in tests/server_integration.rs — a file whose body contains `brownfox` but whose title/filename do not, POST `{"q":"brownfox","scope":"content"}`, assert one result with `is_content_match == true` and a non-empty snippet. Add a unit test for the multi-byte `floor_char_boundary` truncation branch (src/search.rs:694-700). Also drop the now-stale `#[allow(dead_code)]` on `root_dir` (src/search.rs:258).

### [MEDIUM] Server-mode backlinks (regex grep) and build-mode backlinks (parser inversion) diverge; never cross-checked
`src/link_grep.rs:604`

Inbound links have two independent implementations: `find_inbound_links` regex-scans raw file text (no fence/code-span/frontmatter handling anywhere in the 1290-line module), while `Builder::write_link_files` (src/build.rs:637-671) inverts parser-derived outbound links from `Event::Start(Tag::Link)`. A link written inside a fenced code block produces a phantom backlink in server mode (`/notes/howto/` appears in `/docs/guide/links.json` and the info-panel mini graph) and none in build mode — reproduced empirically. Server mode is even internally asymmetric: that page's own outbound list is parser-derived and omits the target. link_grep's 32 tests (src/link_grep.rs:751-1290) use only bare link text; `test_build_links_json_bidirectional` (tests/build_integration.rs:797) uses a flat two-file repo; no test compares the two modes.

**Fix:** Add `find_inbound_links_ignores_links_inside_code_blocks()` in src/link_grep.rs, then a cross-implementation test over one nested fixture (relative `../`, `./`, absolute, wikilink, code-fenced decoy) asserting the sorted `inbound` sets from `build/<page>/links.json` and `GET /<page>/links.json` are equal.

### [MEDIUM] Section wrapping emits mis-nested HTML for rules inside blockquotes/list items; no golden corpus exists
`src/html.rs:255`

html.rs is a forked pulldown-cmark renderer with 5 tests, all substring assertions on 2 config toggles, and no snapshot harness (no insta/expect-test in dev-dependencies, no `*.expected.html` fixtures anywhere). The `Rule` arm unconditionally writes `</section>\n<hr />\n<section>` with no container-depth tracking, and `enable_sections` is always true on the render path (src/markdown.rs:835 → src/html.rs:110-116). A verifier ran the real renderer on `"> q\n>\n> ---\n>\n> r"` and got `</section>` emitted between `<blockquote>` and `</blockquote>`, and likewise between `<li>` and `</li>` — content after the rule escapes its container on browser reparse. `test_sections_enabled_default` (src/html.rs:932) only asserts `html.contains("<section>")`.

Note: open/close counts stay balanced (4/4 in the repro), so the obvious `matches("<section").count() == matches("</section>").count()` assertion would **pass** with the bug present — the test must track container-nesting depth.

**Fix:** Add a test asserting no `</section>` appears between a `<blockquote>`/`<li>` open and its close, and suppress the section split when the Rule is inside a container. Separately add a golden corpus (`tests/fixtures/render/*.md` + `*.expected.html`, `assert_eq!` on full output) covering tables, footnotes, task lists, nested lists, math, HTML blocks, and smart punctuation.

### [MEDIUM] Heading text extraction drops inline code, math, and footnote refs, corrupting TOC labels and anchor IDs
`src/markdown.rs:422`

`collect_events_and_headings` accumulates heading text only from `Event::Text`; `Event::Code`, `InlineMath`, `InlineHtml`, and `FootnoteReference` fall through to the `_ =>` arm at src/markdown.rs:547 and are never appended. `## The \`main\` function` yields `HeadingInfo.text == "The  function"` and anchor id `the--function` (whitespace maps to `-` before `split_whitespace`, so the double space becomes a double dash). This repo has 13 such headings (docs/customization/index.md:11, docs/markdown/relationships.md:31/:98, docs/customization/templates.md:55/70/82, …); the same drop corrupts the H1 title fallback via `extract_first_h1` (src/markdown.rs:297), so site.json titles and `<title>` are affected too. `parse_extracts_headings_metadata` (tests/parse_integration.rs:79) uses only plain-text headings.

Two corrections to the original write-up: emphasis is **not** dropped (its inner Text event is captured), and there is no anchor checker — build.rs:2117 skips `#` hrefs entirely, so hand-written `[link](#the-main-function)` fails as a dead in-page jump, never a build error.

**Fix:** Extend the guard to `Event::Text(t) | Event::Code(t) if in_heading_text.is_some()` and handle `InlineMath`/`FootnoteReference` explicitly; add a test asserting `doc.headings[0].text == "The main function"` and `.id == "the-main-function"`.

### [MEDIUM] `Config::read()` layering untested; docs state the opposite of implemented env-vs-toml precedence
`src/config.rs:624`

`Config::read()` is the entire config pipeline and has no direct test — under default features (`gui`, `media-metadata`; `ffi` off) it is called only from main.rs/browser.rs, and `pub mod quicklook` is gated behind `ffi` (src/lib.rs:63), so its one incidental caller is excluded from `cargo test`. Figment's `merge` gives the *later* provider precedence (verified against figment 0.10.19 and reproduced standalone), so `.mbr/config.toml` beats `MBR_*` env — contradicting docs/reference/configuration.md:705 ("Environment variables override config file settings") while agreeing with the mermaid diagram at lines 10-27 of the same file. `grep -rn "MBR_" --include=*.rs src tests benches` returns exactly one hit: the `Env::prefixed` call itself.

```rust
.merge(Serialized::defaults(default_config))
.merge(Env::prefixed("MBR_"))
.merge(Toml::file(root_dir.join(".mbr/config.toml")))   // wins over env
```

The `ConfigError::ParseFailed` branch is equally untested (errors.rs:621 only exercises the `From` impl), and src/quicklook.rs:130 does `Config::read(&root).unwrap_or_default()`, swallowing it entirely.

**Fix:** Decide the intended precedence, correct docs line 705 (or swap the two `merge` calls), then add tests for: defaults-only, env-only, toml-only, all-three-set-`theme` (assert the documented winner), and a malformed `.mbr/config.toml` returning `Err(ConfigError::ParseFailed)`. Note the suggested `figment::Jail` requires enabling figment's `test` feature — Cargo.toml:49 currently declares only `["toml", "env"]`, and `temp_env` is not a dev-dependency.

### [MEDIUM] ~8,100 lines across the 10 largest Lit components have no tests
`components/src/mbr-search.ts:471`

28 test files exist under components/src, but the 10 largest logic-bearing elements have none: mbr-browse-single (1285 LOC), mbr-search (1208), mbr-media-browser (1162), mbr-video-extras (1072), mbr-fuzzy-nav (1031), editor-crepe (897), mbr-media-viewer (526), mbr-page-errors (500), mbr-live-reload (302), dynamic-loader (112). These build server request payloads and parse responses. `mbr-search._performServerSearch` hand-builds the POST body as `Record<string, any>` (mbr-search.ts:454-468) with no shared type against the Rust `SearchQuery`, which has `#[serde(default)]` on `filetype`/`folder`/`folder_scope` and **no** `deny_unknown_fields` (src/search.rs:155-184) — so a field-name drift (`folder_scope` → `folderScope`) silently falls back to `FolderScope::Everywhere` and returns 200 with wrong-scope results. Rust integration tests hand-write their own JSON literals (tests/server_integration.rs:905, :915, :3569), so they are fully decoupled from the client. CI (.github/workflows/ci.yml:388) runs `bun run test` with no coverage threshold, so zero-test files pass silently.

**Fix:** Add `mbr-search.test.ts` stubbing `fetch` and asserting the exact JSON body per scope/folder_scope/filetype combination plus the `!response.ok` and `AbortError` branches; `mbr-live-reload.test.ts` for `_shouldReloadForFile` (mbr-live-reload.ts:235) and reconnect backoff; `dynamic-loader.test.ts` for `getMbrAssetBase()` under a non-root basePath (mbr-genealogy.test.ts:23 mocks it to `''`, so that case is uncovered). Then mbr-browse-single, mbr-media-browser, mbr-video-extras by LOC.

### [LOW] `generate_anchor_id` has no tests and its dedup scheme produces duplicate IDs
`src/markdown.rs:1243`

`generate_anchor_id` is the sole source of heading anchor IDs and has zero test references (`grep -rn generate_anchor_id` hits only the definition and the single call site at src/markdown.rs:445). Its per-base counter appends `-N` without checking whether the composed candidate is already taken, so `["Step 1", "Step 1", "Step 1-2"]` → `step-1`, `step-1-2`, `step-1-2` (reproduced standalone; same for `["Intro","Intro","Intro 2"]`). Two headings share an id, so the info-panel TOC link (mbr-info.ts:507), the permalink anchor (mbr-heading-enhancer.ts:63), and `getElementById` (mbr-fuzzy-nav.ts:270/400) all resolve to the first match.

```rust
let count = anchor_ids.entry(base_id.clone()).or_insert(0);
*count += 1;
if *count == 1 { base_id } else { format!("{}-{}", base_id, count) }
```

Severity is low, not the claimed high: link validation is unaffected (build.rs:2143 strips fragments before checking) and headings are not serialized into site.json — the blast radius is a wrong in-page scroll target.

**Fix:** Loop the suffix — `while anchor_ids.contains_key(&candidate) { n += 1 }` — and add a uniqueness test over `["Step 1","Step 1","Step 1-2","","","Ünïcode Ünïcode"]` asserting `ids.iter().collect::<HashSet<_>>().len() == ids.len()`, plus a proptest over `Vec<String>`.

### [LOW] `impl From<&Config> for ServerConfig` is never executed by a test
`src/server.rs:472`

The conversion maps ~30 config fields into runtime config and runs only from src/main.rs:427/452 and src/browser.rs:319; every server test uses the struct literal at tests/server_integration.rs:11-52, and the doc example at src/server.rs:400 is fenced ```` ```ignore ````. The only genuinely unguarded logic is the mode-specific default `mark_incomplete: config.mark_incomplete.unwrap_or(true)` (src/server.rs:501) — flipping it to `unwrap_or(false)` leaves `cargo test` green because `test_server_marks_incomplete_blocks_by_default` (tests/server_integration.rs:220) routes through the helper that hardcodes `mark_incomplete: true`. The build side's equivalent (src/build.rs:838) *is* covered because tests/build_integration.rs:186-224 passes a real `mbr::Config`.

Correction that caps severity at low: dropping a field line is a compile error (E0063) — `ServerConfig` has no `Default` impl and the literal has no `..base`, so only same-typed mis-mappings and the one `unwrap_or` default can regress silently.

**Fix:** Add `test_server_config_default_mark_incomplete_is_on` asserting `ServerConfig::from(&Config::default()).mark_incomplete == true`, mirroring the build-side coverage.

### [LOW] main.rs flag-to-config wiring is unreachable from the test suite
`src/main.rs:181`

src/main.rs:141-222 is the only place CLI flags are applied to `Config`, and it lives inline inside `async fn main()` — not callable from any test. src/cli.rs tests assert only that clap fills `Args` fields (cli.rs:371 checks `args.theme == Some("amber")`, never that `config.theme` changed); integration tests set `Config`/`ServerConfig` directly. There is no `Command::new`, `assert_cmd`, or `CARGO_BIN_EXE` anywhere in tests/ or src/. Deleting src/main.rs:181-183 leaves fmt/clippy/test green while `mbr -s --theme amber` silently serves the default. Same hole for `--host` (incl. the IPv6-rejection branch), `--template-folder` (incl. `TemplateFolderNotDirectory`, constructed only at main.rs:158 with no test reference), `--build-concurrency`, `--transcode`, `--mark-incomplete`, `--title-prefix/suffix`, and `--edit`'s re-`validate()`.

Two mitigations cap this at low: CI does smoke-run the binary (.github/workflows/ci.yml:187, docs.yml:42 with `--output ./docs-build` followed by `ls`, so relative `--output` resolution at main.rs:335-341 *is* exercised), and the `--edit` branch is defense-in-depth — `check_edit_access` (src/server.rs:1759-1802) independently 401s every edit request regardless of startup validation.

**Fix:** Extract the override block into `pub fn apply_overrides(config: Config, args: &Args) -> Result<Config, ConfigError>` in cli.rs and unit-test set/omit per flag plus the two error branches (`--host ::1` → `InvalidHost`, `--template-folder <file>` → `TemplateFolderNotDirectory`).

### [LOW] `--fail-on-broken-links` exit path is untested, so the CI docs gate could silently no-op
`src/main.rs:363`

The flag exists only in src/cli.rs and src/main.rs — it is absent from config.rs, lib.rs, and build.rs, so no library-level (testable) code implements it. tests/build_integration.rs:918+ asserts the `stats.broken_links` counter via the `Builder` API; cli.rs:408 asserts clap sets the bool; nothing connects the two. The only execution is `.github/workflows/ci.yml:187` (`mbr -b docs --fail-on-broken-links`) against a docs tree that by design has zero broken links, so only the pass path is ever taken.

```rust
if args.fail_on_broken_links && stats.broken_links > 0 {
    eprintln!("Error: {} broken internal link(s) detected; ...", stats.broken_links);
    std::process::exit(1);
}
```

**Fix:** Add `tests/cli_integration.rs::test_fail_on_broken_links_exits_nonzero` — TempDir repo with `[Broken](/missing/)`, run `env!("CARGO_BIN_EXE_mbr")`, assert `status.code() == Some(1)` and stderr contains "broken internal link". Companions for the clean case and the `--skip-link-checks` interaction. This introduces a subprocess test pattern the repo does not currently have; it would also cover the CLI-wiring gap above.

### [LOW] One malformed user template silently discards ALL `.mbr/` HTML overrides
`src/templates.rs:77`

`Tera::new(glob)` parses every `.mbr/**/*.html` at once; tera 1.20.1's `load_from_glob` (tera.rs:162-231) accumulates errors and returns `Err`, dropping the partially populated Tera. The `unwrap_or_else` then falls back to `Tera::default()`, so an unclosed `{% if %}` in a newly added `.mbr/index.html` also silently discards a working `.mbr/_nav.html`. The only signal is a `tracing::warn!`; `reload()` (templates.rs:121) hits the same path, and server.rs:1040 only logs, so hot reload can swap in defaults mid-session. templates.rs's two loading tests both write only valid templates, and no integration test writes any `.mbr/*.html` at all.

Correction: `.mbr/theme.css` is **not** dropped — the glob is `**/*.html` and CSS is served as a static asset.

**Fix:** Add `test_malformed_user_template_falls_back_to_defaults` (valid `_nav.html` + broken `index.html`; assert `Templates::new` returns `Ok` and built-in output renders) plus a server test asserting 200 not 500. For per-file resilience, load templates individually with `add_raw_template` and skip only the failing file, then assert the valid `_nav.html` override survives.

---

## Performance (18 findings)

### [HIGH] `TagIndex::add_page` clones the entire page Vec per insert — O(k²) per tag
`src/tag_index.rs:142`

Every `add_page` rebuilds the tag's whole `Vec<TaggedPage>` via `existing.clone()` plus an O(k) linear dedupe scan, so indexing k pages under one tag costs O(k²) clones (3–4 heap `String`s each). Default config ships one tag source (`tags`, `src/config.rs:186`), so a 10k-note vault where most notes share `tags: [note]` melts down. Measured in a standalone repro against papaya 0.2.4: 10k inserts = **~20s wall, ~357s CPU, 9.3 GB peak RSS**; under `into_par_iter` the update closure fired 103,868 times instead of 9,999 (10.4x CAS-retry amplification, since `U: Fn(&V) -> V` re-runs the clone). Even a realistic Zipf distribution (10k notes × 3 tags, top tag 1240 pages) takes 784 ms — already blowing the sub-second scan budget. It is not one-off: `rebuild_tag_index()` (`src/repo.rs:1241`) replays the entire loop **serially** on every single-file save in server mode (`src/server.rs:1149`), and `directory_to_html` re-runs per-directory inserts on every directory request.

```rust
guard.update_or_insert_with(
    key,
    move |existing| {
        if existing.iter().any(|p| p.url_path == page.url_path) {
            existing.clone()
        } else {
            let mut pages = existing.clone();
            pages.push(page.clone());
            pages
        }
    },
    || vec![page_for_insert],
);
```

**Fix:** stop storing `Vec<TaggedPage>` as an immutable value that must be cloned to grow — either key the map by `(tag_key, url_path)` so inserts are O(1) and `get_pages` collects on read, or hold `Arc<Mutex<Vec<TaggedPage>>>` buckets; replace the linear dedupe with a `HashSet<String>` of url_paths. Note `benches/repo_scan.rs:28` passes `&[]` for `tag_sources`, so the tag index is never benchmarked at all — add a 10k-file/one-dominant-tag bench.

### [MEDIUM] Directory listings re-scan the directory from disk and re-parse frontmatter on every request
`src/server.rs:4612`

`directory_to_html` builds a throwaway `Repo` and calls `scan_folder` per request, re-opening and re-parsing YAML frontmatter for every file in that directory — data already resident in `config.repo.markdown_files`. `Repo::init` creates a fresh `scanned_folders: HashSet::new()`, so `scan_folder`'s own memo guard (`src/repo.rs:619`) can never fire across requests, and the response is `CACHE_CONTROL_NO_STORE` with the ETag computed after the full render, so there is no 304 short-circuit either. Measured on a release build with 5000 markdown files in one directory: home listing = **387 ms, of which 28 ms WalkDir + 326 ms frontmatter extraction (~91% of latency)**, identical across five consecutive requests (0.379/0.364/0.360/0.362/0.362 s — zero caching). A markdown page using the in-memory `sibling_nav_cache` served in 0.6 ms. Every request also pays the quadratic `add_page` cost above into a temp index that is then discarded. Reachable from any directory without an index file, including the home page (`src/server.rs:4893`).

**Fix:** serve listings from `config.repo.markdown_files` using the same parent-directory predicate as `compute_sibling_files` (`src/server.rs:5030`), memoized per directory and invalidated alongside `sibling_nav_cache` (`src/server.rs:1184`). Keep the disk scan only as a fallback when `!repo.is_scan_complete()` — `tests/server_integration.rs:293` deliberately does not call `wait_for_scan()`.

### [MEDIUM] `/.mbr/site.json` is rebuilt and double-serialized per request, with no cached body and no conditional response
`src/server.rs:1524`

*(Merged: two findings, same root cause.)* `get_site_info` materializes the whole `Repo` into an intermediate `serde_json::Value` DOM — including the entire `other_files` subtree, which it discards one statement later at line 1531 — then re-serializes that tree to a `String`, inline on a Tokio worker with no `spawn_blocking`, no cached body, and only a `Content-Type` header. `components/src/shared.ts:222` fetches this at module scope and mbr is an MPA, so it runs on every navigation. Measured with a faithful serde_json harness at 10k markdown + 20k assets: `to_value` 9.3 ms + `to_string` 2.6 ms = 16.4 ms/request producing 3.65 MB, versus 2.3 ms for direct serialization without `other_files` — ~14 ms and 20k transient `Value` subtrees of pure waste per page view, before `CompressionLayer` gzips it.

```rust
let mut response = serde_json::to_value(&*config.repo)?;
if let Some(obj) = response.as_object_mut() {
    obj.remove("other_files");   // built, then thrown away
```

**Fix:** cache the rendered bytes in `ServerState` (an `ArcSwap<Bytes>` + ETag), rebuild after `scan_all`/watcher invalidation where `sibling_nav_cache` is cleared, and serve the clone. Use a dedicated `SiteJson` view struct so `other_files` is never materialized. Note the ETag half of the usual advice buys nothing here on its own — `grep -n "IF_NONE_MATCH|NOT_MODIFIED|304" src/*.rs` returns **zero hits**, so no endpoint in this server answers conditional requests; 304 handling would have to be added too.

### [MEDIUM] Static build never strips `other_files` from site.json — media catalog shipped twice
`src/build.rs:1766`

*(Merged: two findings, same root cause.)* `Builder::handle_mbr_folder` serializes the whole `Repo` into `site.json` and inserts `sort`/`tag_sources`, but unlike `server.rs:1531` it never calls `obj.remove("other_files")` — then writes the identical array again to `media.json` (step 4b). `src/repo.rs:65` has no `#[serde(skip)]` (the manual `impl Serialize for Repo` that would have renamed the field is commented out at `repo.rs:317`), so the key really ships. Verified empirically: a build with 2000 PNGs produced site.json = 378,502 B of which `other_files` was 402,000 B of JSON — **97.7% of the file** — and a sorted comparison against media.json's array returned identical. Both files are fetched at module scope on every page (`components/src/shared.ts:222` and `:300`) and re-parsed; no consumer reads `site.json.other_files` (only `mbr-media-browser.ts:112`, `editor-media-picker.ts`, `editor-crepe.ts:832`, all off media.json).

**Fix:** add `obj.remove("other_files");` as the first statement in the `if let Some(obj) = response.as_object_mut()` block, mirroring `src/server.rs:1531`, and add a build_integration assertion (`site["other_files"].is_null()`) matching `tests/server_integration.rs:1492`. Longer term, extract one shared `site_json_payload(repo, config) -> Value` — the two payloads already diverge on `other_files`, `sidebar_style`/`sidebar_max_items`, and `tag_sources`.

### [MEDIUM] Server computes backlinks with a full-repo grep per page instead of inverting the link index once
`src/link_grep.rs:589`

Every `links.json` inbound cache miss calls `find_inbound_links`, which `WalkDir`s the entire repo and `read_to_string`s every markdown file, single-threaded. The obvious escape hatch does not fire: `compute_patterns_for_folder` (`src/link_grep.rs:292`) always adds the absolute `/{target}` variant, so the pattern set is never empty and no folder is ever skipped. The info panel's BFS fetches `links.json` for the focus page plus the whole level-1 frontier (`components/src/graph/bfs.ts:69`, default depth 2, up to 80 nodes), so opening the panel triggers N full-repo walks, serialized 2-at-a-time by `INBOUND_GREP_MAX_CONCURRENCY` (`src/server.rs:94`). The single-flight guard only dedups the *same* path, not the N distinct neighbours, and the 4 MB / 300 s cache (`src/server.rs:83`, `:89`) guarantees re-greps. `Builder::write_link_files` (`src/build.rs:637`) already does the O(repo) inversion once for all pages.

**Fix:** build a repo-wide inbound index in the background after `scan_all` (and incrementally on watcher events) and serve inbound from it, keeping `find_inbound_links` only as a warm-up fallback. Note the server's outbound `link_cache` is populated lazily per rendered page (`src/server.rs:4472`), so the build's inversion is not directly portable — the server needs its own one-pass repo-wide extraction.

### [MEDIUM] Search deep-clones every `MarkdownInfo` per query and rebuilds a grep `Searcher` per file
`src/search.rs:359`

`search_metadata` (line 359) and `search_content` (line 629) each `.map(|(_, info)| info.clone()).collect()` the full candidate set into a `Vec<MarkdownInfo>` — `PathBuf`, two `String`s, `Vec<RawRelationship>`, and a `HashMap<String, serde_json::Value>` of frontmatter per entry — only to iterate once and pass `&info` to a closure. The existing `.filter(|(_, info)| self.matches_folder_filter(info, ...))` closures already borrow off the live pin guard, so the clone is not a borrowck requirement. Separately `search_file_content` (`src/search.rs:664`) builds a fresh `SearcherBuilder::new()...build()` per file; grep-searcher 0.1.17 allocates `vec![0; 8*1024]` decode + `vec![0; 64*1024]` line buffers on each build — **~72 KB zero-filled per file scanned**. Default scope is `SearchScope::All` with `FolderScope::Everywhere`, and the POST `/.mbr/search` endpoint has no minimum-length gate (only the client debounces at 150 ms / 2 chars).

**Fix:** iterate `&MarkdownInfo` straight off the held pin guard in `search_metadata` and drop the `clone()/collect()`; hoist the `Searcher` construction into `search_content` and pass `&mut Searcher` down. Caveat: holding the papaya guard across `search_content`'s file I/O delays seize reclamation during a background scan — the `Searcher` hoist and the metadata fix are the unconditionally safe parts.

### [MEDIUM] Video/PDF metadata cache is sized from `oembed_cache_size` (2 MB), thrashing on cover images
`src/server.rs:895`

`VideoMetadataCache` stores full JPEG cover payloads (`Cover(Vec<u8>)`, `src/video_metadata_cache.rs:15`) but is constructed with the oembed *text*-metadata budget (`DEFAULT_OEMBED_CACHE_SIZE = 2 MB`, `src/config.rs:16`) — this is the only production construction of it. Covers render at up to 1200 px wide (`PDF_COVER_MAX_WIDTH`, `src/pdf_metadata.rs:202`) at JPEG q85, so only ~13–25 fit, and eviction is FIFO on `inserted_at` (`video_metadata_cache.rs:94`), not LRU. On repos without pre-generated `.pdf.cover.jpg` sidecars each miss re-runs `extract_cover_sync`, serialized behind a 1-permit semaphore *and* a global mutex (`pdf_metadata.rs:150-168`) and re-binding the pdfium library per call. Responses carry `no-cache` with no ETag/Last-Modified (`src/server.rs:3960`), so a media-browser grid re-requests every cover per visit. Setting `--oembed-cache-size 0` to disable link previews — documented as valid at `docs/reference/cli.md:55` — silently disables all media caching via `SizeBoundedMap::is_disabled`.

```rust
// Initialize video metadata cache with same size as oembed cache
let video_metadata_cache = Arc::new(VideoMetadataCache::new(oembed_cache_size));
```

**Fix:** give it its own `media_cache_size` (default 64–128 MB, `MBR_MEDIA_CACHE_SIZE`) documented in `docs/reference/configuration.md`, and cache the `Pdfium` bindings in a `OnceLock` instead of calling `create_pdfium_instance()` per cover.

### [MEDIUM] media.json is fetched at module scope on every page although only the media browser consumes it
`components/src/shared.ts:300`

`shared.ts` is in the always-loaded main bundle (imported by `mbr-nav`, `mbr-browse`, `mbr-search`, `mbr-info`; `templates/_scripts.html:2` loads it everywhere) and fires a top-level `fetch(getMediaJsonUrl())`. Confirmed against a real vite build: the minified bundle contains `fetch(...".mbr/media.json").then(...)` as a **top-level statement, not inside any function** — Rollup did not tree-shake it, and `inlineDynamicImports: true` means dynamic imports cannot defer it either. The sole consumer is `mbr-media-browser.ts:108`, and that element is only instantiated inside the search popup (`mbr-search.ts:725`, guarded by `_isMediaBrowserOpen`). The whole catalog is parsed and retained in `mediaNavState.data` for the page lifetime. In server mode the handler awaits `wait_for_media()` (`src/server.rs:1584`), so on a cold start the first page's request parks until every PDF has been lopdf-parsed and every image/video probed.

**Fix:** replace the eager `const mediaNav = fetch(...)` with a memoized `loadMediaNav()` triggered from `subscribeMediaNav` in `mbr-media-browser.connectedCallback`.

### [MEDIUM] `mbr-nav` rebuilds a second full folder tree and sorts the whole site on every page load to render two buttons
`components/src/mbr-nav.ts:68`

`_computeNavigation` runs `buildFolderTree(allFiles)` + `flattenToLinearSequence(...)` over the entire site just to locate prev/next, duplicating the tree `mbr-browse` built from the same `site.json` moments earlier (`mbr-browse.ts:145`). It is unconditional — `<mbr-nav>` is hard-coded in `templates/index.html:15`, eagerly exported from the main bundle, with no `requestIdleCallback`, size cap, or memo. Measured on Node 24 with 10,000 files: ~10 ms warm / ~21 ms first run, on top of `mbr-browse`'s own tree and the other eager passes; prev/next is below the fold. Separately, `buildFolderTree` is called **without `nav.index_file`**, so it falls back to the `'index.md'` default (`sorting.ts:190`) and mis-orders repos configured with `_index.md` — the folder's landing page gets pushed into the parent's `files` instead of the folder node (`sorting.ts:222`, `:257`). `site.json` does expose the field (`src/repo.rs:50`), and `mbr-browse` reads it correctly.

**Fix:** memoize the folder tree once in `shared.ts` off the `siteNav` promise and share it across `mbr-nav`/`mbr-browse`/`mbr-browse-single`; pass `nav.index_file` through; defer `_computeNavigation` to `requestIdleCallback`; swap `valA.toLowerCase().localeCompare(...)` at `components/src/sorting.ts:130` for a module-level `Intl.Collator({sensitivity:'base'})` (~3x faster on the same 10k sort).

### [MEDIUM] `mbr-browse` middle pane renders one card per matching file with no virtualization or cap
`components/src/mbr-browse.ts:1039`

`_renderMiddlePane` maps every `_selectedFiles` entry to a card. `_updateSelectedFiles` caps nothing for the `tag` (line 443), `folder` (452) or `frontmatter` (480) selection types — only `recent` is bounded, at `MAX_RECENT = 30`. `_allFiles` holds the full site index (line 132), and there is no `IntersectionObserver`, scroll listener, `repeat()` keying, or `content-visibility` anywhere in the file. Clicking a common tag in a 10k-note vault synchronously materializes thousands of cards (~8–13 elements each) into a 320 px scroll pane that shows ~8 at a time. `mbr-browse-single.ts:656` already implements exactly the needed paging (`_getPageItems`/`_hasMoreItems`/`_showMore`), and `_renderRecentSection` slices to 5 with a "Show all N" button — so this pane is the outlier.

**Fix:** slice `_selectedFiles` to an initial window (~100) with a "show more" affordance, or add an `IntersectionObserver` sentinel / `@lit-labs/virtualizer`.

### [LOW] `mbr-media-browser` filters and sorts the whole library three times per render
`components/src/mbr-media-browser.ts:642`

*(Merged: two findings, same root cause.)* `_renderGrid` calls `_getFilteredFiles()` at 642, `_getDisplayedFiles()` at 643 (which calls `_getFilteredFiles()` again), and `render()` calls it a third time at 716 — plus one O(n) `_getTypeCount()` scan per type tab. Nothing is memoized: no `willUpdate`/`shouldUpdate`, no cache field, no debounce, so every keystroke in the text filter and every card `@mouseenter` (line 611 writes the `@state` `_selectedIndex`) pays all of it. Measured at 20k entries: ~0.9 ms per pass with the default `created` sort, ~2.3 ms with `alpha`/`localeCompare`, so ~2–8 ms of redundant work per render — real waste, but under a frame budget and dominated by Lit re-rendering the 200 cards.

**Fix:** compute the filtered+sorted array once in `willUpdate` keyed on `(_allMediaFiles, _selectedType, _textFilter, _sortField, _sortDirection)` and derive `displayedFiles`, `filteredCount`, and the type counts from it. Note `_selectedIndex` also drives keyboard nav (lines 344, 364-401), so a plain CSS `:hover` swap is not a drop-in replacement.

### [LOW] `mbr-search`'s lazy media-browser import is defeated by `inlineDynamicImports`
`components/src/mbr-search.ts:8`

`const loadMediaBrowser = () => import('./mbr-media-browser.js')` reads as lazy, but `components/vite.config.ts:80` sets `output.inlineDynamicImports: true` on a single-entry lib build, so Rollup inlines the whole component. Verified by building: output is one bundle containing media-browser-only symbols (`focusTextFilter`, "Browse Media"); a control build stubbing the import went 247.33 kB → 222.56 kB (**~24.8 kB min / ~4.8 kB gz shipped on every page**), and the `@customElement` registration runs at parse time. The project's own `vite.graph.config.ts:6` and `vite.editor.config.ts:6` document exactly this hazard, which is why editor/graph/genealogy use `import(/* @vite-ignore */ url)` against separately built chunks. Nothing warns; the regression is invisible.

**Fix:** either add a `vite.media.config.ts` chunk loaded via the `/* @vite-ignore */` runtime-URL pattern (and register it in `src/server.rs:5173` alongside the other three), or drop the fake indirection and import statically so the source matches reality.

### [LOW] Markdown `<img>` output has no `loading="lazy"`, `decoding`, or intrinsic dimensions
`src/html.rs:511`

The custom pulldown-cmark writer emits a bare `<img src alt title />`. Nothing downstream adds the attributes — no template, CSS rule, or component touches `img` (the only `loading="lazy"` in the codebase is `src/oembed.rs:234` for Giphy and `mbr-media-browser.ts:626`), so a photo-heavy note fires N eager requests at first paint and shifts layout as each pops in. Note the finding's premise that dimensions are already available is only half true: `StaticFileKind::Image { width, height }` is populated by `populate_media_metadata`, which is `media-metadata`-gated and called **only from server mode** (`src/server.rs:962`, `:1177`) as a deferred background pass — `src/build.rs` never calls it, so static builds have `width: None, height: None` for every image.

**Fix:** emit `loading="lazy" decoding="async"` unconditionally in the `Tag::Image` arm (a two-line change, zero prerequisites). The `width`/`height` half requires new work — reading image headers during the build scan — not just threading `other_files` into the writer.

### [LOW] `mbr-browse` recomputes frontmatter value counts with an O(values × files) scan inside `render()`
`components/src/mbr-browse.ts:992`

`_renderDynamicSections` derives each value's count by filtering all of `_allFiles` inside a `.map()` inside the render function, even though `_detectDynamicFields` already computed those counts at line 337 and discards them at line 352 (`new Set(stats.values.keys())`). Any `@state` change while the panel is open re-runs it. Measured at the worst admissible shape (49 values × 10,000 files, dictionary-mode objects): 6.8 ms on Node, 1.6–2.5 ms on Bun — real but ~1–2 orders of magnitude below the finder's "multi-hundred-millisecond" estimate, and gated behind an explicit section expand (`_expandedSections` starts as `new Set(['notes'])`).

**Fix:** widen `_dynamicFields` (line 81) to `Map<string, Map<string, number>>`, return `stats.values` directly at line 352, and read `values.get(value)` in render. Minor behavior note: the detector skips values >100 chars and non-scalars, so the cached counts are not byte-identical to the render-time `String(...)` filter in edge cases.

### [LOW] `claim_inflight` get-then-insert is not atomic, so single-flight can admit two producers
`src/server.rs:68`

The doc comment at line 58 promises "Produce to exactly one caller", but `pin()` is a seize epoch guard (reclamation only, not mutual exclusion), so `get` and `insert` are two independent lock-free ops on a multi-threaded runtime. Two callers can both see `None`, both decode/grep, and the first one's unconditional `pin().remove(&key)` (lines 3230, 3679) drops the second's `Notify`, letting a third request start yet another producer. Neither call site is serialized upstream — the `inbound_grep_semaphore` is acquired *inside* the Produce arm. Correctness is preserved (idempotent values, bounded waiter timeouts), so this is wasted work in a nanosecond-wide window. `test_metadata_single_flight_one_decode` (`src/server.rs:6165`) claims the slot sequentially first, so it exercises the Wait path only.

**Fix:** use papaya 0.2.4's `try_insert` — insert a fresh `Notify`, return `Produce` on `Ok`, `Wait(current.clone())` on `Err(OccupiedError { current, .. })`. The same check-then-insert shape exists in `HlsCache::start_generation` (`src/video_transcode_cache.rs:104`) and should be fixed together.

---

## Review Completeness Critique

### Not covered

**Zero attention from any of the 15 reviewers (verified by `ls` + reading each coverage note):**

- **`/Users/pwalsh/src/sideprojects/mbr-markdown-browser/2026-07-25-code-review/quicklook/` (entire Swift target, ~5 files).** sec-supply only audited the UniFFI boundary; sec-path explicitly skipped it. Highest-risk thing I found in this pass: `quicklook/MBRPreview/MBRPreview.entitlements` grants `com.apple.security.temporary-exception.files.absolute-path.read-only` = `/`, and `MBRFileSchemeHandler` (PreviewViewController.swift:41-48) does `Data(contentsOf: URL(fileURLWithPath: url.path))` with **no containment check at all** — any path the user can read. Combined with `src/html.rs:245` passing raw `Html` events through, a `<script>` inside a previewed `.md` can `fetch('mbrfile:///Users/x/.ssh/id_rsa')` and render it. `config.preferences.setValue(true, forKey: "developerExtrasEnabled")` (line 140) is also on in the shipped extension. Nobody looked at this.
- **`src/errors.rs` (792 lines).** No reviewer mentions it. Most variants embed `PathBuf` (`RootDirNotFound { path }`, `ReadMarkdownFile { path }`, `ScanFolder { path }`, …) and `server.rs` handlers return `Result<Response, MbrError>`. sec-web/sec-frontend both asserted "error pages leak no absolute filesystem paths" without ever reading the error type that produces them.
- **`src/quicklook.rs` logic (1121 lines).** Only skimmed for `unsafe`. `convert_root_relative_urls` (line 248) does **regex rewriting of rendered HTML** into `mbrfile://` URLs and `resolve_asset_path` (line 219) joins untrusted URL paths against the root with no traversal check — this is the Rust half of the entitlement problem above.
- **`src/page_errors.rs` (848 lines)** — only ~120 lines read (dry-rust 55-175). **`src/link_rewrite.rs` lines 330-845** — the code that *mutates user files on disk* during renames; bug-derived flagged two suspected bugs there and dropped them unverified. **`src/embedded_pico.rs` (269), `src/embedded_hljs.rs` (198), `src/constants.rs` (13)** — unread.
- **`src/browser.rs` and `src/lib.rs` have no `#[cfg(test)]` module at all** (only 3 files in src/ don't); test-system never flagged this.
- **`components/src/mbr-slides.ts` (384) + reveal.js integration.** Zero attention from all four frontend reviewers. It does `main.innerHTML = ''` and re-parents `<section>` nodes after stripping nav/footer — the only DOM-restructuring component in the codebase, driven by `---`-generated sections.
- **Other TS files nobody named:** `mbr-footnote-preview.ts` (192), `mbr-heading-enhancer.ts` (105, writes `anchor.href` from a generated id), `mbr-link-enhancement.ts` (102), `mbr-sidebar-trigger.ts` (90), `editor-media-edit.ts` (108), `editor-frontmatter.ts` (83), `editor-media-target.ts` (63), `fuzzy.ts` (107), `graph/viewport.ts` (102), `genealogy/family-chart-data.ts` (95).
- **Templates nobody read:** `templates/media_viewer.html` (has `{{ media_type | safe }}` in three sinks incl. a JS string literal), `_breadcrumbs.html`, `_head_custom.html`, `_footer_custom.html`, `error.html` beyond a skim, `reveal-slides.css`.
- **Vendored third-party JS in `templates/`** (hljs 11.11.1, katex 0.16.27, mermaid 11.12.2, reveal 5.2.1, pico) — sec-supply flagged the *fetch script* but nobody checked these pinned versions against known advisories, nor that `components/src/mbr-mermaid.ts:63` calls `initialize()` with **no `securityLevel`** while rendering attacker-authored diagrams.

**Review dimensions absent entirely from the 15:**

- **Unicode / i18n correctness.** `rg` for `nfc|normaliz|unicode_normalization` across `src/` returns **zero hits**. On macOS (APFS returns NFD) a file `Café.md` will not match a wikilink/tag/link typed as NFC `Café`. Also: no CJK handling in `src/search.rs` (whitespace tokenization = unusable for CJK), Turkish-i hazard in the case-insensitive wikilink fallback, and `localeCompare` vs Rust `sort` producing different orders between server and client.
- **Accessibility.** ~26k lines of Lit with sporadic `aria-`/`role` (0-14 per file, `mbr-keys.ts` has 1). No one checked focus traps/restore in the search, fuzzy-nav, editor and media-viewer overlays, `aria-live` on live-reload, or keyboard reachability of the graph/genealogy canvases.
- **Docs-vs-code accuracy as a first-class pass.** `src/cli.rs` declares 21 `long =` flags; nobody diffed them against `docs/reference/cli.md` or `Config` fields against `configuration.md`. The one contradiction found (env vs config.toml precedence) was incidental.
- **Licensing/attribution.** Crate is `GPL-3.0-or-later` and `include_bytes!`s MIT/BSD/ISC third-party JS+CSS into the binary and into every static build; there is **no NOTICE/THIRD-PARTY file** in the repo root. sec-supply checked provenance, not license obligations.
- **Windows behavior as a dimension** (added in the tip commit #240): one speculative finding, no reviewer could test path separators, `\\?\` canonicalization, CRLF, or file-locking on rename.
- **Logging/observability**: no reviewer examined `tracing` levels or whether untrusted paths/URLs are logged unescaped.
- Housekeeping: `test-blockquotes.md` and `test-simple.md` are stray fixtures committed at repo root.

### Cheap checks that would settle open questions

- **Every perf-rust magnitude** (TagIndex O(k²), per-request site.json rebuild, directory re-scan, search deep-clone, per-page backlink grep) is reasoned, never measured, and the existing benches cap at 500 files / 8 tags. Check: generate a 10k-file/500-tag repo and run `cargo bench --no-default-features --bench repo_scan --bench search`, plus `hyperfine 'curl -s localhost:PORT/.mbr/site.json'`.
- **"Static build output is non-deterministic (site.json ordering)"** — asserted from source. Check: `cargo run -- -b --output /tmp/a repo && cargo run -- -b --output /tmp/b repo && diff -r /tmp/a /tmp/b`.
- **perf-frontend's 3.86 MB site.json / 25 ms derivation** come from a synthetic Node script, not from mbr. Check: build a real large repo and `ls -l build/.mbr/site.json`, then `performance.now()` around `shared.ts`'s parse.
- **"mbr-search's lazy media-browser import is defeated by `inlineDynamicImports`"** and all bundle-size claims — derived from vite config, never from an emitted bundle. Check: `cd components && bun install && bun run build && ls -l dist/*.js && rg -c 'mbr-media-browser' dist/mbr-components.min.js`.
- **Pagefind excerpt escaping** (the one surviving `unsafeHTML`, mbr-search.ts:565) and **family-chart `setCardDisplay` innerHTML** were both dropped for missing `node_modules`. Check: `bun install`, then `rg -n '\.html\(|innerHTML' components/node_modules/family-chart/dist`, and run `npx pagefind` over a fixture whose heading contains `<img src=x onerror=alert(1)>` and inspect `data().excerpt`.
- **Dependency advisories were never actually scanned** (sec-supply's `cargo audit` failed to clone the DB). Check: `cargo audit` / `cargo deny check advisories` in CI, plus `npm audit --prefix components`.
- **The Windows tag-page containment finding** is std-path inference, never executed. Check: add a `#[cfg(windows)]` unit test asserting `..\\..\\evil` is rejected, and let the existing `windows` CI job run it.
- **bug-parsing's panic on comment-only frontmatter** was proven in a standalone probe crate, not in mbr. Check: add `#[test]` with `---\n# c\n---\n` to `src/markdown.rs` and run `cargo test`.
- **All test-derived "no test covers X" claims are grep-based.** Check: for each, delete/invert the implicated branch and confirm `cargo test` still passes (a 5-minute manual mutation check).
- **Config precedence** (docs say env wins, code says config.toml wins) — untested either way. Check: set `MBR_PORT=1234` with `port = 5678` in `.mbr/config.toml` and assert which binds.
- **bug-media's dismissal of oembed slow-loris** rests on assumed reqwest 0.13 semantics. Check: `nc -l` a server that dribbles bytes forever and confirm the request dies at `oembed_timeout_ms`.
- **QuickLook file-read** — check by previewing a `.md` containing `<script>fetch('mbrfile:///etc/passwd').then(r=>r.text()).then(t=>document.body.textContent=t)</script>` in Finder.

---

## Refuted and dropped during verification

- **Windows release binaries built without --locked while all Nix artifacts use it** (.github/workflows/release.yml:248)
  - Why dropped: The literal code claim is accurate but the security impact is blocked by the workflow's job-dependency graph.

What I checked:
- `.github/workflows/release.yml:247-248` does contain the quoted `- name: Build release binary` / `run: cargo build --release --no-default-features --features gui` with no `--locked`. Same for `.github/workflows/ci.yml:275/278/281`. So the evidence exists.
- `flake.nix:346` (`cargoArtifacts`), `flake.nix:396` (`mbr-quicklook-staticlib`), `flake.nix:472` (`mbr-cli`) do pass `--locked` via `cargoExtraArgs`, and `.#release` is built from `packages.mbr-cli` (`flake.nix:665/696/720`). So drift genuinely fails the Nix jobs — as the finder concedes.

The guard that refutes the stated failure scenario:
- `.github/workflows/release.yml:311-313`: the `release` job (the only job that publishes anything, generates `SHA256SUMS` at line 340, and calls `softprops/action-gh-release` at line 401) declares `needs: [validate-version, build, build-windows, package-dmg]` with no `if: always()`. GitHub Actions skips a job whose `needs` did not all succeed. `fail-fast: false` on the `build` matrix (line 167) only prevents cancelling sibling matrix legs; the job result is still failure.
- So on Cargo.toml/Cargo.lock drift, the Nix `build` job (`nix build .#release`, line 199 — offline sandbox, vendored from Cargo.lock, `--locked`) fails on the same checkout, `release` is skipped, and no `mbr-windows-*.zip` and no `SHA256SUMS` are ever published. The finding's concrete harm ("ship mbr-windows-x86_64.zip … the SHA256SUMS published in the release then attest to an artifact that cannot be reproduced") cannot occur.

Two further weakenings:
- Cargo.lock is tracked (`git ls-files Cargo.lock`) and the lockfile is target-independent, so a Windows-only dependency bump still trips `--locked` on the Linux/macOS Nix jobs; there is no drift that Windows sees and Nix misses.
- `.github/dependabot.yml:11-14` uses the `cargo` ecosystem, which regenerates `Cargo.lock` alongside `Cargo.toml`, so the primary automated bump path cannot produce the drift at all.
- The finding's premise that "every Nix-built artifact passes `--locked`" is also imprecise: `release.yml:122` (`nix develop --command cargo test --all-features`) and `ci.yml:222/225` run plain, unlocked cargo inside a networked dev shell.
- Without drift, `cargo build` reuses the committed lock verbatim; cargo does a minimal update only when the manifest is unsatisfiable, so "silently re-resolve to the newest semver-compatible versions" overstates the behavior.

Adding `--locked` on the Windows steps is still reasonable consistency hygiene (it would make the Windows leg fail identically instead of relying on a sibling job), but it is a style/defense-in-depth nit, not a reachable supply-chain defect: the published-artifact scenario is already gated at release.yml:313.
