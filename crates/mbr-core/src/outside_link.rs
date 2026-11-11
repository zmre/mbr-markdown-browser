//! Links that leave the repository.
//!
//! Notes written for an editor link to files anywhere on disk:
//! `[TODOs](~/src/project/TODO.md)` or a `../../other-repo/README.md` chain
//! that climbs above the folder mbr serves. mbr never serves those files —
//! the sandbox is the point — but neither are they broken links, so they get
//! their own treatment:
//!
//! - **Classification is lexical.** [`classify`] looks only at the authored
//!   destination and the page's own URL: `~` / `~/…`, or a relative path whose
//!   `..` segments climb above the repository root. Nothing is ever `stat`ed
//!   for the page's sake: an existence check whose answer reached the page
//!   would be a file-existence oracle for any script in a rendered note (mbr
//!   renders raw HTML). A root-relative `/Users/…` is *not* outside — in mbr
//!   `/x` means the site root — and `file://` is a scheme like any other.
//! - **Mounts first.** When the target, made absolute (`~` expanded, `..`
//!   resolved against the canonical root), lies under an accepted read-only
//!   symlink mount, the link is rewritten to the mount's in-vault location and
//!   then treated as any internal link ([`OutsideLinkContext::site_url`]; the
//!   longest target prefix wins, as in the mount table's own `map`).
//! - **Everything else** points at the `/.mbr/outside/?path=…` page
//!   ([`viewer_url`]), which says where the link goes and why it cannot be
//!   shown. `page_errors` skips `/.mbr/*` and the static build's checker skips
//!   `?path=` routes, so these are no longer reported as broken.
//!
//! Without an [`OutsideLinkContext`] (CLI, QuickLook, the link tools) there is
//! no repository to map into and no viewer page: a `~` link is left exactly as
//! authored (it used to become `../~/…`) and a `../` chain keeps its historical
//! treatment.

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, percent_decode_str, utf8_percent_encode};

use crate::link_transform::LinkTransformConfig;
use crate::vault::{Vault, VaultPath};

/// The page an unmapped outside link points at. Its `path` query parameter
/// names the target; see [`viewer_url`].
pub const OUTSIDE_VIEWER_ROUTE: &str = "/.mbr/outside/";

/// What a `path=` query value escapes: everything but unreserved characters
/// and `/`, so `~/src/x.md` stays readable while `&`, `#`, `+` (a space to
/// `URLSearchParams`), `%` and spaces round-trip.
const QUERY_VALUE: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~')
    .remove(b'/');

/// What a path segment of a mapped in-vault URL escapes.
const PATH_SEGMENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// Where an outside link starts from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutsideBase {
    /// `~` — the user's home directory.
    Home,
    /// The repository root, which the link climbed above.
    RepositoryRoot,
}

/// An authored link destination that leaves the repository, normalized
/// lexically: `up` levels above [`Self::base`], then down `segments`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutsideTarget {
    pub base: OutsideBase,
    /// Levels above the base. Always at least 1 for
    /// [`OutsideBase::RepositoryRoot`] — that is what made it outside.
    pub up: usize,
    /// The authored segments below that, `.` and `..` resolved, **not**
    /// percent-decoded (see [`Self::decoded_segments`]).
    pub segments: Vec<String>,
}

/// Classifies an authored link destination as outside the repository, or not.
///
/// `current_page_url` and `is_index_file` say where the note is, exactly as
/// [`crate::link_transform::LinkTransformConfig`] carries them: a non-index
/// page's last URL segment is its file stem, so relative paths resolve against
/// its parent. Any `?query` / `#fragment` is ignored, so classification never
/// depends on them. Purely lexical: no filesystem access.
///
/// ```
/// use mbr_core::outside_link::{classify, OutsideBase};
///
/// let home = classify("~/src/app/TODO.md#next", "/projects/", false).unwrap();
/// assert_eq!(home.base, OutsideBase::Home);
/// assert_eq!(home.display(), "~/src/app/TODO.md");
///
/// let above = classify("../../other/README.md", "/notes/today/", false).unwrap();
/// assert_eq!(above.base, OutsideBase::RepositoryRoot);
/// assert_eq!(above.display(), "../other/README.md");
///
/// assert!(classify("../sibling.md", "/notes/today/", false).is_none());
/// assert!(classify("/Users/me/x.md", "/notes/today/", false).is_none());
/// ```
pub fn classify(url: &str, current_page_url: &str, is_index_file: bool) -> Option<OutsideTarget> {
    if crate::url_path::is_external_url(url) {
        return None;
    }
    let path = url.split(['?', '#']).next().unwrap_or_default();
    if path.is_empty() || path.starts_with('/') {
        return None;
    }

    if let Some(rest) = path.strip_prefix('~')
        && (rest.is_empty() || rest.starts_with('/'))
    {
        let (up, segments) = normalize(rest.split('/'), Vec::new());
        return Some(OutsideTarget {
            base: OutsideBase::Home,
            up,
            segments,
        });
    }

    let (up, segments) = normalize(
        path.split('/'),
        page_folder(current_page_url, is_index_file),
    );
    (up > 0).then_some(OutsideTarget {
        base: OutsideBase::RepositoryRoot,
        up,
        segments,
    })
}

/// The URL segments of the folder a page's relative links are written from.
fn page_folder(current_page_url: &str, is_index_file: bool) -> Vec<String> {
    let mut segments: Vec<String> = current_page_url
        .split('/')
        .filter(|segment| !segment.is_empty())
        .map(str::to_string)
        .collect();
    if !is_index_file {
        segments.pop();
    }
    segments
}

/// Applies `parts` to `start` lexically, counting the `..` that found nothing
/// to pop.
fn normalize<'a>(parts: impl Iterator<Item = &'a str>, start: Vec<String>) -> (usize, Vec<String>) {
    parts.fold((0, start), |(up, mut segments), part| match part {
        "" | "." => (up, segments),
        ".." => {
            if segments.pop().is_none() {
                (up + 1, segments)
            } else {
                (up, segments)
            }
        }
        segment => {
            segments.push(segment.to_string());
            (up, segments)
        }
    })
}

/// A segment percent-decoded, when that yields one plain path segment;
/// otherwise as authored (so `%2F` can never add a directory level).
fn decode_segment(segment: &str) -> String {
    percent_decode_str(segment)
        .decode_utf8()
        .ok()
        .filter(|decoded| {
            !decoded.is_empty()
                && decoded != "."
                && decoded != ".."
                && !decoded.contains(['/', '\\', '\0'])
        })
        .map_or_else(|| segment.to_string(), |decoded| decoded.into_owned())
}

impl OutsideTarget {
    /// The segments as file names (percent-decoded where that is safe).
    pub fn decoded_segments(&self) -> impl Iterator<Item = String> + '_ {
        self.segments.iter().map(|segment| decode_segment(segment))
    }

    /// The target in the author's own terms: `~/src/x.md`, or `../src/x.md`
    /// relative to the repository root. Never absolute, so it is safe to
    /// publish.
    pub fn display(&self) -> String {
        let below = self.decoded_segments().collect::<Vec<_>>().join("/");
        match self.base {
            OutsideBase::Home => {
                let mut out = "~".to_string();
                for _ in 0..self.up {
                    out.push_str("/..");
                }
                if !below.is_empty() {
                    out.push('/');
                    out.push_str(&below);
                }
                out
            }
            OutsideBase::RepositoryRoot => format!("{}{below}", "../".repeat(self.up)),
        }
    }

    /// The target as an absolute path, lexically: `root` (canonical) or
    /// `home` for the base, `up` levels above it, then the segments. `None`
    /// when there is no home directory or the climb passes the filesystem
    /// root.
    pub fn absolute(&self, root: &Path, home: Option<&Path>) -> Option<PathBuf> {
        let base = match self.base {
            OutsideBase::Home => home?,
            OutsideBase::RepositoryRoot => root,
        };
        let mut path = base.ancestors().nth(self.up)?.to_path_buf();
        path.extend(self.decoded_segments());
        Some(path)
    }
}

/// The viewer page for an outside target named by `path`: `/.mbr/outside/?path=…`.
///
/// Site-absolute; a static build relativizes it like any other root-relative
/// URL. The authored fragment and query are not carried: they mean nothing to
/// the viewer.
pub fn viewer_url(path: &str) -> String {
    format!(
        "{OUTSIDE_VIEWER_ROUTE}?path={}",
        utf8_percent_encode(path, QUERY_VALUE)
    )
}

/// Maps an absolute, lexically normalized path to the site URL of the same
/// file inside the repository — under the root, an external static overlay or
/// an accepted symlink mount — or `None`.
pub type SiteUrlLookup = Arc<dyn Fn(&Path) -> Option<String> + Send + Sync>;

/// What the link transform needs to treat outside links: the canonical root
/// (to resolve `../` escapes), the home directory (for `~`) and a lookup into
/// the repository's mounts.
#[derive(Clone)]
pub struct OutsideLinkContext {
    root: PathBuf,
    home: Option<PathBuf>,
    site_url: SiteUrlLookup,
    /// Whether a `../` escape's viewer link names its absolute path (see
    /// [`Self::naming_absolute_paths`]).
    absolute_paths: bool,
}

impl std::fmt::Debug for OutsideLinkContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OutsideLinkContext")
            .field("root", &self.root)
            .field("home", &self.home)
            .field("absolute_paths", &self.absolute_paths)
            .finish_non_exhaustive()
    }
}

impl OutsideLinkContext {
    /// A context from its parts (tests, or an embedder with its own lookup).
    pub fn new(root: PathBuf, home: Option<PathBuf>, site_url: SiteUrlLookup) -> Self {
        Self {
            root,
            home,
            site_url,
            absolute_paths: false,
        }
    }

    /// Name a `../` escape by its absolute path in the viewer link, rather
    /// than relative to the repository root. For the local server and GUI
    /// only, where the reader is the machine's owner and the GUI opens its
    /// folder; a static build may be published and must never write one.
    #[must_use]
    pub fn naming_absolute_paths(mut self) -> Self {
        self.absolute_paths = true;
        self
    }

    /// The context for a repository: its vault and external static overlay,
    /// each with the mounts accepted *so far* (read at lookup time, so mounts
    /// the scan discovers after this is built still count). `$HOME` is
    /// canonicalized once, as [`crate::vault::MountPolicy::new`] does, so it
    /// compares with canonical mount targets.
    pub fn for_repo(repo: &crate::repo::Repo) -> Self {
        let vaults: Vec<Arc<dyn Vault>> = std::iter::once(Arc::clone(repo.vault()))
            .chain(repo.static_vault().cloned())
            .collect();
        Self {
            root: repo.vault().root().to_path_buf(),
            home: crate::config::home_dir().map(|home| home.canonicalize().unwrap_or(home)),
            site_url: Arc::new(move |absolute: &Path| site_url_in_vaults(&vaults, absolute)),
            absolute_paths: false,
        }
    }

    /// The target as an absolute path (see [`OutsideTarget::absolute`]).
    pub fn absolute(&self, target: &OutsideTarget) -> Option<PathBuf> {
        target.absolute(&self.root, self.home.as_deref())
    }

    /// The in-repository site URL for an absolute path, if it has one.
    pub fn site_url(&self, absolute: &Path) -> Option<String> {
        (self.site_url)(absolute)
    }
}

/// [`SiteUrlLookup`] over a repository's vaults: the vault roots themselves and
/// every accepted mount, longest prefix first. No filesystem access — mounts
/// are what discovery has already accepted.
pub fn site_url_in_vaults(vaults: &[Arc<dyn Vault>], absolute: &Path) -> Option<String> {
    vaults
        .iter()
        .flat_map(|vault| {
            std::iter::once((vault.root().to_path_buf(), VaultPath::root())).chain(
                vault
                    .mounts()
                    .into_iter()
                    .map(|mount| (mount.target, mount.location)),
            )
        })
        .filter(|(target, _)| absolute.starts_with(target))
        .max_by_key(|(target, _)| target.components().count())
        .and_then(|(target, location)| {
            let below = absolute.strip_prefix(&target).ok()?;
            site_url_of(&location, below)
        })
}

/// `/location/below`, each segment percent-encoded. `None` if `below` holds
/// anything but plain names (it never does for a normalized path).
fn site_url_of(location: &VaultPath, below: &Path) -> Option<String> {
    let below: Vec<String> = below
        .components()
        .map(|component| match component {
            Component::Normal(name) => name.to_str().map(str::to_string),
            _ => None,
        })
        .collect::<Option<_>>()?;
    let segments: Vec<String> = location
        .segments()
        .map(str::to_string)
        .chain(below)
        .map(|segment| utf8_percent_encode(&segment, PATH_SEGMENT).to_string())
        .collect();
    Some(format!("/{}", segments.join("/")))
}

/// What the link transform does with an outside link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutsideResolution {
    /// Mapped into the repository: the canonical site-absolute URL
    /// (markdown → trailing-slash page URL), authored query/fragment kept.
    InVault(String),
    /// Not mapped: the site-absolute [`viewer_url`].
    Viewer(String),
    /// No repository context: leave the destination exactly as authored.
    Unchanged,
}

/// Decides what an authored relative destination becomes when it leaves the
/// repository; `None` when it does not (or, without an
/// [`LinkTransformConfig::outside_links`] context, when it is a `../` chain,
/// which keeps its historical treatment).
///
/// The `path=` value names the target as the author wrote it (`~/…`), except
/// a `../` escape under a context [naming absolute
/// paths](OutsideLinkContext::naming_absolute_paths) (the server), which is
/// written as the absolute path it resolves to, so the reader can copy it and
/// the GUI can open its folder. A static build, which may be published, writes
/// it relative to the repository root (`../src/x.md`) and never names a local
/// absolute path.
pub fn resolve(url: &str, config: &LinkTransformConfig) -> Option<OutsideResolution> {
    let target = classify(url, &config.current_page_url, config.is_index_file)?;
    let Some(context) = &config.outside_links else {
        return (target.base == OutsideBase::Home).then_some(OutsideResolution::Unchanged);
    };

    let absolute = context.absolute(&target);
    if let Some(site_url) = absolute.as_deref().and_then(|path| context.site_url(path)) {
        let suffix = url.find(['?', '#']).map_or("", |at| &url[at..]);
        return Some(OutsideResolution::InVault(
            crate::link_transform::canonicalize_root_relative(
                &format!("{site_url}{suffix}"),
                config,
            ),
        ));
    }

    let shown = match (target.base, context.absolute_paths, absolute) {
        (OutsideBase::RepositoryRoot, true, Some(path)) => path.to_string_lossy().into_owned(),
        _ => target.display(),
    };
    Some(OutsideResolution::Viewer(viewer_url(&shown)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn target(base: OutsideBase, up: usize, segments: &[&str]) -> OutsideTarget {
        OutsideTarget {
            base,
            up,
            segments: segments.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn home_links_are_outside() {
        assert_eq!(
            classify("~/src/ledgeline/TODO.md", "/current-projects/", false),
            Some(target(
                OutsideBase::Home,
                0,
                &["src", "ledgeline", "TODO.md"]
            ))
        );
        assert_eq!(
            classify("~", "/a/", false),
            Some(target(OutsideBase::Home, 0, &[]))
        );
        assert_eq!(
            classify("~/../shared/x.md", "/a/", false),
            Some(target(OutsideBase::Home, 1, &["shared", "x.md"]))
        );
    }

    #[test]
    fn tilde_names_that_are_not_home_are_not_outside() {
        assert_eq!(classify("~user/x.md", "/a/", false), None);
        assert_eq!(classify("./~/x.md", "/a/", false), None);
        assert_eq!(classify("notes~/x.md", "/a/", false), None);
    }

    #[test]
    fn climbing_above_the_root_is_outside() {
        // `/notes/today/` is notes/today.md: its folder is /notes/.
        assert_eq!(classify("../x.md", "/notes/today/", false), None);
        assert_eq!(
            classify("../../x.md", "/notes/today/", false),
            Some(target(OutsideBase::RepositoryRoot, 1, &["x.md"]))
        );
        // An index page's URL is its folder.
        assert_eq!(classify("../x.md", "/notes/", true), None);
        assert_eq!(
            classify("../../x.md", "/notes/", true),
            Some(target(OutsideBase::RepositoryRoot, 1, &["x.md"]))
        );
        assert_eq!(
            classify("../a/../../b/./c.md", "/", true),
            Some(target(OutsideBase::RepositoryRoot, 2, &["b", "c.md"]))
        );
    }

    #[test]
    fn other_destinations_are_not_outside() {
        for url in [
            "",
            "#top",
            "?q=1",
            "/Users/me/x.md",
            "/x.md",
            "https://example.com/~/x",
            "file:///Users/me/x.md",
            "mailto:me@example.com",
            "x.md",
            "sub/../x.md",
        ] {
            assert_eq!(classify(url, "/notes/today/", false), None, "{url}");
        }
    }

    #[test]
    fn display_is_authored_and_decoded() {
        let home = classify("~/My%20Docs/a.md", "/", true).unwrap();
        assert_eq!(home.display(), "~/My Docs/a.md");
        let up = classify("../../p/q%2Fr.md", "/n/", true).unwrap();
        // `%2F` cannot add a level: kept as written.
        assert_eq!(up.display(), "../p/q%2Fr.md");
    }

    #[test]
    fn absolute_resolves_lexically() {
        let root = Path::new("/home/me/Notes");
        let home = Path::new("/home/me");
        let above = classify("../../src/x.md", "/n/a/", false).unwrap();
        assert_eq!(
            above.absolute(root, Some(home)),
            Some(PathBuf::from("/home/me/src/x.md"))
        );
        let tilde = classify("~/src/My%20x.md", "/", true).unwrap();
        assert_eq!(
            tilde.absolute(root, Some(home)),
            Some(PathBuf::from("/home/me/src/My x.md"))
        );
        assert_eq!(tilde.absolute(root, None), None);
        let too_far = target(OutsideBase::RepositoryRoot, 9, &["x"]);
        assert_eq!(too_far.absolute(root, Some(home)), None);
    }

    #[test]
    fn viewer_url_encodes_the_query_value() {
        assert_eq!(
            viewer_url("~/src/a b&c#d+e.md"),
            "/.mbr/outside/?path=~/src/a%20b%26c%23d%2Be.md"
        );
        assert!(crate::url_path::is_mbr_query_route(&viewer_url("/x/y.md")));
    }

    fn context(root: &str, home: &str, mounts: Vec<(&str, &str)>) -> OutsideLinkContext {
        let root = PathBuf::from(root);
        let mounts: Vec<(PathBuf, String)> = mounts
            .into_iter()
            .map(|(target, location)| (PathBuf::from(target), location.to_string()))
            .collect();
        let lookup_root = root.clone();
        OutsideLinkContext::new(
            root,
            Some(PathBuf::from(home)),
            Arc::new(move |absolute: &Path| {
                if let Ok(below) = absolute.strip_prefix(&lookup_root) {
                    return site_url_of(&VaultPath::root(), below);
                }
                mounts
                    .iter()
                    .filter(|(target, _)| absolute.starts_with(target))
                    .max_by_key(|(target, _)| target.components().count())
                    .and_then(|(target, location)| {
                        site_url_of(
                            &VaultPath::new(location).unwrap(),
                            absolute.strip_prefix(target).ok()?,
                        )
                    })
            }),
        )
    }

    fn config(page: &str, is_index: bool, ctx: Option<OutsideLinkContext>) -> LinkTransformConfig {
        LinkTransformConfig {
            current_page_url: page.to_string(),
            is_index_file: is_index,
            outside_links: ctx,
            ..LinkTransformConfig::default()
        }
    }

    #[test]
    fn without_a_context_home_links_are_left_alone() {
        let cfg = config("/a/", false, None);
        assert_eq!(resolve("~/x.md", &cfg), Some(OutsideResolution::Unchanged));
        assert_eq!(resolve("../../x.md", &cfg), None);
        // CLI / QuickLook: a home link is no longer turned into `../~/x.md`.
        assert_eq!(
            crate::link_transform::transform_link("~/x.md#a", &cfg),
            "~/x.md#a"
        );
        assert_eq!(
            crate::link_transform::transform_link("../../x.md", &cfg),
            "../../../x/"
        );
    }

    #[test]
    fn unmapped_targets_point_at_the_viewer() {
        let ctx = context("/home/me/Notes", "/home/me", vec![]);
        let cfg = config("/n/a/", false, Some(ctx.clone().naming_absolute_paths()));
        assert_eq!(
            resolve("~/src/x.md#frag", &cfg),
            Some(OutsideResolution::Viewer(
                "/.mbr/outside/?path=~/src/x.md".to_string()
            ))
        );
        // Server mode: a `../` escape is named by its absolute path.
        assert_eq!(
            resolve("../../src/x.md", &cfg),
            Some(OutsideResolution::Viewer(
                "/.mbr/outside/?path=/home/me/src/x.md".to_string()
            ))
        );
        // A build: never an absolute path.
        let build = config("/n/a/", false, Some(ctx));
        assert_eq!(
            resolve("../../src/x.md", &build),
            Some(OutsideResolution::Viewer(
                "/.mbr/outside/?path=../src/x.md".to_string()
            ))
        );
    }

    /// A loaded URL (`<img src>`) never points at the viewer page, which is
    /// HTML; a mounted one still maps.
    #[test]
    fn asset_urls_map_into_mounts_but_never_load_the_viewer() {
        use crate::link_transform::{transform_asset_url, transform_link};
        let ctx = context(
            "/home/me/Notes",
            "/home/me",
            vec![("/home/me/pics", "pics")],
        );
        let cfg = config("/n/a/", false, Some(ctx));
        assert_eq!(transform_asset_url("~/x.png", &cfg), "~/x.png");
        assert_eq!(transform_asset_url("../../x.png", &cfg), "../../../x.png");
        assert_eq!(transform_asset_url("~/pics/a.png", &cfg), "/pics/a.png");
        assert_eq!(
            transform_link("~/x.png", &cfg),
            "/.mbr/outside/?path=~/x.png"
        );
    }

    #[test]
    fn mounted_targets_map_into_the_vault_longest_prefix_first() {
        let ctx = context(
            "/home/me/Notes",
            "/home/me",
            vec![
                ("/home/me/src", "code"),
                ("/home/me/src/ledgeline", "projects/ledgeline"),
            ],
        );
        let cfg = config("/n/a/", false, Some(ctx));
        assert_eq!(
            resolve("~/src/ledgeline/TODO.md#next", &cfg),
            Some(OutsideResolution::InVault(
                "/projects/ledgeline/TODO/#next".to_string()
            ))
        );
        assert_eq!(
            resolve("../../src/other/My%20Notes.md", &cfg),
            Some(OutsideResolution::InVault(
                "/code/other/My%20Notes/".to_string()
            ))
        );
        // Climbing out and back in lands in the root itself.
        assert_eq!(
            resolve("~/Notes/n/b.md", &cfg),
            Some(OutsideResolution::InVault("/n/b/".to_string()))
        );
    }

    /// The repository context reads the mounts the scan accepted: a `../`
    /// chain or an absolute target under a symlinked folder lands on the
    /// link's in-vault page, and links out of it stay outside.
    #[cfg(unix)]
    #[test]
    fn for_repo_maps_targets_under_a_scanned_mount() {
        let outside = tempfile::Builder::new()
            .prefix("mbr-outside-test-")
            .tempdir()
            .unwrap();
        let project = outside.path().join("ledgeline");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(project.join("TODO.md"), "# Todo").unwrap();
        let root = tempfile::Builder::new()
            .prefix("mbr-outside-root-")
            .tempdir()
            .unwrap();
        std::fs::write(root.path().join("index.md"), "# Home").unwrap();
        std::os::unix::fs::symlink(&project, root.path().join("ledgeline")).unwrap();

        let repo = crate::repo::Repo::init(
            root.path().to_path_buf(),
            "static".to_string(),
            &["md".to_string()],
            &[],
            &[],
            "index.md".to_string(),
            &[],
            &[],
        );
        repo.scan_all().unwrap();
        let ctx = OutsideLinkContext::for_repo(&repo);

        let canonical_project = project.canonicalize().unwrap();
        assert_eq!(
            ctx.site_url(&canonical_project.join("TODO.md")),
            Some("/ledgeline/TODO.md".to_string())
        );
        let outside_name = outside.path().file_name().unwrap().to_str().unwrap();
        let cfg = config("/", true, Some(ctx.clone()));
        assert_eq!(
            crate::link_transform::transform_link(
                &format!("../{outside_name}/ledgeline/TODO.md#top"),
                &cfg
            ),
            "/ledgeline/TODO/#top"
        );
        // Beside the mount, not in it: the viewer, named relative to the root.
        assert_eq!(
            crate::link_transform::transform_link(&format!("../{outside_name}/other.md"), &cfg),
            format!("/.mbr/outside/?path=../{outside_name}/other.md")
        );
        // Build mode relativizes both like any root-relative URL.
        let build = LinkTransformConfig {
            url_depth: Some(1),
            ..config("/n/", true, Some(ctx))
        };
        assert_eq!(
            crate::link_transform::transform_link(
                &format!("../../{outside_name}/ledgeline/TODO.md"),
                &build
            ),
            "../ledgeline/TODO/"
        );
        assert_eq!(
            crate::link_transform::transform_link("~/x.md", &build),
            "../.mbr/outside/?path=~/x.md"
        );
    }

    #[test]
    fn outbound_links_record_mapped_targets_and_drop_outside_ones_from_internal() {
        use crate::link_index::{OutboundLink, resolve_outbound_links};
        let ctx = context("/home/me/Notes", "/home/me", vec![("/home/me/src", "code")]);
        let link = |to: &str| OutboundLink {
            to: to.to_string(),
            text: String::new(),
            anchor: None,
            internal: true,
        };
        let resolved = resolve_outbound_links(
            vec![link("~/src/a.md"), link("~/elsewhere/b.md"), link("c.md")],
            &config("/n/x/", false, Some(ctx)),
        );
        let got: Vec<(&str, bool)> = resolved
            .iter()
            .map(|l| (l.to.as_str(), l.internal))
            .collect();
        assert_eq!(
            got,
            vec![
                ("/code/a/", true),
                ("/.mbr/outside/?path=~/elsewhere/b.md", false),
                ("/n/c/", true),
            ]
        );
        // Without a context a `~` link is still never an internal page link.
        let plain = resolve_outbound_links(vec![link("~/b.md")], &config("/n/x/", false, None));
        assert_eq!((plain[0].to.as_str(), plain[0].internal), ("~/b.md", false));
    }

    proptest! {
        /// A chain that never climbs above the page's folder depth stays
        /// inside the repository.
        #[test]
        fn prop_chain_within_root_is_never_outside(
            folders in proptest::collection::vec("[a-z]{1,6}", 0..5),
            ups in 0usize..5,
            tail in proptest::collection::vec("[a-z]{1,6}", 0..3),
        ) {
            let ups = ups.min(folders.len());
            let page = format!("/{}/", folders.join("/"));
            let url = format!("{}{}", "../".repeat(ups), tail.join("/"));
            prop_assert_eq!(classify(&url, &page, true), None);
        }

        /// A chain one level deeper than the page's folder always is outside.
        #[test]
        fn prop_chain_past_root_is_outside(
            folders in proptest::collection::vec("[a-z]{1,6}", 0..5),
            extra in 1usize..4,
            tail in proptest::collection::vec("[a-z]{1,6}", 0..3),
        ) {
            let page = format!("/{}/", folders.join("/"));
            let url = format!("{}{}", "../".repeat(folders.len() + extra), tail.join("/"));
            let target = classify(&url, &page, true).unwrap();
            prop_assert_eq!(target.up, extra);
            prop_assert_eq!(target.base, OutsideBase::RepositoryRoot);
        }

        /// Query and fragment never change the classification.
        #[test]
        fn prop_query_and_fragment_are_ignored(
            path in "(~|\\.\\.|[a-z]{1,4})(/(\\.\\.|[a-z]{1,4}))*",
            page in "(/[a-z]{1,4}){0,3}/",
            suffix in "(\\?[a-z=]{0,6})?(#[a-z]{0,6})?",
            is_index in any::<bool>(),
        ) {
            prop_assert_eq!(
                classify(&path, &page, is_index),
                classify(&format!("{path}{suffix}"), &page, is_index)
            );
        }
    }
}
