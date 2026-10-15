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

    /// Asks `vault` to download every file this stamp found not downloaded,
    /// and returns how many that was. Once they arrive, the next stamp
    /// differs and a refresh acts on it (reloads the templates, rebuilds for
    /// the config, tells the app to reload the page).
    pub(crate) fn request_downloads(&self, vault: &dyn Vault) -> u32 {
        if self.not_downloaded.is_empty() {
            return 0;
        }
        vault.request_download(&self.not_downloaded);
        let count = u32::try_from(self.not_downloaded.len()).unwrap_or(u32::MAX);
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
}
