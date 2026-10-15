//! What [`super::MbrApp::refresh`] needs to notice in the vault's `.mbr/`
//! folder (and a configured `template_folder`): the configuration, the
//! templates, and the asset overrides (`theme.css`, `user.css`,
//! `components/*.js`, …).
//!
//! The repository scan never looks inside `.mbr/` (it skips hidden folders),
//! and the app has no file watcher, so a refresh compares a **stat-only**
//! fingerprint — size and modification time per file, no content read — taken
//! at open with one taken now. Only what changed is acted on:
//!
//! | Changed | What the app does |
//! |---------|-------------------|
//! | `config.toml` | re-reads it and rebuilds the server (see `MbrApp::rebuild`) |
//! | a `*.html` template | reloads the templates in place, as the desktop watcher does |
//! | anything else (CSS, JS, images) | nothing server-side: `/.mbr/*` assets are read per request with `no-cache`, so the page reload the app does anyway picks them up |

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Folders deeper than this under `.mbr/` are not looked at. Templates are
/// loaded from `**/*.html`, but in practice they and the asset overrides sit
/// at most a level or two down (`components/x.js`).
const MAX_DEPTH: usize = 4;

/// An upper bound on the files stamped, so a `.mbr/` holding something large
/// (a built site, say) cannot make every refresh slow.
const MAX_FILES: usize = 4096;

/// Size and modification time of one file: enough to tell an edit, an atomic
/// replace or a sync from nothing having happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileStamp {
    size: u64,
    modified: Option<SystemTime>,
}

impl FileStamp {
    fn of(metadata: &std::fs::Metadata) -> Self {
        Self {
            size: metadata.len(),
            modified: metadata.modified().ok(),
        }
    }
}

/// The fingerprint of `.mbr/` and the template folder at one moment.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct MbrStamp {
    config: Option<FileStamp>,
    templates: BTreeMap<PathBuf, FileStamp>,
    assets: BTreeMap<PathBuf, FileStamp>,
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
    /// Stamps `root/.mbr/` and, when configured, `template_folder` (relative
    /// to `root` unless absolute). Missing folders stamp as empty. Stats only.
    pub(crate) fn take(root: &Path, template_folder: Option<&Path>) -> Self {
        let mut stamp = Self::default();
        let mbr = root.join(".mbr");
        stamp.config = std::fs::metadata(mbr.join("config.toml"))
            .ok()
            .filter(std::fs::Metadata::is_file)
            .map(|m| FileStamp::of(&m));
        let mut budget = MAX_FILES;
        stamp.walk(&mbr, 0, &mut budget);
        if let Some(folder) = template_folder {
            let folder = if folder.is_absolute() {
                folder.to_path_buf()
            } else {
                root.join(folder)
            };
            if folder != mbr {
                stamp.walk(&folder, 0, &mut budget);
            }
        }
        stamp
    }

    /// What changed since `earlier`. A file that appeared or disappeared is a
    /// change of its kind.
    pub(crate) fn changes_since(&self, earlier: &Self) -> MbrChanges {
        MbrChanges {
            config: self.config != earlier.config,
            templates: self.templates != earlier.templates,
            assets: self.assets != earlier.assets,
        }
    }

    fn walk(&mut self, dir: &Path, depth: usize, budget: &mut usize) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            if *budget == 0 {
                return;
            }
            let path = entry.path();
            // Follows symlinks, as the template loader and asset server do.
            let Ok(metadata) = std::fs::metadata(&path) else {
                continue;
            };
            if metadata.is_dir() {
                if depth + 1 < MAX_DEPTH {
                    self.walk(&path, depth + 1, budget);
                }
                continue;
            }
            if !metadata.is_file() {
                continue;
            }
            *budget -= 1;
            let is_config = depth == 0 && path.file_name().is_some_and(|n| n == "config.toml");
            if is_config {
                continue; // stamped on its own
            }
            let stamp = FileStamp::of(&metadata);
            if path.extension().is_some_and(|e| e == "html") {
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

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
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
        let a = MbrStamp::take(dir.path(), None);
        let b = MbrStamp::take(dir.path(), None);
        assert_eq!(b.changes_since(&a), MbrChanges::default());
        assert!(!b.changes_since(&a).any());
        assert_eq!(a.templates.len(), 1);
        assert_eq!(a.assets.len(), 2, "user.css and components/x.js");
        assert!(a.config.is_some());
    }

    #[test]
    fn each_kind_of_change_is_told_apart() {
        let dir = vault();
        let before = MbrStamp::take(dir.path(), None);
        write(&dir.path().join(".mbr/user.css"), "body { color: red }");
        let after = MbrStamp::take(dir.path(), None);
        assert_eq!(
            after.changes_since(&before),
            MbrChanges {
                assets: true,
                ..MbrChanges::default()
            }
        );

        write(&dir.path().join(".mbr/_nav.html"), "<nav></nav>");
        let templates = MbrStamp::take(dir.path(), None);
        assert!(templates.changes_since(&after).templates);
        assert!(!templates.changes_since(&after).config);

        write(&dir.path().join(".mbr/config.toml"), "theme = \"blue\"\n");
        let config = MbrStamp::take(dir.path(), None);
        assert_eq!(
            config.changes_since(&templates),
            MbrChanges {
                config: true,
                ..MbrChanges::default()
            }
        );

        std::fs::remove_file(dir.path().join(".mbr/config.toml")).unwrap();
        assert!(
            MbrStamp::take(dir.path(), None)
                .changes_since(&config)
                .config
        );
    }

    #[test]
    fn notes_outside_mbr_are_not_its_business() {
        let dir = vault();
        let before = MbrStamp::take(dir.path(), None);
        write(&dir.path().join("note.md"), "# Edited");
        write(&dir.path().join("docs/index.html"), "<p>not a template</p>");
        assert!(
            !MbrStamp::take(dir.path(), None)
                .changes_since(&before)
                .any()
        );
    }

    #[test]
    fn a_template_folder_is_stamped_too() {
        let dir = vault();
        write(&dir.path().join("tmpl/index.html"), "<html>1</html>");
        let before = MbrStamp::take(dir.path(), Some(Path::new("tmpl")));
        write(&dir.path().join("tmpl/index.html"), "<html>two</html>");
        let after = MbrStamp::take(dir.path(), Some(Path::new("tmpl")));
        assert!(after.changes_since(&before).templates);
    }

    #[test]
    fn a_missing_mbr_folder_stamps_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(MbrStamp::take(dir.path(), None), MbrStamp::default());
    }
}
