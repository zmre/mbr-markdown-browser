//! Path resolution logic for the mbr server.
//!
//! This module contains pure functions for determining what resource to serve
//! based on a URL path. By keeping this logic separate from I/O, it becomes
//! easily testable. Every filesystem question it asks goes through a
//! [`Vault`]: `stat` for the probes, [`resolve_under`] for containment.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::vault::{EntryKind, Vault, VaultPath, has_hidden_segment, resolve_under};

/// Safely resolves a request path inside the vault, preventing path traversal.
///
/// Returns `None` if the path is not a valid [`VaultPath`] (it climbs above
/// the root, or is absolute), resolves outside the vault once symlinks are
/// followed, or is **hidden** — see [`visible`]. See [`resolve_under`] for
/// the exact containment answers.
///
/// # Security
///
/// This function guards against path traversal attacks by:
/// 1. Normalizing the request lexically, refusing any `..` that would climb
///    above the root
/// 2. Canonicalizing the joined path inside the vault
/// 3. Verifying the resolved path is still inside the vault
/// 4. Refusing a hidden request *and* a hidden resolution
fn safe_join(vault: &dyn Vault, request_path: &str, exempt: &[VaultPath]) -> Option<VaultPath> {
    let request = visible(VaultPath::new(request_path).ok()?, exempt)?;
    visible(resolve_under(vault, &VaultPath::root(), &request)?, exempt)
}

/// `path`, unless it has a hidden segment the user did not name
/// ([`has_hidden_segment`]).
///
/// Hidden files are never served — not from the repository, the static
/// overlay or a symlink mount — because dot paths are where a repository keeps
/// credentials and tooling state (`.env`, `.git/config`). Checked on the
/// request (`/.env`) and again on what it resolved to, so a link that is not
/// itself hidden cannot reach a hidden target (`notes -> .git`). `/.mbr/*` never
/// gets here: those are routes, matched before the page handler.
fn visible(path: VaultPath, exempt: &[VaultPath]) -> Option<VaultPath> {
    (!has_hidden_segment(&path, exempt)).then_some(path)
}

/// The result of resolving a URL path to a resource.
///
/// Paths are index keys ([`Vault::key`]) — for a [`crate::vault::LocalVault`],
/// canonical filesystem paths, as they have always been.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedPath {
    /// Serve a static file directly (non-markdown)
    StaticFile(PathBuf),
    /// Render a markdown file
    MarkdownFile(PathBuf),
    /// Generate a directory listing
    DirectoryListing(PathBuf),
    /// Render a tag page listing all pages with this tag
    TagPage {
        /// The tag source (e.g., "tags", "performers", "taxonomy.tags")
        source: String,
        /// The normalized tag value (e.g., "rust", "joshua_jay")
        value: String,
    },
    /// Render a tag source index listing all tags from this source
    TagSourceIndex {
        /// The tag source (e.g., "tags", "performers")
        source: String,
    },
    /// Resource not found
    NotFound,
    /// Redirect to canonical URL (e.g., /x/index/ → /x/)
    Redirect(String),
}

/// Configuration for path resolution.
#[derive(Debug, Clone, Copy)]
pub struct PathResolverConfig<'a> {
    /// The repository ([`crate::repo::Repo::vault`]).
    pub vault: &'a dyn Vault,
    /// The external static overlay ([`crate::repo::Repo::static_vault`]), when
    /// `static_folder` resolves outside the repository. Takes precedence over
    /// `static_folder`, which then only names it.
    pub static_vault: Option<&'a dyn Vault>,
    /// The configured static folder. Without a `static_vault`, it is placed
    /// inside `vault` ([`crate::vault::configured_folder`]); a value that lands
    /// nowhere in the vault disables the overlay.
    pub static_folder: &'a str,
    pub markdown_extensions: &'a [String],
    pub index_file: &'a str,
    /// Valid tag source URL identifiers (e.g., ["tags", "performers", "taxonomy.tags"])
    /// Used to detect tag page URLs like /tags/rust/
    pub tag_sources: &'a [String],
    /// Hidden directories that may be served anyway, as vault paths: the ones
    /// the user named on the command line (`Config::explicit_hidden_dirs`,
    /// [`crate::repo::Repo::exempt_hidden_dirs`]). Empty almost always.
    pub exempt_hidden_dirs: &'a [VaultPath],
}

/// Owned counterpart of [`PathResolverConfig`].
///
/// [`PathResolverConfig`] borrows everything, which is right for a request
/// handler but impossible for anything that must outlive the borrow — notably
/// the `'static` closure [`crate::link_transform::LinkTransformConfig`] carries
/// to answer "does this link target resolve to a markdown page?". Owning the
/// values costs a handful of small clones (and two `Arc` bumps) per page render
/// and keeps a single definition of the resolution inputs.
#[derive(Debug, Clone)]
pub struct OwnedPathResolverConfig {
    pub vault: Arc<dyn Vault>,
    pub static_vault: Option<Arc<dyn Vault>>,
    pub static_folder: String,
    pub markdown_extensions: Vec<String>,
    pub index_file: String,
    pub tag_sources: Vec<String>,
    pub exempt_hidden_dirs: Vec<VaultPath>,
}

impl OwnedPathResolverConfig {
    /// Borrows this configuration as the form [`resolve_request_path`] takes.
    pub fn as_config(&self) -> PathResolverConfig<'_> {
        PathResolverConfig {
            vault: self.vault.as_ref(),
            static_vault: self.static_vault.as_deref(),
            static_folder: &self.static_folder,
            markdown_extensions: &self.markdown_extensions,
            index_file: &self.index_file,
            tag_sources: &self.tag_sources,
            exempt_hidden_dirs: &self.exempt_hidden_dirs,
        }
    }

    /// The configuration for `repo`'s storage — its vault and static overlay —
    /// with the given resolution settings.
    pub fn for_repo(
        repo: &crate::repo::Repo,
        static_folder: &str,
        markdown_extensions: &[String],
        index_file: &str,
        tag_sources: Vec<String>,
    ) -> Self {
        Self {
            vault: Arc::clone(repo.vault()),
            static_vault: repo.static_vault().cloned(),
            static_folder: static_folder.to_string(),
            markdown_extensions: markdown_extensions.to_vec(),
            index_file: index_file.to_string(),
            tag_sources,
            exempt_hidden_dirs: repo.exempt_hidden_dirs().to_vec(),
        }
    }
}

/// Decides whether a request that resolved to a markdown page arrived at a
/// non-canonical URL, and if so what to redirect to.
///
/// Markdown pages are served at directory-style URLs (`docs/guide.md` →
/// `/docs/guide/`). Serving the same page at `/docs/guide` with a 200 is not
/// harmless: the browser's base for resolving that page's own relative links
/// becomes `/docs/` instead of `/docs/guide/`, so every `../`-prefixed href the
/// renderer emitted lands one directory too high and 404s. The defect surfaces
/// one click *after* the wrong URL, which is what makes it so hard to trace.
///
/// `request_path` is the path axum's catch-all captured: percent-decoded and
/// without a leading slash. `canonical_url` is the page's canonical URL as
/// produced by [`crate::repo::build_markdown_url_path`] (leading and trailing
/// slash). Returns `None` when the request already used the canonical URL.
///
/// Note this deliberately also catches `/docs/guide.md`: an extension-bearing
/// URL is just another non-canonical spelling of the same page, and leaving it
/// at a 200 has the identical relative-link consequence.
pub fn canonical_page_redirect(request_path: &str, canonical_url: &str) -> Option<String> {
    let requested = format!("/{}", request_path.trim_start_matches('/'));
    (requested != canonical_url).then(|| canonical_url.to_string())
}

/// Normalizes an authored link target (href) into the request-path form that
/// [`resolve_request_path`] expects.
///
/// Live requests reach the server through axum's `extract::Path`, which
/// percent-decodes the URL path before `resolve_request_path` ever sees it.
/// Any code that feeds *authored* hrefs (still percent-encoded, possibly
/// carrying fragments or query strings) into the resolver must apply this
/// identical normalization, or its results diverge from what the server
/// actually serves. That divergence previously caused bogus "broken internal
/// link" 404 reports in the GUI error panel for links like
/// `/IronCore%20Swag%20T-shirts%20Gifts/` pointing at
/// `IronCore Swag T-shirts Gifts.md`.
///
/// Mirrors exactly what a real HTTP request undergoes:
/// 1. Strip the fragment (`#...`), then the query string (`?...`).
/// 2. Percent-decode (lossy UTF-8, matching axum's decoding).
/// 3. Trim leading and trailing `/`.
///
/// Note: valid percent escapes are always decoded, so a literal `%` must be
/// authored as `%25` (e.g. `100%25` normalizes to `100%`).
pub fn normalize_link_target(href: &str) -> String {
    let base = href.split('#').next().unwrap_or(href);
    let base = base.split('?').next().unwrap_or(base);
    let decoded = percent_encoding::percent_decode_str(base).decode_utf8_lossy();
    decoded.trim_matches('/').to_string()
}

/// Resolves a URL path to determine what resource should be served.
///
/// This is a pure function that performs filesystem checks (through
/// `config.vault`) but no I/O operations like reading file contents. It
/// determines the type of resource to serve.
///
/// # Resolution Order
///
/// 1. Direct file match in base_dir → StaticFile
/// 2. Directory with configured index file (e.g., index.md) → MarkdownFile
/// 3. Path with trailing slash matching a markdown file (e.g., /foo/ → foo.md) → MarkdownFile
/// 4. File in static folder → StaticFile
/// 5. Directory with index.{markdown_ext} → MarkdownFile
/// 6. Directory without index → DirectoryListing
/// 7. Tag source index (e.g., /tags/) → TagSourceIndex (if source matches config)
/// 8. Tag page (e.g., /tags/rust/) → TagPage (if source matches config)
/// 9. Nothing matches → NotFound
///
/// Note: Filesystem paths (steps 1-6) always take precedence over tag URLs (steps 7-8).
/// If a file or directory named "tags" exists, it will be served instead of the tag index.
///
/// # Security
///
/// Path traversal attacks (e.g., `../../../etc/passwd`) are blocked by validating
/// that all resolved paths remain within the configured base directory.
pub fn resolve_request_path(config: &PathResolverConfig, request_path: &str) -> ResolvedPath {
    let vault = config.vault;
    // Step 4 and step 4b ask the same question; answer it at most once.
    let mut static_checked = false;

    // Use safe_join to prevent path traversal attacks
    // If the path would escape base_dir, skip to tag resolution or NotFound
    if let Some(candidate) = safe_join(vault, request_path, config.exempt_hidden_dirs) {
        // One `stat` answers both "is it a file" and "is it a directory".
        let kind = vault
            .stat(&candidate)
            .ok()
            .flatten()
            .map(|entry| entry.kind);

        // 1. Direct file match
        if kind == Some(EntryKind::File) {
            let name = Path::new(candidate.file_name().unwrap_or_default());
            let key = vault.key(&candidate);
            return if is_markdown_file(name, config.markdown_extensions) {
                ResolvedPath::MarkdownFile(key)
            } else {
                ResolvedPath::StaticFile(key)
            };
        }

        // 2. Directory with configured index file
        if kind == Some(EntryKind::Dir)
            && let Ok(index_path) = candidate.join(config.index_file)
            && vault.is_file(&index_path)
        {
            return ResolvedPath::MarkdownFile(vault.key(&index_path));
        }

        // 3a. Check for non-canonical index URL (e.g., /x/index/ should redirect to /x/)
        // This must come before step 3 to catch URLs like /docs/index/ before they resolve
        let index_stem = Path::new(config.index_file)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("index");

        if candidate.file_name() == Some(index_stem)
            && let Some(parent) = candidate.parent()
            && let Ok(index_path) = parent.join(config.index_file)
            && vault.is_file(&index_path)
        {
            // Build canonical URL: /x/index/ → /x/. The parent is already the
            // canonical vault path, which is `/`-separated on every platform.
            let canonical = if parent.is_root() {
                "/".to_string()
            } else {
                format!("/{parent}/")
            };
            return ResolvedPath::Redirect(canonical);
        }

        // 3. Try markdown extensions on base path (for /foo/ → foo.md)
        if let Some(md_path) = find_markdown_file(vault, &candidate, config.markdown_extensions) {
            return ResolvedPath::MarkdownFile(vault.key(&md_path));
        }

        // 4. Check static folder (has its own path traversal protection)
        static_checked = true;
        if let Some(static_path) = find_in_static_folder(config, request_path) {
            return ResolvedPath::StaticFile(static_path);
        }

        // 5. Directory with index.{markdown_ext}
        if kind == Some(EntryKind::Dir) {
            if let Ok(index_base) = candidate.join("index")
                && let Some(md_path) =
                    find_markdown_file(vault, &index_base, config.markdown_extensions)
            {
                return ResolvedPath::MarkdownFile(vault.key(&md_path));
            }

            // 6. Directory without index → listing
            return ResolvedPath::DirectoryListing(vault.key(&candidate));
        }
    }

    // 4b. Static folder check - ALSO check here for paths not in base_dir
    // This handles the case where the path doesn't exist in base_dir but exists in static folder
    // (e.g., /images/blog/photo.png where images/ only exists under static/)
    if !static_checked && let Some(static_path) = find_in_static_folder(config, request_path) {
        return ResolvedPath::StaticFile(static_path);
    }

    // 7-8. Check for tag URLs (only if nothing matched in filesystem)
    // This is also reached if safe_join returned None (path traversal blocked)
    if let Some(tag_result) = try_resolve_tag_url(request_path, config.tag_sources) {
        return tag_result;
    }

    // 9. Nothing found
    ResolvedPath::NotFound
}

/// Checks if a path is a markdown file based on configured extensions.
fn is_markdown_file(path: &Path, extensions: &[String]) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| extensions.iter().any(|md_ext| md_ext == ext))
        .unwrap_or(false)
}

/// Strips trailing path separator(s) from a path.
///
/// Both `/` and the platform separator are trimmed. The input here is derived
/// from a request URL, where the separator is *always* `/` regardless of
/// platform — trimming only `std::path::MAIN_SEPARATOR` silently did nothing on
/// Windows, leaving a trailing slash on the candidate path.
///
/// The resolver itself no longer needs this — a [`VaultPath`] never carries a
/// trailing separator — but it remains the one definition for callers that
/// still hold native paths.
pub fn strip_trailing_separator(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    let trimmed = s.trim_end_matches(['/', std::path::MAIN_SEPARATOR]);
    PathBuf::from(trimmed)
}

/// Finds a markdown file by trying each configured extension.
///
/// The URL for a markdown file strips only its final extension (see
/// `build_markdown_url_path`), so a file named `a.b.c.md` is served at `/a.b.c/`.
/// We therefore reverse that by *appending* the extension to the full stem;
/// `Path::set_extension` would instead replace the trailing dotted segment
/// (`a.b.c` -> `a.b.md`) and 404 on any file whose name contains a dot.
///
/// The vault root has no name, so it never probes for a sibling of the
/// repository (`<parent>/<repo>.md`), which the path-based version did.
fn find_markdown_file(
    vault: &dyn Vault,
    base_path: &VaultPath,
    extensions: &[String],
) -> Option<VaultPath> {
    let file_name = base_path.file_name()?;
    let parent = base_path.parent()?;
    extensions
        .iter()
        .filter_map(|ext| parent.child(&format!("{file_name}.{ext}")).ok())
        .find(|path| vault.is_file(path))
}

/// Finds a file in the static folder.
///
/// # Security
///
/// Containment is enforced against the **static root** — the canonicalized
/// static directory itself — not against the repository root. The overlay is
/// allowed to live outside the repository root (`static_folder = "../static"`
/// for the common `repo/content` + `repo/static` layout), so requiring every
/// served file to sit under the repository root would 404 the entire overlay.
/// An overlay outside the root is a vault of its own (`config.static_vault`);
/// one inside it is a folder of `config.vault`.
///
/// *How far* the overlay may reach is decided once, at load time, by
/// `Config::validate_static_folder`: inside the root, or under a directory at
/// most two levels above it, never reached through `$HOME` or the filesystem
/// root, and never a directory containing the root. What this function must
/// still guarantee — and does — is that a *request path* cannot walk out of
/// whatever directory that policy settled on, including through a symlink inside
/// the overlay pointing at, say, `/etc/passwd`: the candidate is canonicalized
/// before the containment check, so the symlink's target is what gets judged.
fn find_in_static_folder(config: &PathResolverConfig, request_path: &str) -> Option<PathBuf> {
    let request = visible(VaultPath::new(request_path).ok()?, &[])?;
    let (vault, static_dir) = match config.static_vault {
        Some(overlay) => (overlay, VaultPath::root()),
        None => (
            config.vault,
            crate::vault::configured_folder(config.vault, config.static_folder)?,
        ),
    };

    // Canonicalize the static directory (it must exist) and the candidate
    // (which must exist too — unlike `resolve_under`, there is no "probe a
    // name that is not there yet" here, so a miss costs no second lookup),
    // then require the candidate to still be inside the former.
    let static_dir = vault.canonicalize(&static_dir).ok()?;
    let canonical = vault.canonicalize(&static_dir.join_path(&request)).ok()?;
    (canonical.starts_with(&static_dir)
        && !hidden_below(&canonical, &static_dir)
        && vault.is_file(&canonical))
    .then(|| vault.key(&canonical))
}

/// Whether `path` has a hidden segment below `base` (which it starts with).
///
/// The static folder is judged from *inside*: the folder itself is whatever
/// the configuration named — an in-root `.vuepress/public` is a real layout —
/// while everything served out of it answers to the hidden-file rule. (An
/// *external* overlay that is itself hidden is refused when the configuration
/// is validated, by [`crate::config::external_folder_refusal`].)
fn hidden_below(path: &VaultPath, base: &VaultPath) -> bool {
    path.segments()
        .skip(base.segments().count())
        .any(crate::vault::is_hidden_segment)
}

/// Attempts to resolve a URL path as a tag URL.
///
/// Matches patterns like:
/// - `{source}/` → TagSourceIndex (e.g., "tags/" → list all tags)
/// - `{source}/{value}/` → TagPage (e.g., "tags/rust/" → pages tagged "rust")
///
/// The source must match one of the configured tag sources (case-insensitive).
/// Returns `None` if the path doesn't match a tag URL pattern.
fn try_resolve_tag_url(request_path: &str, tag_sources: &[String]) -> Option<ResolvedPath> {
    // Skip if no tag sources configured
    if tag_sources.is_empty() {
        return None;
    }

    // Normalize path: strip leading and trailing slashes
    let path = request_path.trim_matches('/');

    // Empty path is not a tag URL
    if path.is_empty() {
        return None;
    }

    // Split path into segments
    let segments: Vec<&str> = path.split('/').collect();

    match segments.len() {
        // Single segment: might be a tag source index (e.g., "tags")
        1 => {
            let source = segments[0].to_lowercase();
            if tag_sources.iter().any(|s| s.to_lowercase() == source) {
                Some(ResolvedPath::TagSourceIndex { source })
            } else {
                None
            }
        }
        // Two segments: might be a tag page (e.g., "tags/rust")
        2 => {
            let source = segments[0].to_lowercase();
            let value = segments[1].to_lowercase();

            // Don't match empty values
            if value.is_empty() {
                return None;
            }

            if tag_sources.iter().any(|s| s.to_lowercase() == source) {
                Some(ResolvedPath::TagPage { source, value })
            } else {
                None
            }
        }
        // More than 2 segments: not a tag URL
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::LocalVault;
    use std::fs;
    use tempfile::TempDir;

    /// `safe_join` on a local vault rooted at `base`, answered as the key
    /// (canonical path) it resolves to — the shape the path-based `safe_join`
    /// returned, so the security tests below assert exactly what they did.
    fn safe_join_local(base: &Path, request: &str) -> Option<PathBuf> {
        let vault = LocalVault::new(base);
        safe_join(&vault, request, &[]).map(|p| vault.key(&p))
    }

    /// The overlay vault the repository would build for `static_folder`, using
    /// the same policy call `Repo::init` makes.
    fn overlay_vault(root: &Path, static_folder: &str) -> Option<LocalVault> {
        match crate::config::resolve_static_overlay(root, static_folder) {
            Ok(crate::config::StaticOverlay::External(dir)) => Some(LocalVault::new(dir)),
            _ => None,
        }
    }

    /// Test fixture that owns the extensions and tag_sources vectors
    struct TestFixture {
        dir: TempDir,
        vault: LocalVault,
        extensions: Vec<String>,
        tag_sources: Vec<String>,
    }

    impl TestFixture {
        fn new() -> Self {
            let dir = TempDir::new().unwrap();
            fs::create_dir(dir.path().join("static")).unwrap();
            let vault = LocalVault::new(dir.path());
            Self {
                dir,
                vault,
                extensions: vec![String::from("md")],
                tag_sources: vec![],
            }
        }

        fn with_extensions(extensions: Vec<String>) -> Self {
            let dir = TempDir::new().unwrap();
            fs::create_dir(dir.path().join("static")).unwrap();
            let vault = LocalVault::new(dir.path());
            Self {
                dir,
                vault,
                extensions,
                tag_sources: vec![],
            }
        }

        fn with_tag_sources(tag_sources: Vec<String>) -> Self {
            let dir = TempDir::new().unwrap();
            fs::create_dir(dir.path().join("static")).unwrap();
            let vault = LocalVault::new(dir.path());
            Self {
                dir,
                vault,
                extensions: vec![String::from("md")],
                tag_sources,
            }
        }

        fn config(&self) -> PathResolverConfig<'_> {
            PathResolverConfig {
                vault: &self.vault,
                static_vault: None,
                static_folder: "static",
                markdown_extensions: &self.extensions,
                index_file: "index.md",
                tag_sources: &self.tag_sources,
                exempt_hidden_dirs: &[],
            }
        }

        fn path(&self) -> &Path {
            self.dir.path()
        }

        /// Returns the canonicalized base path (resolves symlinks like /var -> /private/var on macOS)
        fn canonical_path(&self) -> PathBuf {
            self.dir.path().canonicalize().unwrap()
        }
    }

    #[test]
    fn test_direct_markdown_file() {
        let fixture = TestFixture::new();
        fs::write(fixture.path().join("readme.md"), "# Test").unwrap();

        let result = resolve_request_path(&fixture.config(), "readme.md");

        // safe_join returns canonicalized paths
        assert_eq!(
            result,
            ResolvedPath::MarkdownFile(fixture.canonical_path().join("readme.md"))
        );
    }

    #[test]
    fn test_direct_static_file() {
        let fixture = TestFixture::new();
        fs::write(fixture.path().join("image.png"), "fake image").unwrap();

        let result = resolve_request_path(&fixture.config(), "image.png");

        // safe_join returns canonicalized paths
        assert_eq!(
            result,
            ResolvedPath::StaticFile(fixture.canonical_path().join("image.png"))
        );
    }

    #[test]
    fn test_directory_with_index() {
        let fixture = TestFixture::new();
        let subdir = fixture.path().join("docs");
        fs::create_dir(&subdir).unwrap();
        fs::write(subdir.join("index.md"), "# Docs").unwrap();

        let result = resolve_request_path(&fixture.config(), "docs");

        // safe_join returns canonicalized paths
        let expected = fixture.canonical_path().join("docs/index.md");
        assert_eq!(result, ResolvedPath::MarkdownFile(expected));
    }

    #[test]
    fn test_trailing_slash_to_markdown() {
        let fixture = TestFixture::new();
        fs::write(fixture.path().join("about.md"), "# About").unwrap();

        let result = resolve_request_path(&fixture.config(), "about/");

        // safe_join returns canonicalized paths
        assert_eq!(
            result,
            ResolvedPath::MarkdownFile(fixture.canonical_path().join("about.md"))
        );
    }

    #[test]
    fn test_dotted_filename_trailing_slash_to_markdown() {
        // Regression: a file whose name contains a period is served at a URL that
        // strips only the final extension (see build_markdown_url_path). The resolver
        // must reverse that by appending the extension, not replacing the trailing
        // dotted segment, otherwise `patrick-walsh-b.2010-03-03.md` 404s at
        // `/patrick-walsh-b.2010-03-03/`.
        let fixture = TestFixture::new();
        fs::write(
            fixture.path().join("patrick-walsh-b.2010-03-03.md"),
            "# Patrick",
        )
        .unwrap();

        let result = resolve_request_path(&fixture.config(), "patrick-walsh-b.2010-03-03/");

        assert_eq!(
            result,
            ResolvedPath::MarkdownFile(
                fixture
                    .canonical_path()
                    .join("patrick-walsh-b.2010-03-03.md")
            )
        );
    }

    #[test]
    fn test_dotted_filename_without_trailing_slash_to_markdown() {
        // Mirrors test_trailing_slash_to_markdown semantics: a dotted filename must
        // also resolve when requested WITHOUT the trailing slash.
        let fixture = TestFixture::new();
        fs::write(
            fixture.path().join("patrick-walsh-b.2010-03-03.md"),
            "# Patrick",
        )
        .unwrap();

        let result = resolve_request_path(&fixture.config(), "patrick-walsh-b.2010-03-03");

        assert_eq!(
            result,
            ResolvedPath::MarkdownFile(
                fixture
                    .canonical_path()
                    .join("patrick-walsh-b.2010-03-03.md")
            )
        );
    }

    #[test]
    fn test_multi_dot_filename_trailing_slash_to_markdown() {
        // A filename with multiple interior dots must resolve at its canonical URL
        // (`report.2024.final.md` -> `/report.2024.final/`).
        let fixture = TestFixture::new();
        fs::write(fixture.path().join("report.2024.final.md"), "# Report").unwrap();

        let result = resolve_request_path(&fixture.config(), "report.2024.final/");

        assert_eq!(
            result,
            ResolvedPath::MarkdownFile(fixture.canonical_path().join("report.2024.final.md"))
        );
    }

    #[test]
    fn test_static_folder_file() {
        let fixture = TestFixture::new();
        fs::write(fixture.path().join("static/style.css"), "body {}").unwrap();

        let result = resolve_request_path(&fixture.config(), "style.css");

        // The static file path is canonicalized
        let expected = fixture
            .path()
            .join("static/style.css")
            .canonicalize()
            .unwrap();
        assert_eq!(result, ResolvedPath::StaticFile(expected));
    }

    #[test]
    fn test_static_folder_nested_path() {
        let fixture = TestFixture::new();
        fs::create_dir_all(fixture.path().join("static/images/blog")).unwrap();
        fs::write(
            fixture.path().join("static/images/blog/photo.png"),
            "fake image",
        )
        .unwrap();

        // Request for /images/blog/photo.png should find static/images/blog/photo.png
        let result = resolve_request_path(&fixture.config(), "images/blog/photo.png");

        let expected = fixture
            .path()
            .join("static/images/blog/photo.png")
            .canonicalize()
            .unwrap();
        assert_eq!(result, ResolvedPath::StaticFile(expected));
    }

    #[test]
    fn test_directory_listing() {
        let fixture = TestFixture::new();
        let subdir = fixture.path().join("posts");
        fs::create_dir(&subdir).unwrap();
        // No index file

        let result = resolve_request_path(&fixture.config(), "posts/");

        // safe_join returns canonicalized paths
        let expected = fixture.canonical_path().join("posts");
        assert_eq!(result, ResolvedPath::DirectoryListing(expected));
    }

    #[test]
    fn test_not_found() {
        let fixture = TestFixture::new();

        let result = resolve_request_path(&fixture.config(), "nonexistent");

        assert_eq!(result, ResolvedPath::NotFound);
    }

    #[test]
    fn test_nested_directory_with_index() {
        let fixture = TestFixture::new();
        let nested = fixture.path().join("blog/2024");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("index.md"), "# Blog 2024").unwrap();

        let result = resolve_request_path(&fixture.config(), "blog/2024");

        // safe_join returns canonicalized paths
        let expected = fixture.canonical_path().join("blog/2024/index.md");
        assert_eq!(result, ResolvedPath::MarkdownFile(expected));
    }

    #[test]
    fn test_multiple_markdown_extensions() {
        let fixture =
            TestFixture::with_extensions(vec![String::from("md"), String::from("markdown")]);
        fs::write(fixture.path().join("notes.markdown"), "# Notes").unwrap();

        let result = resolve_request_path(&fixture.config(), "notes/");

        // safe_join returns canonicalized paths
        assert_eq!(
            result,
            ResolvedPath::MarkdownFile(fixture.canonical_path().join("notes.markdown"))
        );
    }

    #[test]
    fn test_prefers_first_extension() {
        let fixture =
            TestFixture::with_extensions(vec![String::from("md"), String::from("markdown")]);
        // Create both .md and .markdown files
        fs::write(fixture.path().join("test.md"), "# MD").unwrap();
        fs::write(fixture.path().join("test.markdown"), "# Markdown").unwrap();

        let result = resolve_request_path(&fixture.config(), "test/");

        // Should prefer .md (first in list), safe_join returns canonicalized paths
        assert_eq!(
            result,
            ResolvedPath::MarkdownFile(fixture.canonical_path().join("test.md"))
        );
    }

    #[test]
    fn test_root_path_empty_string() {
        let fixture = TestFixture::new();
        fs::write(fixture.path().join("index.md"), "# Home").unwrap();

        let result = resolve_request_path(&fixture.config(), "");

        // Empty path resolves to base_dir, which is a directory with index.md
        // safe_join returns canonicalized paths
        assert_eq!(
            result,
            ResolvedPath::MarkdownFile(fixture.canonical_path().join("index.md"))
        );
    }

    #[test]
    fn test_is_markdown_file() {
        let extensions = vec![String::from("md"), String::from("markdown")];

        assert!(is_markdown_file(Path::new("test.md"), &extensions));
        assert!(is_markdown_file(Path::new("test.markdown"), &extensions));
        assert!(!is_markdown_file(Path::new("test.txt"), &extensions));
        assert!(!is_markdown_file(Path::new("test"), &extensions));
    }

    #[test]
    fn test_strip_trailing_separator() {
        // A forward slash must be stripped on every platform: this function's
        // input comes from a request URL, so `/` is the separator even where
        // `std::path::MAIN_SEPARATOR` is `\`.
        assert_eq!(
            strip_trailing_separator(Path::new("/foo/bar/")),
            PathBuf::from("/foo/bar")
        );
        assert_eq!(
            strip_trailing_separator(Path::new("/foo/bar")),
            PathBuf::from("/foo/bar")
        );
        assert_eq!(
            strip_trailing_separator(Path::new("relative/")),
            PathBuf::from("relative")
        );
        // Repeated separators are all removed.
        assert_eq!(
            strip_trailing_separator(Path::new("/foo/bar//")),
            PathBuf::from("/foo/bar")
        );
    }

    /// The platform separator is stripped too, so a natively-joined path with a
    /// trailing separator is handled the same way as a URL-derived one.
    #[cfg(windows)]
    #[test]
    fn test_strip_trailing_separator_windows_backslash() {
        assert_eq!(
            strip_trailing_separator(Path::new(r"\foo\bar\")),
            PathBuf::from(r"\foo\bar")
        );
        // Mixed separators, which Windows accepts in real request handling.
        assert_eq!(
            strip_trailing_separator(Path::new(r"\foo\bar/")),
            PathBuf::from(r"\foo\bar")
        );
    }

    // ==================== normalize_link_target Tests ====================

    #[test]
    fn test_normalize_link_target_plain_path() {
        assert_eq!(normalize_link_target("docs/guide"), "docs/guide");
    }

    #[test]
    fn test_normalize_link_target_decodes_encoded_spaces() {
        assert_eq!(
            normalize_link_target("/IronCore%20Swag%20T-shirts%20Gifts"),
            "IronCore Swag T-shirts Gifts"
        );
    }

    #[test]
    fn test_normalize_link_target_strips_fragment() {
        assert_eq!(normalize_link_target("/docs/guide/#section"), "docs/guide");
    }

    #[test]
    fn test_normalize_link_target_strips_query() {
        assert_eq!(normalize_link_target("/docs/guide/?x=1&y=2"), "docs/guide");
    }

    #[test]
    fn test_normalize_link_target_strips_query_and_fragment_with_decoding() {
        assert_eq!(
            normalize_link_target("/My%20Page/?x=1#top"),
            "My Page",
            "fragment and query must be stripped before decoding/trimming"
        );
    }

    #[test]
    fn test_normalize_link_target_trims_leading_and_trailing_slashes() {
        assert_eq!(normalize_link_target("/docs/guide/"), "docs/guide");
        assert_eq!(normalize_link_target("docs/guide"), "docs/guide");
        assert_eq!(normalize_link_target("/"), "");
    }

    #[test]
    fn test_normalize_link_target_decodes_literal_percent_escape() {
        // A literal `%` must be authored as `%25`; valid escapes always decode.
        assert_eq!(normalize_link_target("/100%25"), "100%");
    }

    // ==================== Tag URL Resolution Tests ====================

    #[test]
    fn test_tag_source_index() {
        let fixture = TestFixture::with_tag_sources(vec!["tags".to_string()]);
        let result = resolve_request_path(&fixture.config(), "tags/");

        assert_eq!(
            result,
            ResolvedPath::TagSourceIndex {
                source: "tags".to_string()
            }
        );
    }

    #[test]
    fn test_tag_source_index_without_trailing_slash() {
        let fixture = TestFixture::with_tag_sources(vec!["tags".to_string()]);
        let result = resolve_request_path(&fixture.config(), "tags");

        assert_eq!(
            result,
            ResolvedPath::TagSourceIndex {
                source: "tags".to_string()
            }
        );
    }

    #[test]
    fn test_tag_page() {
        let fixture = TestFixture::with_tag_sources(vec!["tags".to_string()]);
        let result = resolve_request_path(&fixture.config(), "tags/rust/");

        assert_eq!(
            result,
            ResolvedPath::TagPage {
                source: "tags".to_string(),
                value: "rust".to_string()
            }
        );
    }

    #[test]
    fn test_tag_page_without_trailing_slash() {
        let fixture = TestFixture::with_tag_sources(vec!["tags".to_string()]);
        let result = resolve_request_path(&fixture.config(), "tags/rust");

        assert_eq!(
            result,
            ResolvedPath::TagPage {
                source: "tags".to_string(),
                value: "rust".to_string()
            }
        );
    }

    #[test]
    fn test_tag_url_case_insensitive_source() {
        let fixture = TestFixture::with_tag_sources(vec!["Tags".to_string()]);

        // Uppercase in URL should match lowercase config
        let result = resolve_request_path(&fixture.config(), "TAGS/rust/");

        assert_eq!(
            result,
            ResolvedPath::TagPage {
                source: "tags".to_string(),
                value: "rust".to_string()
            }
        );
    }

    #[test]
    fn test_tag_url_unknown_source_not_matched() {
        let fixture = TestFixture::with_tag_sources(vec!["tags".to_string()]);

        // "categories" is not a configured tag source
        let result = resolve_request_path(&fixture.config(), "categories/rust/");

        assert_eq!(result, ResolvedPath::NotFound);
    }

    #[test]
    fn test_tag_url_no_sources_configured() {
        let fixture = TestFixture::new(); // Empty tag_sources
        let result = resolve_request_path(&fixture.config(), "tags/rust/");

        assert_eq!(result, ResolvedPath::NotFound);
    }

    #[test]
    fn test_tag_url_multiple_sources() {
        let fixture = TestFixture::with_tag_sources(vec![
            "tags".to_string(),
            "performers".to_string(),
            "taxonomy.categories".to_string(),
        ]);

        // All sources should be recognized
        assert_eq!(
            resolve_request_path(&fixture.config(), "tags/rust/"),
            ResolvedPath::TagPage {
                source: "tags".to_string(),
                value: "rust".to_string()
            }
        );
        assert_eq!(
            resolve_request_path(&fixture.config(), "performers/joshua_jay/"),
            ResolvedPath::TagPage {
                source: "performers".to_string(),
                value: "joshua_jay".to_string()
            }
        );
        assert_eq!(
            resolve_request_path(&fixture.config(), "taxonomy.categories/"),
            ResolvedPath::TagSourceIndex {
                source: "taxonomy.categories".to_string()
            }
        );
    }

    #[test]
    fn test_file_takes_precedence_over_tag_url() {
        let fixture = TestFixture::with_tag_sources(vec!["tags".to_string()]);
        // Create a real markdown file at "tags.md"
        fs::write(fixture.path().join("tags.md"), "# Real Tags Page").unwrap();

        // File should take precedence over tag source index
        let result = resolve_request_path(&fixture.config(), "tags/");

        // safe_join returns canonicalized paths
        assert_eq!(
            result,
            ResolvedPath::MarkdownFile(fixture.canonical_path().join("tags.md"))
        );
    }

    #[test]
    fn test_directory_takes_precedence_over_tag_url() {
        let fixture = TestFixture::with_tag_sources(vec!["tags".to_string()]);
        // Create a real directory "tags/"
        fs::create_dir(fixture.path().join("tags")).unwrap();

        // Directory listing should take precedence
        let result = resolve_request_path(&fixture.config(), "tags/");

        // safe_join returns canonicalized paths
        assert_eq!(
            result,
            ResolvedPath::DirectoryListing(fixture.canonical_path().join("tags"))
        );
    }

    #[test]
    fn test_nested_tag_value_not_matched() {
        let fixture = TestFixture::with_tag_sources(vec!["tags".to_string()]);

        // More than 2 segments is not a valid tag URL
        let result = resolve_request_path(&fixture.config(), "tags/rust/advanced/");

        assert_eq!(result, ResolvedPath::NotFound);
    }

    #[test]
    fn test_try_resolve_tag_url_directly() {
        let sources = vec!["tags".to_string(), "performers".to_string()];

        // Tag source index
        assert_eq!(
            try_resolve_tag_url("tags/", &sources),
            Some(ResolvedPath::TagSourceIndex {
                source: "tags".to_string()
            })
        );

        // Tag page
        assert_eq!(
            try_resolve_tag_url("tags/rust", &sources),
            Some(ResolvedPath::TagPage {
                source: "tags".to_string(),
                value: "rust".to_string()
            })
        );

        // Unknown source
        assert_eq!(try_resolve_tag_url("unknown/value", &sources), None);

        // Empty path
        assert_eq!(try_resolve_tag_url("", &sources), None);

        // Empty sources
        assert_eq!(try_resolve_tag_url("tags/rust", &[]), None);
    }

    // ==================== Non-Canonical Index URL Redirect Tests ====================

    #[test]
    fn test_non_canonical_index_redirects() {
        let fixture = TestFixture::new();
        let docs = fixture.path().join("docs");
        fs::create_dir(&docs).unwrap();
        fs::write(docs.join("index.md"), "# Docs Index").unwrap();

        // /docs/index/ should redirect to /docs/
        let result = resolve_request_path(&fixture.config(), "docs/index/");
        assert_eq!(result, ResolvedPath::Redirect("/docs/".to_string()));
    }

    #[test]
    fn test_root_index_redirects() {
        let fixture = TestFixture::new();
        fs::write(fixture.path().join("index.md"), "# Home").unwrap();

        // /index/ should redirect to /
        let result = resolve_request_path(&fixture.config(), "index/");
        assert_eq!(result, ResolvedPath::Redirect("/".to_string()));
    }

    #[test]
    fn test_nested_index_redirects() {
        let fixture = TestFixture::new();
        let nested = fixture.path().join("a/b/c");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("index.md"), "# Nested").unwrap();

        // /a/b/c/index/ should redirect to /a/b/c/
        let result = resolve_request_path(&fixture.config(), "a/b/c/index/");
        assert_eq!(result, ResolvedPath::Redirect("/a/b/c/".to_string()));

        // A redirect target is a URL, so it must never carry a platform
        // separator. Asserted explicitly because the equality above only fails
        // on platforms where `\` is the separator.
        let ResolvedPath::Redirect(target) = result else {
            panic!("expected a redirect");
        };
        assert!(
            !target.contains('\\'),
            "redirect target must not contain a backslash, got {target}"
        );
    }

    #[test]
    fn test_regular_file_named_index_no_redirect() {
        let fixture = TestFixture::new();
        // Create a regular file that happens to be named index.md (not in a directory with index)
        fs::write(fixture.path().join("index.md"), "# Regular Index").unwrap();

        // But also create docs/readme.md as a standalone file (no parent index)
        let docs = fixture.path().join("docs");
        fs::create_dir(&docs).unwrap();
        fs::write(docs.join("readme.md"), "# Readme").unwrap();

        // /docs/readme/ should NOT redirect (readme is not the index file)
        let result = resolve_request_path(&fixture.config(), "docs/readme/");
        assert!(matches!(result, ResolvedPath::MarkdownFile(_)));
    }

    #[test]
    fn test_index_without_trailing_slash_redirects() {
        let fixture = TestFixture::new();
        let docs = fixture.path().join("docs");
        fs::create_dir(&docs).unwrap();
        fs::write(docs.join("index.md"), "# Docs Index").unwrap();

        // /docs/index (without trailing slash) should also redirect to /docs/
        let result = resolve_request_path(&fixture.config(), "docs/index");
        assert_eq!(result, ResolvedPath::Redirect("/docs/".to_string()));
    }

    // ==================== Path Traversal Security Tests ====================

    #[test]
    fn test_path_traversal_blocked_with_dotdot() {
        let fixture = TestFixture::new();
        // Create a file outside the temp directory (simulating /etc/passwd)
        // We can't actually create /etc/passwd, so we test that path traversal returns NotFound

        // Various path traversal attempts should all return NotFound
        let attacks = vec![
            "../../../etc/passwd",
            "..%2F..%2F..%2Fetc/passwd",
            "foo/../../../etc/passwd",
            "foo/bar/../../../etc/passwd",
            "....//....//etc/passwd",
        ];

        for attack in attacks {
            let result = resolve_request_path(&fixture.config(), attack);
            assert_eq!(
                result,
                ResolvedPath::NotFound,
                "Path traversal should be blocked for: {}",
                attack
            );
        }
    }

    #[test]
    fn test_path_traversal_blocked_in_static_folder() {
        let fixture = TestFixture::new();
        // Create a file in static folder
        fs::write(fixture.path().join("static/safe.txt"), "safe content").unwrap();

        // Path traversal within static folder should be blocked
        let attacks = vec![
            "../readme.md",     // Try to escape static to base_dir
            "../../etc/passwd", // Try to escape completely
            "foo/../../../etc/passwd",
        ];

        for attack in &attacks {
            let result = find_in_static_folder(&fixture.config(), attack);
            assert!(
                result.is_none(),
                "Static folder path traversal should be blocked for: {}",
                attack
            );
        }

        // But valid file should still work
        let valid = find_in_static_folder(&fixture.config(), "safe.txt");
        assert!(valid.is_some(), "Valid static file should be found");
    }

    #[test]
    fn test_safe_join_blocks_traversal() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Create a file inside
        fs::write(base.join("inside.txt"), "inside").unwrap();

        // Valid path should work
        let valid = safe_join_local(base, "inside.txt");
        assert!(valid.is_some(), "Valid path should work");
        assert!(valid.unwrap().ends_with("inside.txt"));

        // Path traversal should be blocked
        let attack = safe_join_local(base, "../../../etc/passwd");
        assert!(attack.is_none(), "Path traversal should be blocked");

        // Complex traversal should be blocked
        let attack2 = safe_join_local(base, "foo/../../../etc/passwd");
        assert!(
            attack2.is_none(),
            "Complex path traversal should be blocked"
        );
    }

    #[test]
    fn test_safe_join_allows_internal_dotdot() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Create nested structure
        fs::create_dir_all(base.join("foo/bar")).unwrap();
        fs::write(base.join("foo/sibling.txt"), "sibling").unwrap();

        // Going up and back down within base_dir should work
        let valid = safe_join_local(base, "foo/bar/../sibling.txt");
        assert!(valid.is_some(), "Internal navigation should work");
        let resolved = valid.unwrap();
        assert!(
            resolved.ends_with("sibling.txt"),
            "Should resolve to sibling.txt, got: {:?}",
            resolved
        );
    }

    #[test]
    fn test_path_traversal_returns_not_found_not_error() {
        let fixture = TestFixture::new();

        // Path traversal should cleanly return NotFound, not panic or error
        let result = resolve_request_path(&fixture.config(), "../../../../etc/passwd");

        // Should be NotFound, not a panic or file access
        assert_eq!(result, ResolvedPath::NotFound);
    }

    #[test]
    fn test_symlink_escape_blocked() {
        // This test verifies that symlinks pointing outside base_dir are blocked
        let dir = TempDir::new().unwrap();
        let base = dir.path();
        fs::create_dir(base.join("static")).unwrap();

        // Create a symlink in static folder pointing outside
        // (This is OS-dependent and may not work on all systems)
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let link_path = base.join("static/escape");
            // Try to create symlink to /tmp (which exists on most Unix systems)
            if symlink("/tmp", &link_path).is_ok() {
                let extensions = vec![String::from("md")];
                let tag_sources: Vec<String> = vec![];
                let vault = LocalVault::new(base);
                let config = PathResolverConfig {
                    vault: &vault,
                    static_vault: None,
                    static_folder: "static",
                    markdown_extensions: &extensions,
                    index_file: "index.md",
                    tag_sources: &tag_sources,
                    exempt_hidden_dirs: &[],
                };

                // Following the symlink should be blocked
                let result = find_in_static_folder(&config, "escape/some_file");
                assert!(result.is_none(), "Symlink escape should be blocked");
            }
        }
    }

    /// Regression: `safe_join` used to fall through to its "path doesn't exist
    /// yet" branch when `canonicalize()` *succeeded* but resolved outside the
    /// base. That branch validates only the parent, so it handed back the
    /// unresolved symlink and the server served the out-of-repo target
    /// (`GET /passwd` -> `/etc/passwd`).
    #[cfg(unix)]
    #[test]
    fn test_safe_join_blocks_symlink_to_absolute_path_outside_base() {
        use std::os::unix::fs::symlink;

        let outside = TempDir::new().unwrap();
        let secret = outside.path().join("secret.txt");
        fs::write(&secret, "top secret").unwrap();

        let dir = TempDir::new().unwrap();
        let base = dir.path();
        symlink(&secret, base.join("passwd")).unwrap();

        assert_eq!(
            safe_join_local(base, "passwd"),
            None,
            "a symlink resolving outside the base must not be joined"
        );
    }

    /// The relative form of the same escape: an attacker needs no knowledge of
    /// the victim's absolute paths.
    #[cfg(unix)]
    #[test]
    fn test_safe_join_blocks_relative_symlink_outside_base() {
        use std::os::unix::fs::symlink;

        let outer = TempDir::new().unwrap();
        fs::write(outer.path().join("outside.txt"), "top secret").unwrap();
        let base = outer.path().join("repo/nested");
        fs::create_dir_all(&base).unwrap();
        symlink("../../outside.txt", base.join("escape.txt")).unwrap();

        assert_eq!(
            safe_join_local(&base, "escape.txt"),
            None,
            "a relative symlink resolving outside the base must not be joined"
        );
    }

    /// A symlink that stays inside the base is still followed.
    #[cfg(unix)]
    #[test]
    fn test_safe_join_allows_symlink_inside_base() {
        use std::os::unix::fs::symlink;

        let dir = TempDir::new().unwrap();
        let base = dir.path();
        fs::write(base.join("real.txt"), "inside").unwrap();
        symlink("real.txt", base.join("alias.txt")).unwrap();

        let joined = safe_join_local(base, "alias.txt").expect("in-base symlink should resolve");
        assert_eq!(joined, base.canonicalize().unwrap().join("real.txt"));
    }

    /// The escape fix must not break the two branches that legitimately return
    /// `Some`: an existing in-base file, and a not-yet-existing sibling whose
    /// parent is in-base (how `/foo/` probes for `foo.md`).
    #[test]
    fn test_safe_join_existing_file_and_missing_sibling() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();
        let canonical = base.canonicalize().unwrap();
        fs::create_dir(base.join("docs")).unwrap();
        fs::write(base.join("docs/guide.md"), "# Guide").unwrap();

        assert_eq!(
            safe_join_local(base, "docs/guide.md"),
            Some(canonical.join("docs/guide.md")),
            "an existing in-base file must resolve"
        );
        assert_eq!(
            safe_join_local(base, "docs/guide"),
            Some(canonical.join("docs/guide")),
            "a not-yet-existing name under an in-base parent must resolve"
        );
    }

    /// Retargeted. This layer used to insist that the overlay itself sit under
    /// the repository root, which 404'd the whole `repo/content` +
    /// `repo/static` layout. Deciding *how far* `static_folder` may reach is
    /// `Config::validate_static_folder`'s job now (at most two levels up, never
    /// via `$HOME` or `/`); what this layer owes is that an out-of-root overlay actually
    /// resolves — the regression — while a request path still cannot walk out
    /// of it.
    #[test]
    fn test_find_in_static_folder_serves_peer_overlay_but_contains_requests() {
        let outer = TempDir::new().unwrap();
        let peer = outer.path().join("static");
        fs::create_dir(&peer).unwrap();
        fs::write(peer.join("logo.png"), "PNG").unwrap();
        fs::write(outer.path().join("id_rsa"), "PRIVATE KEY").unwrap();

        let base = outer.path().join("content");
        fs::create_dir(&base).unwrap();
        let canonical = base.canonicalize().unwrap();

        let extensions = vec![String::from("md")];
        let tag_sources: Vec<String> = vec![];

        let vault = LocalVault::new(&base);
        let peer_vault = overlay_vault(&canonical, "../static").expect("peer overlay accepted");
        let peer_overlay = PathResolverConfig {
            vault: &vault,
            static_vault: Some(&peer_vault),
            static_folder: "../static",
            markdown_extensions: &extensions,
            index_file: "index.md",
            tag_sources: &tag_sources,
            exempt_hidden_dirs: &[],
        };
        assert_eq!(
            find_in_static_folder(&peer_overlay, "logo.png"),
            Some(peer.canonicalize().unwrap().join("logo.png")),
            "a peer static folder must serve its own files"
        );
        assert!(
            matches!(
                resolve_request_path(&peer_overlay, "logo.png"),
                ResolvedPath::StaticFile(_)
            ),
            "the resolver must route the peer overlay's files as static files"
        );

        // Containment is measured against the overlay, so climbing out of it
        // fails even though the target is readable and nearby.
        for attack in ["../id_rsa", "../../etc/passwd", "sub/../../id_rsa"] {
            assert_eq!(
                find_in_static_folder(&peer_overlay, attack),
                None,
                "a request path must not climb out of the static overlay: {attack}"
            );
        }

        // An absolute overlay (only reachable via MBR_STATIC_FOLDER) behaves the
        // same way: it serves its own contents and contains request paths.
        let absolute = peer.to_string_lossy().into_owned();
        let absolute_vault = overlay_vault(&canonical, &absolute).expect("absolute overlay");
        let absolute_overlay = PathResolverConfig {
            static_folder: &absolute,
            static_vault: Some(&absolute_vault),
            ..peer_overlay
        };
        assert_eq!(
            find_in_static_folder(&absolute_overlay, "logo.png"),
            Some(peer.canonicalize().unwrap().join("logo.png")),
            "an absolute static folder must serve its own files"
        );
        assert_eq!(
            find_in_static_folder(&absolute_overlay, "../id_rsa"),
            None,
            "an absolute static folder must still contain request paths"
        );
    }

    /// A symlink *inside* the overlay is the traversal route that survives the
    /// policy change: the value of `static_folder` is innocent, and only
    /// canonicalizing the request target reveals that it lands on `/etc/passwd`.
    #[cfg(unix)]
    #[test]
    fn test_find_in_static_folder_blocks_symlink_out_of_overlay() {
        use std::os::unix::fs::symlink;

        let outer = TempDir::new().unwrap();
        let secrets = outer.path().join("secrets");
        fs::create_dir(&secrets).unwrap();
        fs::write(secrets.join("passwd"), "root:x:0:0").unwrap();

        let base = outer.path().join("content");
        fs::create_dir(&base).unwrap();
        let canonical = base.canonicalize().unwrap();
        let peer = outer.path().join("static");
        fs::create_dir(&peer).unwrap();

        // A file and a whole directory, each symlinked out of the overlay.
        symlink(secrets.join("passwd"), peer.join("passwd")).unwrap();
        symlink(&secrets, peer.join("leak")).unwrap();

        let extensions = vec![String::from("md")];
        let tag_sources: Vec<String> = vec![];
        let vault = LocalVault::new(&base);
        let peer_vault = overlay_vault(&canonical, "../static").expect("peer overlay accepted");
        let config = PathResolverConfig {
            vault: &vault,
            static_vault: Some(&peer_vault),
            static_folder: "../static",
            markdown_extensions: &extensions,
            index_file: "index.md",
            tag_sources: &tag_sources,
            exempt_hidden_dirs: &[],
        };

        for attack in ["passwd", "leak/passwd"] {
            assert_eq!(
                find_in_static_folder(&config, attack),
                None,
                "a symlink out of the static overlay must not be served: {attack}"
            );
            assert_eq!(
                resolve_request_path(&config, attack),
                ResolvedPath::NotFound,
                "the resolver must 404 a symlink out of the static overlay: {attack}"
            );
        }
    }

    // ==================== Static Folder Tests ====================

    #[test]
    fn test_precedence_base_dir_over_static() {
        // Request /image.png with file in BOTH locations
        // Should prefer base_dir (step 1 wins over step 4b)
        let fixture = TestFixture::new();
        fs::write(fixture.path().join("image.png"), "direct").unwrap();
        fs::write(fixture.path().join("static/image.png"), "static").unwrap();

        let result = resolve_request_path(&fixture.config(), "image.png");

        // Should return base_dir file, not static folder
        assert_eq!(
            result,
            ResolvedPath::StaticFile(fixture.canonical_path().join("image.png"))
        );

        // Verify the correct file would be served by checking content
        let resolved_path = match result {
            ResolvedPath::StaticFile(p) => p,
            _ => panic!("Expected StaticFile"),
        };
        let content = fs::read_to_string(resolved_path).unwrap();
        assert_eq!(
            content, "direct",
            "Should serve file from base_dir, not static folder"
        );
    }

    #[test]
    fn test_safe_join_failure_static_fallback() {
        // Request /images/blog/photo.png where:
        // - base_dir/images/ does NOT exist (safe_join fails)
        // - static/images/blog/photo.png DOES exist
        // This is the exact regression case
        let fixture = TestFixture::new();
        fs::create_dir_all(fixture.path().join("static/images/blog")).unwrap();
        fs::write(fixture.path().join("static/images/blog/photo.png"), "image").unwrap();
        // Note: base_dir/images/ does NOT exist

        let result = resolve_request_path(&fixture.config(), "images/blog/photo.png");

        let expected = fixture
            .path()
            .join("static/images/blog/photo.png")
            .canonicalize()
            .unwrap();
        assert_eq!(result, ResolvedPath::StaticFile(expected));
    }

    #[test]
    fn test_empty_static_folder_config() {
        // Config with static_folder = ""
        // Static folder lookup should be skipped
        let dir = TempDir::new().unwrap();
        fs::create_dir(dir.path().join("static")).unwrap();
        fs::write(dir.path().join("static/file.txt"), "content").unwrap();

        let extensions = vec![String::from("md")];
        let tag_sources: Vec<String> = vec![];
        let vault = LocalVault::new(dir.path());
        let config = PathResolverConfig {
            vault: &vault,
            static_vault: None,
            static_folder: "", // Empty!
            markdown_extensions: &extensions,
            index_file: "index.md",
            tag_sources: &tag_sources,
            exempt_hidden_dirs: &[],
        };

        let result = resolve_request_path(&config, "file.txt");
        assert_eq!(result, ResolvedPath::NotFound);
    }

    #[test]
    fn test_deeply_nested_static_path() {
        // Test 5+ levels of nesting
        let fixture = TestFixture::new();
        fs::create_dir_all(fixture.path().join("static/a/b/c/d/e")).unwrap();
        fs::write(fixture.path().join("static/a/b/c/d/e/deep.png"), "deep").unwrap();

        let result = resolve_request_path(&fixture.config(), "a/b/c/d/e/deep.png");

        let expected = fixture
            .path()
            .join("static/a/b/c/d/e/deep.png")
            .canonicalize()
            .unwrap();
        assert_eq!(result, ResolvedPath::StaticFile(expected));
    }

    /// A trailing slash on a static file request. This used to depend on the
    /// platform's `canonicalize()`: macOS tolerated `photo.png/` and served the
    /// file, Linux rejected it with `ENOTDIR` and 404'd. A request path is now
    /// normalized into a `VaultPath` before the filesystem sees it, so a
    /// trailing separator means nothing anywhere and every platform answers as
    /// macOS did — which is also what the root-folder half of the resolver
    /// already answered on Linux, through `safe_join`'s missing-file branch.
    #[test]
    fn test_static_folder_with_trailing_slash_request() {
        let fixture = TestFixture::new();
        fs::create_dir_all(fixture.path().join("static/images")).unwrap();
        fs::write(fixture.path().join("static/images/photo.png"), "img").unwrap();

        let result = resolve_request_path(&fixture.config(), "images/photo.png/");

        let expected = fixture
            .path()
            .join("static/images/photo.png")
            .canonicalize()
            .unwrap();
        assert_eq!(result, ResolvedPath::StaticFile(expected));
    }

    #[test]
    fn test_static_folder_url_encoded_spaces() {
        // Test that paths with spaces work through static folder
        let fixture = TestFixture::new();
        fs::create_dir_all(fixture.path().join("static/my images")).unwrap();
        fs::write(
            fixture.path().join("static/my images/photo file.jpg"),
            "img",
        )
        .unwrap();

        // URL-decoded path (as server would provide after decoding)
        let result = resolve_request_path(&fixture.config(), "my images/photo file.jpg");

        let expected = fixture
            .path()
            .join("static/my images/photo file.jpg")
            .canonicalize()
            .unwrap();
        assert_eq!(result, ResolvedPath::StaticFile(expected));
    }

    // ==================== Non-local vaults ====================

    fn mem_fixture() -> crate::vault::MemVault {
        crate::vault::MemVault::new()
            .with_file("readme.md", "# R")
            .with_file("docs/index.md", "# D")
            .with_file("docs/guide.md", "# G")
            .with_file("notes/a.b.md", "# AB")
            .with_file("img.png", "png")
            .with_file("static/images/photo.png", "img")
            .with_file("static/readme.md", "# shadowed")
            .with_dir("empty")
    }

    /// Every resolution step answers the same through a vault that has no
    /// filesystem at all.
    #[test]
    fn test_mem_vault_resolves_every_kind() {
        let vault = mem_fixture();
        let extensions = vec![String::from("md")];
        let tag_sources = vec![String::from("tags")];
        let config = PathResolverConfig {
            vault: &vault,
            static_vault: None,
            static_folder: "static",
            markdown_extensions: &extensions,
            index_file: "index.md",
            tag_sources: &tag_sources,
            exempt_hidden_dirs: &[],
        };
        let root = PathBuf::from(crate::vault::MemVault::DEFAULT_ROOT);
        let key = |rel: &str| VaultPath::new(rel).unwrap().to_native(&root);
        let cases = [
            ("readme.md", ResolvedPath::MarkdownFile(key("readme.md"))),
            ("readme/", ResolvedPath::MarkdownFile(key("readme.md"))),
            ("readme", ResolvedPath::MarkdownFile(key("readme.md"))),
            ("docs", ResolvedPath::MarkdownFile(key("docs/index.md"))),
            ("docs/", ResolvedPath::MarkdownFile(key("docs/index.md"))),
            ("docs/index/", ResolvedPath::Redirect("/docs/".to_string())),
            (
                "docs/guide/",
                ResolvedPath::MarkdownFile(key("docs/guide.md")),
            ),
            (
                "notes/a.b/",
                ResolvedPath::MarkdownFile(key("notes/a.b.md")),
            ),
            ("img.png", ResolvedPath::StaticFile(key("img.png"))),
            (
                "images/photo.png",
                ResolvedPath::StaticFile(key("static/images/photo.png")),
            ),
            ("empty/", ResolvedPath::DirectoryListing(key("empty"))),
            ("notes", ResolvedPath::DirectoryListing(key("notes"))),
            ("", ResolvedPath::DirectoryListing(root.clone())),
            ("missing/", ResolvedPath::NotFound),
            ("../readme.md", ResolvedPath::NotFound),
            ("/readme.md", ResolvedPath::NotFound),
            (
                "tags/rust/",
                ResolvedPath::TagPage {
                    source: "tags".to_string(),
                    value: "rust".to_string(),
                },
            ),
        ];
        for (request, expected) in cases {
            assert_eq!(
                resolve_request_path(&config, request),
                expected,
                "{request:?}"
            );
        }
    }

    /// An external overlay is a vault of its own, whatever it is backed by.
    #[test]
    fn test_mem_vault_external_overlay() {
        let vault = mem_fixture();
        let overlay = crate::vault::MemVault::with_root("/mbr-mem-overlay")
            .with_file("logo.png", "PNG")
            .with_file("css/site.css", "body{}");
        let extensions = vec![String::from("md")];
        let tag_sources: Vec<String> = vec![];
        let config = PathResolverConfig {
            vault: &vault,
            static_vault: Some(&overlay),
            static_folder: "../static",
            markdown_extensions: &extensions,
            index_file: "index.md",
            tag_sources: &tag_sources,
            exempt_hidden_dirs: &[],
        };
        assert_eq!(
            resolve_request_path(&config, "css/site.css"),
            ResolvedPath::StaticFile(
                PathBuf::from("/mbr-mem-overlay")
                    .join("css")
                    .join("site.css")
            )
        );
        // The in-vault `static/` folder is not consulted once an overlay is set.
        assert_eq!(
            resolve_request_path(&config, "images/photo.png"),
            ResolvedPath::NotFound
        );
        assert_eq!(find_in_static_folder(&config, "../logo.png"), None);
    }

    /// The vault root has no name, so resolving `/` in a repository without an
    /// index never probes `<parent>/<repo>.md` — a file *outside* the
    /// repository the path-based resolver would have served at `/`.
    #[test]
    fn test_root_request_never_probes_a_sibling_of_the_repository() {
        let outer = TempDir::new().unwrap();
        let repo = outer.path().join("notes");
        fs::create_dir(&repo).unwrap();
        fs::write(outer.path().join("notes.md"), "# Outside the repository").unwrap();
        let vault = LocalVault::new(&repo);
        let extensions = vec![String::from("md")];
        let tag_sources: Vec<String> = vec![];
        let config = PathResolverConfig {
            vault: &vault,
            static_vault: None,
            static_folder: "static",
            markdown_extensions: &extensions,
            index_file: "index.md",
            tag_sources: &tag_sources,
            exempt_hidden_dirs: &[],
        };
        assert_eq!(
            resolve_request_path(&config, ""),
            ResolvedPath::DirectoryListing(vault.root().to_path_buf())
        );
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use crate::vault::LocalVault;
    use proptest::prelude::*;
    use std::fs;
    use tempfile::TempDir;

    // Strategy for valid path component names
    fn path_component_strategy() -> impl Strategy<Value = String> {
        "[a-zA-Z0-9_-]{1,12}"
    }

    // Strategy for valid extensions
    fn extension_strategy() -> impl Strategy<Value = String> {
        "[a-z]{1,5}"
    }

    proptest! {
        /// is_markdown_file is deterministic
        #[test]
        fn prop_is_markdown_file_deterministic(
            filename in path_component_strategy(),
            ext in extension_strategy(),
            extensions in proptest::collection::vec(extension_strategy(), 1..4)
        ) {
            let path = PathBuf::from(format!("{}.{}", filename, ext));
            let result1 = is_markdown_file(&path, &extensions);
            let result2 = is_markdown_file(&path, &extensions);
            prop_assert_eq!(result1, result2);
        }

        /// is_markdown_file returns true when extension matches
        #[test]
        fn prop_is_markdown_file_matches_extension(
            filename in path_component_strategy(),
            extensions in proptest::collection::vec(extension_strategy(), 1..4)
        ) {
            // Use the first extension from the list
            if let Some(ext) = extensions.first() {
                let path = PathBuf::from(format!("{}.{}", filename, ext));
                prop_assert!(is_markdown_file(&path, &extensions));
            }
        }

        /// strip_trailing_separator is idempotent
        #[test]
        fn prop_strip_trailing_separator_idempotent(
            components in proptest::collection::vec(path_component_strategy(), 1..5)
        ) {
            let path_str = format!("/{}/", components.join("/"));
            let path = Path::new(&path_str);

            let once = strip_trailing_separator(path);
            let twice = strip_trailing_separator(&once);

            prop_assert_eq!(once, twice);
        }

        /// strip_trailing_separator never ends with a separator (except for root).
        ///
        /// Asserts on `/` (the URL contract this function actually operates
        /// under) *and* on the platform separator, so neither assumption can
        /// regress independently.
        #[test]
        fn prop_strip_trailing_separator_no_trailing(
            components in proptest::collection::vec(path_component_strategy(), 1..5)
        ) {
            let path_str = format!("/{}/", components.join("/"));
            let path = Path::new(&path_str);
            let result = strip_trailing_separator(path);
            let result_str = result.to_string_lossy();

            prop_assert!(
                !result_str.ends_with('/'),
                "Result {:?} should not end with /",
                result_str
            );
            prop_assert!(
                !result_str.ends_with(std::path::MAIN_SEPARATOR),
                "Result {:?} should not end with the platform separator",
                result_str
            );
        }

        /// Path resolution is deterministic for the same filesystem state
        #[test]
        fn prop_path_resolution_deterministic(
            request_path in proptest::collection::vec(path_component_strategy(), 0..3)
        ) {
            let dir = TempDir::new().unwrap();
            fs::create_dir(dir.path().join("static")).unwrap();

            // Create a markdown file
            fs::write(dir.path().join("test.md"), "# Test").unwrap();

            let extensions = vec![String::from("md")];
            let tag_sources: Vec<String> = vec![];
            let vault = LocalVault::new(dir.path());
            let config = PathResolverConfig {
                vault: &vault,
                static_vault: None,
                static_folder: "static",
                markdown_extensions: &extensions,
                index_file: "index.md",
                tag_sources: &tag_sources,
                exempt_hidden_dirs: &[],
            };

            let path_str = request_path.join("/");

            let result1 = resolve_request_path(&config, &path_str);
            let result2 = resolve_request_path(&config, &path_str);

            prop_assert_eq!(result1, result2);
        }

        /// Path traversal with ".." in paths doesn't cause panics
        /// and returns deterministic results
        #[test]
        fn prop_path_traversal_no_panic(
            prefix in proptest::collection::vec(path_component_strategy(), 0..2),
            suffix in proptest::collection::vec(path_component_strategy(), 0..2)
        ) {
            let dir = TempDir::new().unwrap();
            let base_dir = dir.path();
            fs::create_dir(base_dir.join("static")).unwrap();

            let extensions = vec![String::from("md")];
            let tag_sources: Vec<String> = vec![];
            let vault = LocalVault::new(base_dir);
            let config = PathResolverConfig {
                vault: &vault,
                static_vault: None,
                static_folder: "static",
                markdown_extensions: &extensions,
                index_file: "index.md",
                tag_sources: &tag_sources,
                exempt_hidden_dirs: &[],
            };

            // Try various path traversal patterns
            let attack_paths = vec![
                format!("{}/../{}", prefix.join("/"), suffix.join("/")),
                format!("../{}", suffix.join("/")),
                format!("{}/../../{}", prefix.join("/"), suffix.join("/")),
            ];

            for attack_path in attack_paths {
                // Should not panic and should return consistent results
                let result1 = resolve_request_path(&config, &attack_path);
                let result2 = resolve_request_path(&config, &attack_path);
                prop_assert_eq!(result1, result2, "Results should be deterministic for {:?}", attack_path);
            }
        }

        /// Whatever the request — separators, dots, encodings, drive letters —
        /// anything resolved is inside the vault.
        #[test]
        fn prop_resolved_paths_stay_inside_the_vault(
            request in "[a-z./\\\\%:C]{0,24}"
        ) {
            let vault = crate::vault::MemVault::new()
                .with_file("a/b.md", "# B")
                .with_file("static/c.png", "c")
                .with_file("d.md", "# D");
            let extensions = vec![String::from("md")];
            let tag_sources: Vec<String> = vec![];
            let config = PathResolverConfig {
                vault: &vault,
                static_vault: None,
                static_folder: "static",
                markdown_extensions: &extensions,
                index_file: "index.md",
                tag_sources: &tag_sources,
                exempt_hidden_dirs: &[],
            };
            match resolve_request_path(&config, &request) {
                ResolvedPath::StaticFile(p)
                | ResolvedPath::MarkdownFile(p)
                | ResolvedPath::DirectoryListing(p) => {
                    prop_assert!(
                        p.starts_with(crate::vault::MemVault::DEFAULT_ROOT),
                        "{:?} resolved to {:?}", request, p
                    );
                }
                ResolvedPath::Redirect(url) => {
                    prop_assert!(url.starts_with('/') && !url.contains(".."), "{:?}", url);
                }
                _ => {}
            }
        }
    }
}

/// The hidden-file rule and symlink mounts, end to end through the resolver.
///
/// Unix only where real symlinks are needed. Temp dirs get a visible prefix:
/// `tempfile`'s default `.tmpXXXX` is itself hidden, so a mount target under
/// one would be refused by the very rule under test.
#[cfg(test)]
mod hidden_and_mount_tests {
    use super::*;
    use crate::vault::{LocalVault, MountPolicy};
    use std::fs;

    fn visible_tempdir() -> tempfile::TempDir {
        tempfile::Builder::new()
            .prefix("mbr-resolver-test-")
            .tempdir()
            .unwrap()
    }

    fn resolve(
        vault: &dyn Vault,
        static_vault: Option<&dyn Vault>,
        exempt: &[VaultPath],
        request: &str,
    ) -> ResolvedPath {
        let extensions = vec![String::from("md")];
        let tag_sources = vec![String::from("tags")];
        let config = PathResolverConfig {
            vault,
            static_vault,
            static_folder: if static_vault.is_some() {
                "../static"
            } else {
                "static"
            },
            markdown_extensions: &extensions,
            index_file: "index.md",
            tag_sources: &tag_sources,
            exempt_hidden_dirs: exempt,
        };
        resolve_request_path(&config, request)
    }

    #[test]
    fn hidden_files_and_directories_are_never_resolved() {
        let dir = visible_tempdir();
        let root = dir.path().canonicalize().unwrap();
        for (path, body) in [
            (".env", "SECRET=1"),
            (".git/config", "[remote]"),
            (".git/notes.md", "# in git"),
            ("docs/.private/plan.md", "# plan"),
            ("docs/.hidden.png", "png"),
            ("docs/visible.md", "# ok"),
            ("static/.htpasswd", "user:hash"),
            ("static/images/.DS_Store", "junk"),
            ("static/images/logo.png", "png"),
            ("static/.well-known/security.txt", "Contact: x"),
            (".mbr/config.toml", "edit_token_hash = 'x'"),
        ] {
            let file = root.join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, body).unwrap();
        }
        let vault = LocalVault::new(&root);
        for request in [
            ".env",
            ".git/config",
            ".git",
            ".git/",
            ".git/notes/",
            "docs/.private/plan/",
            "docs/.private/plan.md",
            "docs/.hidden.png",
            ".htpasswd",
            "static/.htpasswd",
            "images/.DS_Store",
            ".mbr/config.toml",
            "docs/../.env",
        ] {
            assert_eq!(
                resolve(&vault, None, &[], request),
                ResolvedPath::NotFound,
                "{request} must not resolve"
            );
        }
        assert!(matches!(
            resolve(&vault, None, &[], "docs/visible/"),
            ResolvedPath::MarkdownFile(_)
        ));
        assert!(matches!(
            resolve(&vault, None, &[], "images/logo.png"),
            ResolvedPath::StaticFile(_)
        ));
        // RFC 8615 site metadata is the one dot path that is published.
        assert!(matches!(
            resolve(&vault, None, &[], ".well-known/security.txt"),
            ResolvedPath::StaticFile(_)
        ));
        // Tag pages are not files: a tag value may start with a dot.
        assert!(matches!(
            resolve(&vault, None, &[], "tags/.net/"),
            ResolvedPath::TagPage { .. }
        ));
    }

    /// The one exemption: hidden directories the user named on the command
    /// line — and only the named chain, not dot directories inside it.
    #[test]
    fn an_explicitly_named_hidden_directory_is_served() {
        let dir = visible_tempdir();
        let root = dir.path().canonicalize().unwrap();
        fs::create_dir_all(root.join(".scratch/.git")).unwrap();
        fs::write(root.join(".scratch/alpha.md"), "# Alpha").unwrap();
        fs::write(root.join(".scratch/.git/config"), "x").unwrap();
        let vault = LocalVault::new(&root);
        let exempt = [VaultPath::new(".scratch").unwrap()];
        assert!(matches!(
            resolve(&vault, None, &exempt, ".scratch/alpha/"),
            ResolvedPath::MarkdownFile(_)
        ));
        assert_eq!(
            resolve(&vault, None, &exempt, ".scratch/.git/config"),
            ResolvedPath::NotFound
        );
        assert_eq!(
            resolve(&vault, None, &[], ".scratch/alpha/"),
            ResolvedPath::NotFound
        );
    }

    /// A link that is not itself hidden cannot reach a hidden target.
    #[cfg(unix)]
    #[test]
    fn a_visible_link_to_a_hidden_directory_is_not_resolved() {
        let dir = visible_tempdir();
        let root = dir.path().canonicalize().unwrap();
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::write(root.join(".git/config"), "x").unwrap();
        std::os::unix::fs::symlink(root.join(".git"), root.join("notes")).unwrap();
        let vault = LocalVault::new(&root);
        assert_eq!(
            resolve(&vault, None, &[], "notes/config"),
            ResolvedPath::NotFound
        );
    }

    /// `site/{content, static}` with `static/videos -> <base>/movies`, and a
    /// set of links that must never serve anything: to `/`, to `$HOME`, to an
    /// ancestor of the root, to a hidden directory. `$HOME` is a fake.
    #[cfg(unix)]
    struct MountSite {
        _dir: tempfile::TempDir,
        base: PathBuf,
        content: PathBuf,
        overlay: PathBuf,
        movies: PathBuf,
        extra: PathBuf,
    }

    #[cfg(unix)]
    impl MountSite {
        fn new() -> Self {
            use std::os::unix::fs::symlink;
            let dir = visible_tempdir();
            let base = dir.path().canonicalize().unwrap();
            let content = base.join("site/content");
            let overlay = base.join("site/static");
            let movies = base.join("movies");
            let extra = base.join("extra");
            for (path, body) in [
                ("site/content/index.md", "# Home"),
                ("site/content/notes/a.md", "# A"),
                ("site/content/.env", "SECRET=1"),
                ("site/static/images/logo.png", "png"),
                ("movies/clip.mp4", "clip"),
                ("movies/notes.md", "# Mounted"),
                ("movies/sub/deep.mp4", "deep"),
                ("movies/.secret/key", "key"),
                ("movies/.dotfile", "dot"),
                ("extra/x.mp4", "x"),
                ("home/diary.md", "diary"),
                (".hidden/key", "key"),
                ("secret.txt", "secret"),
            ] {
                let file = base.join(path);
                fs::create_dir_all(file.parent().unwrap()).unwrap();
                fs::write(file, body).unwrap();
            }
            // Accepted: the user's case, plus a nested mount inside it, a link
            // back into the repository and a cycle.
            symlink(&movies, overlay.join("videos")).unwrap();
            symlink(&movies, content.join("media")).unwrap();
            symlink(&extra, movies.join("more")).unwrap();
            symlink(content.join("notes"), movies.join("back")).unwrap();
            symlink(&movies, movies.join("loop")).unwrap();
            // Refused, every one of them.
            symlink("/", overlay.join("fsroot")).unwrap();
            symlink(base.join("home"), overlay.join("home")).unwrap();
            symlink(&base, content.join("up")).unwrap();
            symlink("../..", movies.join("climb")).unwrap();
            symlink(base.join(".hidden"), overlay.join("hidden")).unwrap();
            Self {
                _dir: dir,
                base,
                content,
                overlay,
                movies,
                extra,
            }
        }

        fn policy(&self, other: &Path) -> MountPolicy {
            MountPolicy::default()
                .with_home(Some(self.base.join("home")))
                .with_other_root(other)
        }

        fn vaults(&self) -> (LocalVault, LocalVault) {
            (
                LocalVault::with_mounts(&self.content, self.policy(&self.overlay)),
                LocalVault::with_mounts(&self.overlay, self.policy(&self.content)),
            )
        }

        /// Where a served key really is, if it is somewhere it may be.
        fn allowed(&self, key: &Path) -> bool {
            let Ok(real) = key.canonicalize() else {
                return false;
            };
            [&self.content, &self.overlay, &self.movies, &self.extra]
                .iter()
                .any(|root| {
                    real.strip_prefix(root).is_ok_and(|below| {
                        !below.components().any(|c| {
                            c.as_os_str()
                                .to_str()
                                .is_none_or(crate::vault::is_hidden_segment)
                        })
                    })
                })
        }
    }

    #[cfg(unix)]
    #[test]
    fn mounts_are_served_and_refused_targets_are_not() {
        let site = MountSite::new();
        let (vault, overlay) = site.vaults();
        let served = |request: &str| match resolve(&vault, Some(&overlay), &[], request) {
            ResolvedPath::StaticFile(key) | ResolvedPath::MarkdownFile(key) => {
                Some(fs::read_to_string(key).unwrap())
            }
            _ => None,
        };
        assert_eq!(served("videos/clip.mp4").as_deref(), Some("clip"));
        assert_eq!(served("videos/sub/deep.mp4").as_deref(), Some("deep"));
        assert_eq!(served("videos/more/x.mp4").as_deref(), Some("x"));
        assert_eq!(served("media/clip.mp4").as_deref(), Some("clip"));
        assert_eq!(served("media/notes/").as_deref(), Some("# Mounted"));
        assert_eq!(served("media/back/a/").as_deref(), Some("# A"));
        assert_eq!(served("media/loop/loop/clip.mp4").as_deref(), Some("clip"));
        for refused in [
            "videos/.secret/key",
            "videos/.dotfile",
            "media/.secret/key",
            "fsroot/etc/hosts",
            "home/diary.md",
            "up/secret.txt",
            "up/home/diary.md",
            "media/climb/secret.txt",
            "hidden/key",
            "videos/../../secret.txt",
            "videos/%2e%2e/%2e%2e/secret.txt",
            ".env",
        ] {
            assert_eq!(served(refused), None, "{refused} must not be served");
        }
        let mut targets: Vec<PathBuf> = vault
            .mounts()
            .into_iter()
            .chain(overlay.mounts())
            .map(|m| m.target)
            .collect();
        targets.sort();
        targets.dedup();
        let mut expected = vec![site.movies.clone(), site.extra.clone()];
        expected.sort();
        assert_eq!(
            targets, expected,
            "only the two legitimate targets are mounted"
        );
    }

    #[cfg(unix)]
    mod prop {
        use super::*;
        use proptest::prelude::*;

        const SEGMENTS: &[&str] = &[
            "videos",
            "media",
            "more",
            "back",
            "loop",
            "climb",
            "up",
            "fsroot",
            "home",
            "hidden",
            "sub",
            "notes",
            "images",
            "etc",
            "clip.mp4",
            "deep.mp4",
            "x.mp4",
            "logo.png",
            "a",
            "a.md",
            "index",
            "secret.txt",
            "diary.md",
            "key",
            ".secret",
            ".dotfile",
            ".env",
            ".hidden",
            "..",
            ".",
            "%2e%2e",
            "static",
            "site",
            "content",
            "movies",
            "extra",
        ];

        proptest! {
            #![proptest_config(ProptestConfig::with_cases(64))]

            /// No request path resolves to anything outside the repository,
            /// the overlay and the accepted mount targets — or to anything
            /// hidden in them — and only the legitimate targets get mounted.
            #[test]
            fn prop_nothing_outside_the_served_roots_resolves(
                requests in proptest::collection::vec(
                    (proptest::collection::vec(proptest::sample::select(SEGMENTS), 0..7), any::<bool>()),
                    1..12,
                )
            ) {
                let site = MountSite::new();
                let (vault, overlay) = site.vaults();
                for (segments, trailing) in requests {
                    let mut request = segments.join("/");
                    if trailing {
                        request.push('/');
                    }
                    match resolve(&vault, Some(&overlay), &[], &request) {
                        ResolvedPath::StaticFile(key)
                        | ResolvedPath::MarkdownFile(key)
                        | ResolvedPath::DirectoryListing(key) => {
                            prop_assert!(site.allowed(&key), "{request:?} served {key:?}");
                        }
                        _ => {}
                    }
                }
                for mount in vault.mounts().into_iter().chain(overlay.mounts()) {
                    prop_assert!(
                        mount.target == site.movies || mount.target == site.extra,
                        "unexpected mount {mount:?}"
                    );
                }
            }
        }
    }
}
