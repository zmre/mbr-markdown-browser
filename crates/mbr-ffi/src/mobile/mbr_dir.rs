//! What [`super::MbrApp::refresh`] needs to notice in the vault's `.mbr/`
//! folder (and a configured `template_folder`): the configuration, the
//! templates, and the asset overrides (`theme.css`, `user.css`,
//! `components/*.js`, …).
//!
//! The repository scan never looks inside `.mbr/` (it skips hidden folders),
//! and the app has no file watcher, so a refresh compares a **stat-only**
//! fingerprint — size, modification time and downloaded-or-not per file, no
//! content read — taken at open with one taken now. Only what changed is
//! acted on:
//!
//! | Changed | What the app does |
//! |---------|-------------------|
//! | `config.toml` | re-reads it and rebuilds the server (see `MbrApp::rebuild`) |
//! | a `*.html` template | reloads the templates in place, as the desktop watcher does |
//! | anything else (CSS, JS, images) | nothing server-side: `/.mbr/*` assets are read per request with `no-cache`, so the page reload the app does anyway picks them up |
//!
//! For the same reason nothing ever asks a cloud provider for a `.mbr/` file
//! it has not downloaded (an iCloud Drive vault's `user.css`, say). Each stamp
//! lists those and [`MbrStamp::request_downloads`] asks for them, at open and
//! on every refresh until they arrive; their arrival is a change like any
//! other.
//!
//! A change is acted on only once its bytes are on the device
//! ([`MbrStamp::settle`]). An edit made on another device reaches an iCloud
//! Drive vault as a *dataless* file — new size and time, contents still in the
//! cloud — and reading it on mbr's fail-fast threads gives `EDEADLK`. Acting
//! on it then turned a perfectly good `config.toml` into "could not be read"
//! and the defaults. So a changed file that is not downloaded keeps its last
//! stamp until it arrives, and its arrival is the change.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use mbr_core::vault::{Availability, Entry, LocalVault, Vault, VaultPath};

/// Folders deeper than this under `.mbr/` are not looked at. Templates are
/// loaded from `**/*.html`, but in practice they and the asset overrides sit
/// at most a level or two down (`components/x.js`).
const MAX_DEPTH: usize = 4;

/// An upper bound on the files stamped, so a `.mbr/` holding something large
/// (a built site, say) cannot make every refresh slow.
const MAX_FILES: usize = 4096;

/// Size, modification time and whether the bytes are on the device: enough
/// to tell an edit, an atomic replace, a sync or a finished download from
/// nothing having happened. A dataless iCloud file keeps its size and time
/// when it downloads, so without the third field its arrival would go unseen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileStamp {
    size: u64,
    modified: SystemTime,
    downloaded: bool,
}

impl FileStamp {
    fn of(entry: &Entry) -> Self {
        Self {
            size: entry.size,
            modified: entry.modified,
            downloaded: entry.availability != Availability::NotDownloaded,
        }
    }
}

/// The fingerprint of `.mbr/` and the template folder at one moment.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct MbrStamp {
    config: Option<FileStamp>,
    templates: BTreeMap<PathBuf, FileStamp>,
    assets: BTreeMap<PathBuf, FileStamp>,
    /// Files in the vault that are listed but not downloaded: what
    /// [`Self::request_downloads`] asks the provider for. Nothing else would:
    /// the scan skips hidden folders, so no page load ever reads them.
    not_downloaded: Vec<VaultPath>,
}

/// What a refresh may act on now: see [`MbrStamp::settle`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Settled {
    /// The next "last look": the stamp taken, except that what changed but
    /// is not on the device yet keeps the earlier stamp, so its arrival is
    /// still a change.
    pub(crate) stamp: MbrStamp,
    /// What changed and can be acted on.
    pub(crate) changes: MbrChanges,
    /// Something changed that is not downloaded yet and was held back.
    pub(crate) waiting: bool,
}

/// Which kinds of file changed between two stamps.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct MbrChanges {
    pub(crate) config: bool,
    pub(crate) templates: bool,
    pub(crate) assets: bool,
}

impl MbrChanges {
    pub(crate) fn any(self) -> bool {
        self.config || self.templates || self.assets
    }
}

impl MbrStamp {
    /// Stamps `.mbr/` in `vault` and, when configured, `template_folder`
    /// (relative to the vault's root unless absolute). Missing folders stamp
    /// as empty. Stats only: listed through the vault, so a dataless file —
    /// or a legacy `.Name.icloud` placeholder — is seen as not downloaded.
    ///
    /// A template folder outside the vault is listed with a plain
    /// [`LocalVault`]: the app has no grant (and no coordinator) there, so
    /// nothing in it is reported or requested as not downloaded.
    pub(crate) fn take(vault: &dyn Vault, template_folder: Option<&Path>) -> Self {
        let mut stamp = Self::default();
        let Ok(mbr) = VaultPath::new(".mbr") else {
            return stamp;
        };
        if let Ok(config) = mbr.child("config.toml")
            && let Ok(Some(entry)) = vault.stat(&config)
            && entry.is_file()
        {
            stamp.note_availability(&entry);
            stamp.config = Some(FileStamp::of(&entry));
        }
        let mut budget = MAX_FILES;
        stamp.walk(vault, &mbr, 0, &mut budget, true);
        if let Some(folder) = template_folder {
            let root = vault.root();
            let folder = if folder.is_absolute() {
                folder.to_path_buf()
            } else {
                root.join(folder)
            };
            match vault.vault_path(&folder) {
                Ok(inside) if inside == mbr => {}
                Ok(inside) => stamp.walk(vault, &inside, 0, &mut budget, true),
                Err(_) => {
                    let outside = LocalVault::new(&folder);
                    stamp.walk(&outside, &VaultPath::root(), 0, &mut budget, false);
                }
            }
        }
        stamp
    }

    /// What changed since `earlier`. A file that appeared or disappeared, or
    /// finished downloading, is a change of its kind.
    pub(crate) fn changes_since(&self, earlier: &Self) -> MbrChanges {
        MbrChanges {
            config: self.config != earlier.config,
            templates: self.templates != earlier.templates,
            assets: self.assets != earlier.assets,
        }
    }

    /// What can be acted on since `seen` (the stamp last acted on), holding
    /// back what is not on the device yet:
    ///
    /// - **`config.toml` and templates** are read server-side, together (a
    ///   rebuild reads both), so while any of them is not downloaded neither
    ///   kind moves: a refresh would otherwise read `EDEADLK` (a "broken"
    ///   configuration, a missing template) for a file that is merely in the
    ///   cloud.
    /// - **Assets** are held file by file: a changed `user.css` still in the
    ///   cloud keeps its earlier stamp, so the page is reloaded when it
    ///   arrives rather than before. An evicted file (same size and time,
    ///   no longer downloaded) is held the same way, and is no change at all
    ///   once it is back.
    ///
    /// A file that was already not downloaded at the last look and still is
    /// is no change, held or not.
    pub(crate) fn settle(&self, seen: &Self) -> Settled {
        let reads_pending = self.config.is_some_and(|c| !c.downloaded)
            || self.templates.values().any(|t| !t.downloaded);
        let (config, templates) = if reads_pending {
            (seen.config, seen.templates.clone())
        } else {
            (self.config, self.templates.clone())
        };
        let assets = self
            .assets
            .iter()
            .filter_map(|(path, stamp)| {
                let earlier = seen.assets.get(path);
                if stamp.downloaded || earlier == Some(stamp) {
                    Some((path.clone(), *stamp))
                } else {
                    earlier.map(|earlier| (path.clone(), *earlier))
                }
            })
            .collect();
        let stamp = Self {
            config,
            templates,
            assets,
            not_downloaded: self.not_downloaded.clone(),
        };
        let waiting = stamp.config != self.config
            || stamp.templates != self.templates
            || stamp.assets != self.assets;
        Settled {
            changes: stamp.changes_since(seen),
            stamp,
            waiting,
        }
    }

    /// This stamp with `seen`'s configuration and templates: for when a
    /// rebuild found `config.toml` not downloaded after all (it was evicted
    /// between the stamp and the read), so the next refresh tries again.
    pub(crate) fn with_reads_from(&self, seen: &Self) -> Self {
        Self {
            config: seen.config,
            templates: seen.templates.clone(),
            ..self.clone()
        }
    }

    /// How many files this stamp found not downloaded.
    pub(crate) fn not_downloaded_count(&self) -> u32 {
        u32::try_from(self.not_downloaded.len()).unwrap_or(u32::MAX)
    }

    /// Asks `vault` to download every file this stamp found not downloaded,
    /// and returns how many that was. Once they arrive, the next stamp
    /// differs and a refresh acts on it (reloads the templates, rebuilds for
    /// the config, tells the app to reload the page).
    pub(crate) fn request_downloads(&self, vault: &dyn Vault) -> u32 {
        if self.not_downloaded.is_empty() {
            return 0;
        }
        vault.request_download(&self.not_downloaded);
        let count = self.not_downloaded_count();
        tracing::info!(
            "Requested download of {count} not-downloaded file(s) in {}",
            vault.root().join(".mbr").display()
        );
        count
    }

    fn note_availability(&mut self, entry: &Entry) {
        if entry.availability == Availability::NotDownloaded {
            self.not_downloaded.push(entry.path.clone());
        }
    }

    /// `request`: whether not-downloaded files here are `vault`'s to request.
    fn walk(
        &mut self,
        vault: &dyn Vault,
        dir: &VaultPath,
        depth: usize,
        budget: &mut usize,
        request: bool,
    ) {
        // Links followed, as the template loader and asset server do.
        let Ok(entries) = vault.list_dir(dir) else {
            return;
        };
        for entry in entries {
            if *budget == 0 {
                return;
            }
            if entry.is_dir() {
                if depth + 1 < MAX_DEPTH {
                    self.walk(vault, &entry.path, depth + 1, budget, request);
                }
                continue;
            }
            if !entry.is_file() {
                continue;
            }
            *budget -= 1;
            if depth == 0 && entry.name() == "config.toml" {
                continue; // stamped on its own
            }
            if request {
                self.note_availability(&entry);
            }
            let stamp = FileStamp::of(&entry);
            let path = vault.key(&entry.path);
            if entry.path.extension() == Some("html") {
                self.templates.insert(path, stamp);
            } else {
                self.assets.insert(path, stamp);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mbr_core::vault::MemVault;

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn take(root: &Path, template_folder: Option<&Path>) -> MbrStamp {
        MbrStamp::take(&LocalVault::new(root), template_folder)
    }

    fn vault() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join(".mbr/config.toml"), "theme = \"amber\"\n");
        write(&dir.path().join(".mbr/index.html"), "<html></html>");
        write(&dir.path().join(".mbr/user.css"), "body {}");
        write(&dir.path().join(".mbr/components/x.js"), "1");
        write(&dir.path().join("note.md"), "# Note");
        dir
    }

    #[test]
    fn nothing_changed_is_no_change() {
        let dir = vault();
        let a = take(dir.path(), None);
        let b = take(dir.path(), None);
        assert_eq!(b.changes_since(&a), MbrChanges::default());
        assert!(!b.changes_since(&a).any());
        assert_eq!(a.templates.len(), 1);
        assert_eq!(a.assets.len(), 2, "user.css and components/x.js");
        assert!(a.config.is_some());
    }

    #[test]
    fn each_kind_of_change_is_told_apart() {
        let dir = vault();
        let before = take(dir.path(), None);
        write(&dir.path().join(".mbr/user.css"), "body { color: red }");
        let after = take(dir.path(), None);
        assert_eq!(
            after.changes_since(&before),
            MbrChanges {
                assets: true,
                ..MbrChanges::default()
            }
        );

        write(&dir.path().join(".mbr/_nav.html"), "<nav></nav>");
        let templates = take(dir.path(), None);
        assert!(templates.changes_since(&after).templates);
        assert!(!templates.changes_since(&after).config);

        write(&dir.path().join(".mbr/config.toml"), "theme = \"blue\"\n");
        let config = take(dir.path(), None);
        assert_eq!(
            config.changes_since(&templates),
            MbrChanges {
                config: true,
                ..MbrChanges::default()
            }
        );

        std::fs::remove_file(dir.path().join(".mbr/config.toml")).unwrap();
        assert!(take(dir.path(), None).changes_since(&config).config);
    }

    #[test]
    fn notes_outside_mbr_are_not_its_business() {
        let dir = vault();
        let before = take(dir.path(), None);
        write(&dir.path().join("note.md"), "# Edited");
        write(&dir.path().join("docs/index.html"), "<p>not a template</p>");
        assert!(!take(dir.path(), None).changes_since(&before).any());
    }

    #[test]
    fn a_template_folder_is_stamped_too() {
        let dir = vault();
        write(&dir.path().join("tmpl/index.html"), "<html>1</html>");
        let before = take(dir.path(), Some(Path::new("tmpl")));
        write(&dir.path().join("tmpl/index.html"), "<html>two</html>");
        let after = take(dir.path(), Some(Path::new("tmpl")));
        assert!(after.changes_since(&before).templates);
    }

    #[test]
    fn a_missing_mbr_folder_stamps_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(take(dir.path(), None), MbrStamp::default());
    }

    #[test]
    fn a_template_folder_outside_the_vault_is_stamped_but_never_requested() {
        let vault = vault();
        let outside = tempfile::tempdir().unwrap();
        write(&outside.path().join("index.html"), "<html>1</html>");
        let before = take(vault.path(), Some(outside.path()));
        assert_eq!(before.templates.len(), 2, "{before:?}");
        assert!(before.not_downloaded.is_empty());
        write(&outside.path().join("index.html"), "<html>two</html>");
        assert!(
            take(vault.path(), Some(outside.path()))
                .changes_since(&before)
                .templates
        );
    }

    /// A dataless `.mbr/` (iCloud Drive on iOS): every file is listed but not
    /// downloaded, so the stamp asks for each, and a file arriving — same
    /// size, same time, only now local — is a change of its kind.
    #[test]
    fn not_downloaded_files_are_requested_and_their_arrival_is_a_change() {
        let vault = MemVault::new()
            .with_file(".mbr/config.toml", "theme = \"amber\"\n")
            .with_file(".mbr/index.html", "<html></html>")
            .with_file(".mbr/user.css", "body {}")
            .with_file(".mbr/slides.svg", "<svg/>")
            .with_file("note.md", "# Note");
        for path in [".mbr/config.toml", ".mbr/index.html", ".mbr/user.css"] {
            vault.set_availability(path, Availability::NotDownloaded);
        }
        vault.set_availability("note.md", Availability::NotDownloaded);
        let before = MbrStamp::take(&vault, None);
        let mut pending: Vec<String> = before
            .not_downloaded
            .iter()
            .map(ToString::to_string)
            .collect();
        pending.sort();
        assert_eq!(
            pending,
            vec![".mbr/config.toml", ".mbr/index.html", ".mbr/user.css"],
            "only .mbr/, and only what is not downloaded"
        );
        assert_eq!(MbrStamp::take(&vault, None), before, "stable while pending");

        // MemVault downloads on request; the next stamp sees all three local.
        assert_eq!(before.request_downloads(&vault), 3);
        assert_eq!(vault.download_requests().len(), 3);
        let after = MbrStamp::take(&vault, None);
        assert!(after.not_downloaded.is_empty());
        assert_eq!(after.request_downloads(&vault), 0);
        assert_eq!(vault.download_requests().len(), 3, "nothing more asked for");
        assert_eq!(
            after.changes_since(&before),
            MbrChanges {
                config: true,
                templates: true,
                assets: true,
            }
        );
    }

    /// A remote edit, as an iCloud Drive vault on iOS sees it: new size and
    /// time, contents still in the cloud.
    fn edit_remotely(vault: &MemVault, path: &str, text: &str) {
        vault.insert_file(path, text);
        vault.set_modified(path, SystemTime::now() + std::time::Duration::from_secs(5));
        vault.set_availability(path, Availability::NotDownloaded);
    }

    fn local_mbr() -> MemVault {
        MemVault::new()
            .with_file(".mbr/config.toml", "title_prefix = \"OLD \"\n")
            .with_file(".mbr/index.html", "<html></html>")
            .with_file(".mbr/user.css", "body {}")
            .with_file(".mbr/theme.css", "html {}")
    }

    /// Regression (iPhone, iCloud vault): `config.toml` edited on a Mac was
    /// acted on while dataless, read as `EDEADLK`, and the vault fell back to
    /// the defaults with a "could not be read" notice. It is held until it
    /// arrives, and its arrival is the change.
    #[test]
    fn a_changed_config_still_in_the_cloud_is_held_until_it_arrives() {
        let vault = local_mbr();
        vault.hold_downloads(true);
        let seen = MbrStamp::take(&vault, None);
        edit_remotely(&vault, ".mbr/config.toml", "title_prefix = \"TEST \"\n");

        let pending = MbrStamp::take(&vault, None).settle(&seen);
        assert!(pending.waiting);
        assert_eq!(pending.changes, MbrChanges::default(), "nothing to act on");
        assert_eq!(pending.stamp.config, seen.config, "the last look stays");
        assert_eq!(pending.stamp.not_downloaded_count(), 1, "still requested");

        // A later refresh, still pending, is the same.
        let again = MbrStamp::take(&vault, None).settle(&pending.stamp);
        assert!(again.waiting && !again.changes.any());

        vault.set_availability(".mbr/config.toml", Availability::Local);
        let arrived = MbrStamp::take(&vault, None).settle(&again.stamp);
        assert!(!arrived.waiting);
        assert_eq!(
            arrived.changes,
            MbrChanges {
                config: true,
                ..MbrChanges::default()
            }
        );
        assert_eq!(arrived.stamp.not_downloaded_count(), 0);
    }

    /// A rebuild reads the templates as well as the configuration, so a
    /// template still in the cloud holds a configuration change too.
    #[test]
    fn a_template_in_the_cloud_holds_the_configuration_back_too() {
        let vault = local_mbr();
        let seen = MbrStamp::take(&vault, None);
        edit_remotely(&vault, ".mbr/index.html", "<html>new</html>");
        vault.insert_file(".mbr/config.toml", "title_prefix = \"NEW \"\n");
        vault.set_modified(
            ".mbr/config.toml",
            SystemTime::now() + std::time::Duration::from_secs(5),
        );
        let pending = MbrStamp::take(&vault, None).settle(&seen);
        assert!(pending.waiting);
        assert!(!pending.changes.config && !pending.changes.templates);

        vault.set_availability(".mbr/index.html", Availability::Local);
        let arrived = MbrStamp::take(&vault, None).settle(&pending.stamp);
        assert!(arrived.changes.config && arrived.changes.templates);
    }

    /// Assets are held one by one: a downloaded `theme.css` edit is acted on
    /// while a `user.css` edit waits for its bytes.
    #[test]
    fn assets_are_held_file_by_file() {
        let vault = local_mbr();
        let seen = MbrStamp::take(&vault, None);
        edit_remotely(&vault, ".mbr/user.css", "main h1 { color: red }");
        vault.insert_file(".mbr/theme.css", "html { color: blue }");
        let first = MbrStamp::take(&vault, None).settle(&seen);
        assert!(first.waiting);
        assert!(first.changes.assets, "theme.css is here");
        assert!(!first.changes.config && !first.changes.templates);

        let nothing_new = MbrStamp::take(&vault, None).settle(&first.stamp);
        assert!(nothing_new.waiting && !nothing_new.changes.any());

        vault.set_availability(".mbr/user.css", Availability::Local);
        let arrived = MbrStamp::take(&vault, None).settle(&nothing_new.stamp);
        assert!(!arrived.waiting && arrived.changes.assets);
    }

    /// An evicted file (same size and time, bytes gone) is waited for, and is
    /// no change once it is back.
    #[test]
    fn an_eviction_is_no_change() {
        let vault = local_mbr();
        let seen = MbrStamp::take(&vault, None);
        vault.set_availability(".mbr/user.css", Availability::NotDownloaded);
        vault.set_availability(".mbr/config.toml", Availability::NotDownloaded);
        let evicted = MbrStamp::take(&vault, None).settle(&seen);
        assert!(!evicted.changes.any());
        vault.set_availability(".mbr/user.css", Availability::Local);
        vault.set_availability(".mbr/config.toml", Availability::Local);
        let back = MbrStamp::take(&vault, None).settle(&evicted.stamp);
        assert!(!back.waiting && !back.changes.any(), "{back:?}");
    }

    #[test]
    fn with_reads_from_restores_only_config_and_templates() {
        let vault = local_mbr();
        let seen = MbrStamp::take(&vault, None);
        vault.insert_file(".mbr/config.toml", "title_prefix = \"X \"\n");
        vault.insert_file(".mbr/index.html", "<html>x</html>");
        vault.insert_file(".mbr/user.css", "body { x: y }");
        let now = MbrStamp::take(&vault, None);
        let reverted = now.with_reads_from(&seen);
        assert_eq!(
            reverted.changes_since(&seen),
            MbrChanges {
                assets: true,
                ..MbrChanges::default()
            }
        );
    }
}
