//! Read-only **symlink mounts**: a symlink inside a [`LocalVault`] whose
//! target lies outside it, served as a vault of its own.
//!
//! The case this exists for is `static/videos -> ~/Movies`: media the author
//! keeps elsewhere, linked into the site. Before mounts the link was refused
//! (the server answered 404, and the static build placed it anyway — the two
//! disagreed). A mount makes it reachable from both, safely:
//!
//! - **Addressed at the link.** A file at `<target>/clip.mp4` is the vault
//!   path `static/videos/clip.mp4` — the link's canonical parent plus its own
//!   name — so its index key, URL, `raw_path` and every other derived value are
//!   exactly what a real directory at that place would produce. Nothing
//!   downstream learns that the bytes live elsewhere.
//! - **No escape.** [`super::Vault::canonicalize`] maps a canonical path back
//!   into the vault only when it is under the root or under an accepted
//!   mount's target, so `..` and nested links cannot leave a mount. A nested
//!   link to somewhere else is judged as a mount of its own, by the same rules.
//! - **Each target once.** Two links to one target, or a link cycle, map to
//!   the *first* mount registered for it, so the scanner's visited set (keyed
//!   by canonical vault path) walks it once.
//! - **Refused targets** are those [`crate::config::external_folder_refusal`]
//!   names — the same policy the external static overlay answers to — plus
//!   targets that cannot be read. A refusal is logged once per link and the
//!   link stays `OutsideRoot`, exactly as before mounts.
//! - **Read-only.** Every write through the vault into a mount is refused with
//!   [`super::VaultError::ReadOnly`] ([`super::Vault::is_read_only`]).
//!
//! Mounts are discovered **lazily**, by `canonicalize`, the first time a path
//! through the link is resolved — the scanner's descent, a request, a walk.
//! That puts the cost on the error path only: a repository without links out
//! pays nothing, not a syscall per file and not a pre-scan.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError, RwLock};

use super::{VaultError, VaultPath};
use crate::config::external_folder_refusal_with_home;

/// One accepted mount.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mount {
    /// Where the link is: its canonical parent in the vault plus its own name.
    /// Every path inside the mount is this plus the path below the target.
    pub location: VaultPath,
    /// The link's canonical target.
    pub target: PathBuf,
    /// Whether the target is a directory. A file link is a mount of one file.
    pub is_dir: bool,
}

/// What a vault may mount: the policy inputs, fixed at construction.
#[derive(Debug, Clone, Default)]
pub struct MountPolicy {
    /// Canonical roots already being served besides the vault's own — the
    /// markdown root and the external static overlay. A mount may never
    /// contain one.
    pub other_roots: Vec<PathBuf>,
    /// `$HOME`, canonical. A mount may be neither it nor an ancestor of it.
    pub home: Option<PathBuf>,
}

impl MountPolicy {
    /// The policy for a vault served alongside `other_roots`, with `$HOME`
    /// read from the environment (`USERPROFILE` on Windows).
    pub fn new(other_roots: Vec<PathBuf>) -> Self {
        Self {
            other_roots,
            home: crate::config::home_dir().map(|home| home.canonicalize().unwrap_or(home)),
        }
    }

    /// Replaces `$HOME`, for tests.
    #[must_use]
    pub fn with_home(mut self, home: Option<PathBuf>) -> Self {
        self.home = home;
        self
    }

    /// Adds a served root a mount may not contain.
    #[must_use]
    pub fn with_other_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.other_roots.push(root.into());
        self
    }
}

/// The mounts of one vault, growing as links are discovered.
#[derive(Debug)]
pub(super) struct MountTable {
    policy: MountPolicy,
    /// Accepted mounts. Read on the `OutsideRoot` path and by writes; written
    /// once per discovered link.
    mounts: RwLock<Vec<Mount>>,
    /// Links already refused (and logged), so each is reported once.
    refused: Mutex<HashSet<VaultPath>>,
}

impl MountTable {
    pub(super) fn new(policy: MountPolicy) -> Self {
        Self {
            policy,
            mounts: RwLock::new(Vec::new()),
            refused: Mutex::new(HashSet::new()),
        }
    }

    pub(super) fn snapshot(&self) -> Vec<Mount> {
        self.mounts
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// The vault path of `canonical`: under `root`, or under the target of the
    /// accepted mount nearest to it (the longest target prefix, so a nested
    /// mount wins over an enclosing one). `None` when it is under neither.
    pub(super) fn map(&self, root: &Path, canonical: &Path) -> Option<VaultPath> {
        if canonical.starts_with(root) {
            return VaultPath::from_absolute(root, canonical).ok();
        }
        let mounts = self.mounts.read().unwrap_or_else(PoisonError::into_inner);
        let mount = mounts
            .iter()
            .filter(|mount| canonical.starts_with(&mount.target))
            .max_by_key(|mount| mount.target.components().count())?;
        let below = VaultPath::from_absolute(&mount.target, canonical).ok()?;
        Some(mount.location.join_path(&below))
    }

    /// Whether `path` is a mount's location or inside one, lexically.
    pub(super) fn contains(&self, path: &VaultPath) -> bool {
        self.mounts
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .any(|mount| path.starts_with(&mount.location))
    }

    /// Resolves `path`, whose canonical form `canonical` is under neither the
    /// root nor any accepted mount, by discovering the link it leaves through.
    ///
    /// Each turn finds the first prefix of `path` that leaves (its parent maps
    /// into the vault, it does not) and judges that link as a mount. A path can
    /// leave more than once — a link inside a mount to somewhere else — so the
    /// loop repeats, at most once per segment.
    pub(super) fn resolve(
        &self,
        root: &Path,
        native: impl Fn(&VaultPath) -> PathBuf,
        path: &VaultPath,
        mut canonical: PathBuf,
    ) -> Result<VaultPath, VaultError> {
        let outside = |canonical: &Path| VaultError::OutsideRoot {
            path: canonical.display().to_string(),
        };
        for _ in 0..=path.segments().count() {
            if let Some(mapped) = self.map(root, &canonical) {
                return Ok(mapped);
            }
            let (location, target) = self
                .escape_point(root, &native, path)?
                .ok_or_else(|| outside(&canonical))?;
            if !self.admit(root, &location, &target) {
                return Err(outside(&canonical));
            }
            canonical = native(path)
                .canonicalize()
                .map_err(|e| VaultError::from_io(path, e))?;
        }
        Err(outside(&canonical))
    }

    /// The first prefix of `path` that resolves out of the vault, as the
    /// link's location (canonical parent + name) and its canonical target.
    fn escape_point(
        &self,
        root: &Path,
        native: &impl Fn(&VaultPath) -> PathBuf,
        path: &VaultPath,
    ) -> Result<Option<(VaultPath, PathBuf)>, VaultError> {
        let mut parent = VaultPath::root();
        let mut prefix = VaultPath::root();
        for segment in path.segments() {
            prefix = prefix.child(segment)?;
            let canonical = native(&prefix)
                .canonicalize()
                .map_err(|e| VaultError::from_io(&prefix, e))?;
            match self.map(root, &canonical) {
                Some(mapped) => parent = mapped,
                None => return Ok(Some((parent.child(segment)?, canonical))),
            }
        }
        Ok(None)
    }

    /// Judges the link at `location` (in the vault rooted at `root`) to
    /// `target`, registering it when the policy allows. Returns whether
    /// `target` is now mounted — by this link or an earlier one.
    fn admit(&self, root: &Path, location: &VaultPath, target: &Path) -> bool {
        if self
            .mounts
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .any(|mount| mount.target == target)
        {
            return true;
        }
        if self
            .refused
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(location)
        {
            return false;
        }
        let link = location.to_native(root);
        match self.judge(root, target) {
            Ok(is_dir) => {
                let mut mounts = self.mounts.write().unwrap_or_else(PoisonError::into_inner);
                if !mounts.iter().any(|mount| mount.target == target) {
                    tracing::info!(
                        "Serving external folder {} via {} (read-only)",
                        target.display(),
                        link.display()
                    );
                    mounts.push(Mount {
                        location: location.clone(),
                        target: target.to_path_buf(),
                        is_dir,
                    });
                }
                true
            }
            Err(reason) => {
                let first = self
                    .refused
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .insert(location.clone());
                if first {
                    tracing::warn!(
                        "Not serving {} (a link to {}): {reason}",
                        link.display(),
                        target.display()
                    );
                }
                false
            }
        }
    }

    /// The policy verdict for `target`: `Ok(is_dir)` when it may be mounted.
    fn judge(&self, root: &Path, target: &Path) -> Result<bool, String> {
        let protected: Vec<&Path> = std::iter::once(root)
            .chain(self.policy.other_roots.iter().map(PathBuf::as_path))
            .collect();
        let metadata = fs::metadata(target).map_err(|e| format!("it cannot be read: {e}"))?;
        // One policy for both kinds: a *file* target can contain nothing, so
        // of the rules only the hidden-name one can ever refuse it.
        if let Some(refusal) =
            external_folder_refusal_with_home(target, &protected, self.policy.home.as_deref())
        {
            return Err(refusal.to_string());
        }
        if metadata.is_dir() {
            fs::read_dir(target).map_err(|e| format!("it cannot be read: {e}"))?;
        }
        Ok(metadata.is_dir())
    }
}

/// Real symlinks, so Unix only (Windows needs Developer Mode or elevation to
/// create one). Every temp dir here is made with a visible prefix: `tempfile`'s
/// default `.tmpXXXX` names are *hidden*, and a mount target under one is
/// refused by the hidden-directory rule — which is the rule working, not the
/// fixture.
#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::vault::{LocalVault, Vault, resolve_under, walk_files};
    use std::os::unix::fs::symlink;

    fn vp(s: &str) -> VaultPath {
        VaultPath::new(s).unwrap()
    }

    fn visible_tempdir() -> tempfile::TempDir {
        tempfile::Builder::new()
            .prefix("mbr-mount-test-")
            .tempdir()
            .unwrap()
    }

    /// `<base>/repo` (the vault, `docs/a.md`), `<base>/movies` (the target:
    /// `clip.mp4`, `sub/deep.mp4`, `.secret/key`, `.dotfile`, `notes.md`), a
    /// fake `<base>/home`, and `repo/videos -> movies`.
    struct Fixture {
        _dir: tempfile::TempDir,
        base: PathBuf,
        root: PathBuf,
        movies: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let dir = visible_tempdir();
            let base = dir.path().canonicalize().unwrap();
            let root = base.join("repo");
            let movies = base.join("movies");
            for d in [
                root.join("docs"),
                movies.join("sub"),
                movies.join(".secret"),
                base.join("home"),
            ] {
                fs::create_dir_all(d).unwrap();
            }
            fs::write(root.join("docs/a.md"), "# A").unwrap();
            fs::write(movies.join("clip.mp4"), "clip").unwrap();
            fs::write(movies.join("sub/deep.mp4"), "deep").unwrap();
            fs::write(movies.join(".secret/key"), "key").unwrap();
            fs::write(movies.join(".dotfile"), "dot").unwrap();
            fs::write(movies.join("notes.md"), "# Notes").unwrap();
            symlink(&movies, root.join("videos")).unwrap();
            Self {
                _dir: dir,
                base,
                root,
                movies,
            }
        }

        fn policy(&self) -> MountPolicy {
            MountPolicy::default().with_home(Some(self.base.join("home")))
        }

        fn vault(&self) -> LocalVault {
            LocalVault::with_mounts(&self.root, self.policy())
        }

        fn link(&self, target: impl AsRef<Path>, at: &str) {
            symlink(target, self.root.join(at)).unwrap();
        }
    }

    #[test]
    fn a_directory_link_out_is_served_at_the_link() {
        let fx = Fixture::new();
        let vault = fx.vault();
        assert_eq!(
            vault.canonicalize(&vp("videos/clip.mp4")).unwrap(),
            vp("videos/clip.mp4")
        );
        assert_eq!(
            vault.canonicalize(&vp("videos/sub/deep.mp4")).unwrap(),
            vp("videos/sub/deep.mp4")
        );
        assert_eq!(
            vault.mounts(),
            vec![Mount {
                location: vp("videos"),
                target: fx.movies.clone(),
                is_dir: true,
            }]
        );
        // Keyed, located and read exactly like an in-root file.
        let key = vault.key(&vp("videos/clip.mp4"));
        assert_eq!(key, fx.root.join("videos").join("clip.mp4"));
        assert_eq!(vault.vault_path(&key).unwrap(), vp("videos/clip.mp4"));
        assert_eq!(vault.read(&vp("videos/clip.mp4")).unwrap(), b"clip");
        // The resolver's containment answers, including the probe for a name
        // that does not exist yet.
        let root = VaultPath::root();
        assert_eq!(
            resolve_under(&vault, &root, &vp("videos/clip.mp4")),
            Some(vp("videos/clip.mp4"))
        );
        assert_eq!(
            resolve_under(&vault, &root, &vp("videos/missing.mp4")),
            Some(vp("videos/missing.mp4"))
        );
        assert!(vault.is_read_only(&vp("videos")));
        assert!(vault.is_read_only(&vp("videos/clip.mp4")));
        assert!(!vault.is_read_only(&vp("docs/a.md")));
        assert!(!vault.is_read_only(&vp("videosx/a.md")));
    }

    /// Without `with_mounts`, a link out stays refused — the rule every
    /// non-repository vault (QuickLook, tests, link tools) keeps.
    #[test]
    fn a_plain_local_vault_still_refuses_links_out() {
        let fx = Fixture::new();
        let vault = LocalVault::new(&fx.root);
        assert!(matches!(
            vault.canonicalize(&vp("videos/clip.mp4")),
            Err(VaultError::OutsideRoot { .. })
        ));
        assert!(vault.mounts().is_empty());
    }

    #[test]
    fn a_nested_link_out_of_a_mount_is_a_mount_of_its_own() {
        let fx = Fixture::new();
        let extra = fx.base.join("extra");
        fs::create_dir(&extra).unwrap();
        fs::write(extra.join("x.mp4"), "x").unwrap();
        symlink(&extra, fx.movies.join("more")).unwrap();
        let vault = fx.vault();
        assert_eq!(
            vault.canonicalize(&vp("videos/more/x.mp4")).unwrap(),
            vp("videos/more/x.mp4")
        );
        let locations: Vec<String> = vault
            .mounts()
            .into_iter()
            .map(|m| m.location.to_string())
            .collect();
        assert_eq!(locations, vec!["videos", "videos/more"]);
    }

    /// `..` is resolved physically by the OS, after links: a link *to* the
    /// target's parent is judged like any other link — here it contains the
    /// repository root, so it is refused. (Climbing back *into* the root that
    /// way lands inside the vault and is simply a path in it.)
    #[test]
    fn a_link_climbing_out_of_a_mount_is_judged_not_followed() {
        let fx = Fixture::new();
        fs::write(fx.base.join("secret.txt"), "s").unwrap();
        symlink("..", fx.movies.join("up")).unwrap();
        let vault = fx.vault();
        assert!(matches!(
            vault.canonicalize(&vp("videos/up/secret.txt")),
            Err(VaultError::OutsideRoot { .. })
        ));
        assert_eq!(
            vault.canonicalize(&vp("videos/up/repo/docs/a.md")).unwrap(),
            vp("docs/a.md")
        );
        assert_eq!(vault.mounts().len(), 1, "only `videos` itself");
    }

    #[test]
    fn a_link_from_a_mount_back_into_the_repository_resolves_inside_it() {
        let fx = Fixture::new();
        symlink(fx.root.join("docs"), fx.movies.join("back")).unwrap();
        let vault = fx.vault();
        vault.canonicalize(&vp("videos/clip.mp4")).unwrap();
        assert_eq!(
            vault.canonicalize(&vp("videos/back/a.md")).unwrap(),
            vp("docs/a.md")
        );
        assert_eq!(vault.mounts().len(), 1, "no mount for a link into the root");
    }

    #[test]
    fn a_cycle_through_a_mount_maps_back_and_is_walked_once() {
        let fx = Fixture::new();
        symlink(&fx.movies, fx.movies.join("loop")).unwrap();
        symlink(fx.movies.join("sub"), fx.movies.join("sub/again")).unwrap();
        let vault = fx.vault();
        assert_eq!(
            vault
                .canonicalize(&vp("videos/loop/loop/clip.mp4"))
                .unwrap(),
            vp("videos/clip.mp4")
        );
        let mut found: Vec<String> = walk_files(&vault, |_| true)
            .into_iter()
            .map(|e| e.path.to_string())
            .collect();
        found.sort();
        assert_eq!(
            found,
            vec![
                "docs/a.md",
                "videos/.dotfile",
                "videos/.secret/key",
                "videos/clip.mp4",
                "videos/notes.md",
                "videos/sub/deep.mp4",
            ],
            "each directory once, under its canonical (mount) path"
        );
        assert_eq!(vault.mounts().len(), 1);
    }

    #[test]
    fn two_links_to_one_target_share_the_first_mount() {
        let fx = Fixture::new();
        fx.link(&fx.movies, "videos2");
        let vault = fx.vault();
        assert_eq!(
            vault.canonicalize(&vp("videos/clip.mp4")).unwrap(),
            vp("videos/clip.mp4")
        );
        assert_eq!(
            vault.canonicalize(&vp("videos2/clip.mp4")).unwrap(),
            vp("videos/clip.mp4"),
            "the second link resolves to the first mount's location"
        );
        assert_eq!(vault.mounts().len(), 1);
    }

    #[test]
    fn refused_targets_stay_outside_the_vault() {
        let fx = Fixture::new();
        fs::create_dir(fx.base.join(".hidden")).unwrap();
        fs::write(fx.base.join(".hidden/key"), "k").unwrap();
        fs::write(fx.base.join("secret.txt"), "s").unwrap();
        fs::write(fx.base.join("home/diary.md"), "d").unwrap();
        fx.link("/", "fsroot");
        fx.link(fx.base.join("home"), "home");
        fx.link(&fx.base, "ancestor");
        fx.link(fx.base.join(".hidden"), "hidden");
        fx.link(fx.base.join(".hidden/key"), "key.txt");
        let vault = fx.vault();
        for path in [
            "fsroot/etc/hosts",
            "home/diary.md",
            "ancestor/secret.txt",
            "hidden/key",
            "key.txt",
        ] {
            assert!(
                matches!(
                    vault.canonicalize(&vp(path)),
                    Err(VaultError::OutsideRoot { .. })
                ),
                "{path} must stay refused"
            );
            assert_eq!(
                resolve_under(&vault, &VaultPath::root(), &vp(path)),
                None,
                "{path}"
            );
        }
        assert!(vault.mounts().is_empty(), "{:?}", vault.mounts());
    }

    /// The hidden rule only counts components below the point the target
    /// shares with the root, so a repository living inside a hidden directory
    /// can still mount a sibling there.
    #[test]
    fn a_repository_inside_a_hidden_directory_can_mount_a_sibling() {
        let dir = visible_tempdir();
        let base = dir.path().canonicalize().unwrap().join(".notes");
        fs::create_dir_all(base.join("repo")).unwrap();
        fs::create_dir_all(base.join("media")).unwrap();
        fs::write(base.join("media/x.png"), "x").unwrap();
        symlink(base.join("media"), base.join("repo/media")).unwrap();
        let vault = LocalVault::with_mounts(base.join("repo"), MountPolicy::default());
        assert_eq!(
            vault.canonicalize(&vp("media/x.png")).unwrap(),
            vp("media/x.png")
        );
    }

    #[test]
    fn an_unreadable_target_is_refused_not_fatal() {
        use std::os::unix::fs::PermissionsExt;
        let fx = Fixture::new();
        let locked = fx.base.join("locked");
        fs::create_dir(&locked).unwrap();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
        fx.link(&locked, "locked");
        // Root ignores the mode; there is nothing to assert then.
        if fs::read_dir(&locked).is_err() {
            let vault = fx.vault();
            assert!(vault.canonicalize(&vp("locked")).is_err());
            assert!(vault.mounts().is_empty());
        }
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn a_file_link_out_is_a_read_only_file_mount() {
        let fx = Fixture::new();
        fx.link(fx.movies.join("clip.mp4"), "demo.mp4");
        let vault = fx.vault();
        assert_eq!(vault.canonicalize(&vp("demo.mp4")).unwrap(), vp("demo.mp4"));
        assert_eq!(
            vault.mounts(),
            vec![Mount {
                location: vp("demo.mp4"),
                target: fx.movies.join("clip.mp4"),
                is_dir: false,
            }]
        );
        assert!(vault.is_read_only(&vp("demo.mp4")));
        assert!(matches!(
            vault.write_atomic(&vp("demo.mp4"), b"x"),
            Err(VaultError::ReadOnly { .. })
        ));
        assert_eq!(fs::read(fx.movies.join("clip.mp4")).unwrap(), b"clip");
    }

    #[test]
    fn writes_into_a_mount_are_refused_however_they_are_spelled() {
        let fx = Fixture::new();
        // An in-root alias of the root, so `alias/videos/...` reaches the
        // mount through a second link.
        fx.link(&fx.root, "alias");
        let read_only =
            |result: Result<(), VaultError>| matches!(result, Err(VaultError::ReadOnly { .. }));
        // A fresh vault: nothing discovered yet, so the write check itself has
        // to find the mount.
        let vault = fx.vault();
        assert!(read_only(vault.write_atomic(&vp("videos/new.md"), b"x")));
        let vault = fx.vault();
        assert!(read_only(
            vault.write_atomic(&vp("alias/videos/new.md"), b"x")
        ));
        assert!(read_only(vault.write_atomic(&vp("videos/clip.mp4"), b"x")));
        assert!(read_only(vault.create_dir_all(&vp("videos/newdir/deeper"))));
        assert!(read_only(vault.remove_file(&vp("videos/clip.mp4"))));
        assert!(read_only(vault.remove_file(&vp("videos"))));
        assert!(read_only(
            vault.rename(&vp("docs/a.md"), &vp("videos/a.md"))
        ));
        assert!(read_only(
            vault.rename(&vp("videos/clip.mp4"), &vp("docs/c.mp4"))
        ));
        assert!(!fx.movies.join("new.md").exists());
        assert!(!fx.movies.join("newdir").exists());
        assert!(!fx.movies.join("a.md").exists());
        assert_eq!(fs::read(fx.movies.join("clip.mp4")).unwrap(), b"clip");
        assert!(fx.root.join("docs/a.md").exists());
        assert!(fx.root.join("videos").exists(), "the link itself survives");
        // Writes elsewhere are untouched by any of this.
        vault.write_atomic(&vp("docs/b.md"), b"# B").unwrap();
        assert_eq!(fs::read(fx.root.join("docs/b.md")).unwrap(), b"# B");
    }

    /// A refused link can never be written through either: the write check
    /// canonicalizes, and the refusal is `OutsideRoot`.
    #[test]
    fn writes_through_a_refused_link_are_refused() {
        let fx = Fixture::new();
        fs::create_dir(fx.base.join(".hidden")).unwrap();
        fx.link(fx.base.join(".hidden"), "hidden");
        let vault = fx.vault();
        assert!(matches!(
            vault.write_atomic(&vp("hidden/planted"), b"x"),
            Err(VaultError::OutsideRoot { .. })
        ));
        assert!(!fx.base.join(".hidden/planted").exists());
    }

    #[test]
    fn the_shared_policy_refuses_each_kind_of_target() {
        use crate::config::{ExternalFolderRefusal as R, external_folder_refusal_with_home};
        let root = Path::new("/srv/sites/blog/content");
        let home = Some(Path::new("/home/me"));
        let refusal = |dir: &str| external_folder_refusal_with_home(Path::new(dir), &[root], home);
        assert_eq!(refusal("/"), Some(R::FilesystemRoot));
        assert_eq!(refusal("/home/me"), Some(R::HomeDir));
        assert_eq!(refusal("/home"), Some(R::HomeDir));
        assert_eq!(
            refusal("/srv/sites"),
            Some(R::ContainsRoot(root.to_path_buf()))
        );
        assert_eq!(refusal("/home/me/.ssh"), Some(R::Hidden(".ssh".into())));
        assert_eq!(
            refusal("/home/me/.config/app/media"),
            Some(R::Hidden(".config".into()))
        );
        assert_eq!(
            refusal("/srv/sites/blog/.git"),
            Some(R::Hidden(".git".into()))
        );
        assert_eq!(refusal("/home/me/Movies"), None);
        assert_eq!(refusal("/srv/media"), None);
        assert_eq!(refusal("/srv/sites/blog/static"), None);
    }
}
