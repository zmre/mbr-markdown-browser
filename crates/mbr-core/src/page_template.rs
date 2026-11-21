//! Per-note page templates: `template: invoice.html` in a note's frontmatter.
//!
//! A note may name a Tera template that replaces `index.html` for that one
//! page (an invoice, a letter, a CV — anything whose layout is its own). This
//! module is the **one** place that turns the frontmatter value into a
//! template file, so the server, the static build, QuickLook and anything else
//! that renders a note (PDF export) fail and succeed alike.
//!
//! # Resolution
//!
//! The value is a path relative to the note's folder (`invoice.html`,
//! `../templates/invoice.html`), or relative to the repository root when it
//! starts with `/`. It is resolved through the repository's [`Vault`] with
//! the containment the server applies to a request:
//!
//! - lexically first ([`VaultPath::join`]): `..` past the root is refused;
//! - no hidden segment (`.mbr/x.html`, `.git/…`), before and after symlinks
//!   are followed, except the hidden folders the operator named on the command
//!   line ([`crate::repo::Repo::exempt_hidden_dirs`]);
//! - [`resolve_under`] the vault root: a symlink out of the repository is
//!   refused unless the vault accepts it as a read-only mount;
//! - it must be an existing regular `.html` file.
//!
//! # Siblings
//!
//! The `.html` files **directly** in the template's folder are handed to the
//! renderer too (see [`PageTemplate::siblings`]), so a template can
//! `{% include "invoice_lines.html" %}` or `{% extends "base.html" %}` a file
//! next to it. Subfolders are not read: a template at the repository root
//! would otherwise pull in every `.html` file in the repository.
//!
//! # Not downloaded
//!
//! On a cloud vault (iCloud on iOS) the template or a sibling may be listed
//! but not on the device. The template itself is checked with
//! [`Vault::availability`] (no I/O on the desktop) and by its read, and fails
//! [`PageTemplateError::NotDownloaded`]. A sibling that is not downloaded is
//! not read and not requested — the template may never use it — but kept in
//! [`PageTemplate::pending_siblings`]; only when a render fails *naming* one
//! does the render report it as `NotDownloaded`
//! (`Templates::render_page_template_with_tera`). Callers that can wait (the
//! server) request the download and retry; the others fail as for any error.
//!
//! # Not published
//!
//! A file some note names as its template is page structure, not content: the
//! static build does not publish it, `media.json` and search leave it out, and
//! the server answers 404 for it ([`page_template_files`], memoized by
//! [`crate::repo::Repo::page_template_files`]).

use std::collections::HashSet;
use std::path::PathBuf;

use crate::markdown::SimpleMetadata;
use crate::vault::{Availability, Vault, VaultError, VaultPath, has_hidden_segment, resolve_under};

/// The frontmatter key naming a note's page template.
pub const PAGE_TEMPLATE_KEY: &str = "template";

/// The largest template (and sibling) file read, in bytes. A Tera template is
/// a few kilobytes; anything near this is not one.
pub const MAX_TEMPLATE_BYTES: u64 = 1024 * 1024;

/// At most this many sibling `.html` files are read from the template's
/// folder (the rest are ignored, in name order).
pub const MAX_SIBLING_TEMPLATES: usize = 256;

/// A note's page template, located, read, and ready to render.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageTemplate {
    /// The value as the note wrote it, for messages.
    pub authored: String,
    /// The template file, canonical (symlinks followed), in the repository's
    /// vault.
    pub path: VaultPath,
    /// The template's source.
    pub source: String,
    /// `(name, source)` of every other `.html` file directly in the
    /// template's folder, by file name, sorted. Files that could not be read
    /// or are too large are left out.
    pub siblings: Vec<(String, String)>,
    /// `(name, path)` of each sibling that is not downloaded (see the module
    /// docs): not read, not requested, and not registered for the render.
    pub pending_siblings: Vec<(String, VaultPath)>,
}

impl PageTemplate {
    /// The template's folder (the vault root for a template at the root).
    pub fn folder(&self) -> VaultPath {
        self.path.parent().unwrap_or_default()
    }
}

/// Why a note's `template` could not be used. Every variant names the value
/// as the note wrote it.
#[derive(Debug, thiserror::Error)]
pub enum PageTemplateError {
    /// `template:` is not a string (a number, a list, a map).
    #[error("`template` must be a path to an .html file, not {found}")]
    NotAString { found: String },

    /// `template:` is an empty string.
    #[error("`template` is empty; name an .html file")]
    Empty,

    /// The value does not name an `.html` file.
    #[error("template `{template}` is not an .html file")]
    NotHtml { template: String },

    /// The value climbs out of the repository (`..` past the root, an
    /// absolute filesystem path, a symlink out of the repository that is not
    /// an accepted mount).
    #[error("template `{template}` is outside the repository")]
    OutsideRoot { template: String },

    /// The value names a file in a hidden folder (`.mbr/`, `.git/`) or a
    /// hidden file.
    #[error("template `{template}` is in a hidden folder or is a hidden file")]
    Hidden { template: String },

    /// No such file.
    #[error("template `{template}` was not found")]
    NotFound { template: String },

    /// The file exists but is larger than [`MAX_TEMPLATE_BYTES`].
    #[error("template `{template}` is larger than {MAX_TEMPLATE_BYTES} bytes")]
    TooLarge { template: String },

    /// The template, or a sibling the render needs, is listed but its bytes
    /// are not on this device (a cloud vault). `files` are the vault paths to
    /// request ([`Vault::request_download`]) before trying again.
    #[error("template `{template}` is not downloaded yet")]
    NotDownloaded {
        template: String,
        files: Vec<VaultPath>,
    },

    /// The file exists but could not be read (permissions, invalid UTF-8).
    #[error("template `{template}` could not be read: {source}")]
    Unreadable {
        template: String,
        #[source]
        source: VaultError,
    },

    /// The template was found but Tera could not parse or render it.
    /// `message` holds Tera's whole error chain.
    #[error("template `{template}` failed to render: {message}")]
    Render { template: String, message: String },
}

impl PageTemplateError {
    /// Whether the note's own template could not be found or read — as
    /// opposed to a template that was found and is broken. Callers that map
    /// errors to statuses may use it; both are reported the same way today.
    pub fn is_resolution_error(&self) -> bool {
        !matches!(self, Self::Render { .. })
    }

    /// The files to download before trying again, when that is the problem.
    pub fn not_downloaded_files(&self) -> Option<&[VaultPath]> {
        match self {
            Self::NotDownloaded { files, .. } => Some(files),
            _ => None,
        }
    }
}

/// The note's requested template: `None` when the frontmatter has no
/// `template` key, else its string value or why it is not one.
///
/// A YAML `template:` with no value is a null, which the simplified
/// frontmatter drops, so it reads as "no template".
pub fn requested_template(frontmatter: &SimpleMetadata) -> Option<Result<&str, PageTemplateError>> {
    frontmatter.get(PAGE_TEMPLATE_KEY).map(|value| match value {
        serde_json::Value::String(s) if s.trim().is_empty() => Err(PageTemplateError::Empty),
        serde_json::Value::String(s) => Ok(s.as_str()),
        other => Err(PageTemplateError::NotAString {
            found: json_kind(other).to_string(),
        }),
    })
}

fn json_kind(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "a boolean",
        serde_json::Value::Number(_) => "a number",
        serde_json::Value::String(_) => "a string",
        serde_json::Value::Array(_) => "a list",
        serde_json::Value::Object(_) => "a map",
    }
}

/// Whether `name` has an `.html` extension (ASCII case-insensitive).
fn is_html_name(name: &str) -> bool {
    name.rsplit_once('.')
        .is_some_and(|(stem, ext)| !stem.is_empty() && ext.eq_ignore_ascii_case("html"))
}

/// Locates the template `template` names for the note at `note`, without
/// reading it: the canonical vault path of an existing `.html` file inside
/// the repository. See the [module docs](self) for the rules.
///
/// `exempt` is [`crate::repo::Repo::exempt_hidden_dirs`]: hidden folders the
/// operator named on the command line, which may hold templates.
pub fn locate_page_template(
    vault: &dyn Vault,
    note: &VaultPath,
    template: &str,
    exempt: &[VaultPath],
) -> Result<VaultPath, PageTemplateError> {
    let authored = template.trim();
    let owned = || authored.to_string();
    if authored.is_empty() {
        return Err(PageTemplateError::Empty);
    }

    // `/x.html` is relative to the repository root, as a root-relative link
    // is; anything else is relative to the note's folder. A `VaultPath` cannot
    // escape lexically, so a `..` past the root fails here.
    let lexical = match authored.strip_prefix('/') {
        Some(root_relative) => VaultPath::new(root_relative),
        None => note.parent().unwrap_or_default().join(authored),
    }
    .map_err(|_| PageTemplateError::OutsideRoot { template: owned() })?;

    if !lexical.file_name().is_some_and(is_html_name) {
        return Err(PageTemplateError::NotHtml { template: owned() });
    }
    if has_hidden_segment(&lexical, exempt) {
        return Err(PageTemplateError::Hidden { template: owned() });
    }

    // Symlinks: what the name resolves to must still be inside the vault (an
    // accepted mount counts) and still visible — a visible link to
    // `.mbr/index.html` is the same file as `.mbr/index.html`.
    let canonical = resolve_under(vault, &VaultPath::root(), &lexical).ok_or_else(|| {
        // `resolve_under` answers `None` both for a link out of the vault and
        // for a missing parent folder; tell them apart for the message.
        if vault.stat(&lexical).is_ok_and(|entry| entry.is_none()) {
            PageTemplateError::NotFound { template: owned() }
        } else {
            PageTemplateError::OutsideRoot { template: owned() }
        }
    })?;
    if has_hidden_segment(&canonical, exempt) {
        return Err(PageTemplateError::Hidden { template: owned() });
    }
    if !canonical.file_name().is_some_and(is_html_name) {
        return Err(PageTemplateError::NotHtml { template: owned() });
    }

    match vault.stat(&canonical) {
        Ok(Some(entry)) if entry.is_file() => Ok(canonical),
        Ok(Some(_)) => Err(PageTemplateError::NotHtml { template: owned() }),
        Ok(None) => Err(PageTemplateError::NotFound { template: owned() }),
        Err(e) if e.is_not_found() => Err(PageTemplateError::NotFound { template: owned() }),
        Err(source) => Err(PageTemplateError::Unreadable {
            template: owned(),
            source,
        }),
    }
}

/// Locates and reads the template `template` names for the note at `note`,
/// with the `.html` files beside it. The fail-fast entry point every renderer
/// uses; see [`locate_page_template`] for the rules and `exempt`.
pub fn resolve_page_template(
    vault: &dyn Vault,
    note: &VaultPath,
    template: &str,
    exempt: &[VaultPath],
) -> Result<PageTemplate, PageTemplateError> {
    let path = locate_page_template(vault, note, template, exempt)?;
    let authored = template.trim().to_string();
    let not_downloaded = || PageTemplateError::NotDownloaded {
        template: authored.clone(),
        files: vec![path.clone()],
    };
    // Free on the desktop (`LocalVault` answers without I/O); on a cloud
    // vault it saves a read that would only fail.
    if vault.availability(&path) == Availability::NotDownloaded {
        return Err(not_downloaded());
    }
    let source = read_template(vault, &path).map_err(|e| match e {
        ReadProblem::TooLarge => PageTemplateError::TooLarge {
            template: authored.clone(),
        },
        ReadProblem::Vault(VaultError::NotDownloaded { .. }) => not_downloaded(),
        ReadProblem::Vault(source) => PageTemplateError::Unreadable {
            template: authored.clone(),
            source,
        },
    })?;
    let Siblings { read, pending } = read_siblings(vault, &path, exempt);
    Ok(PageTemplate {
        authored,
        path,
        source,
        siblings: read,
        pending_siblings: pending,
    })
}

/// A page template from **unsaved** text: `source` as the contents of the
/// template file at `template` (from the repository root; a leading `/` is
/// allowed), for a template editor's live preview. Nothing is cached, so the
/// text never replaces what is on disk for any other render.
///
/// The path obeys the rules of [`locate_page_template`] (inside the
/// repository, not hidden, `.html`), but the file need not exist yet (a new
/// template). The siblings are read from disk as [`resolve_page_template`]
/// reads them — the template's own file excluded, so its saved version is not
/// registered beside the unsaved one.
pub fn unsaved_page_template(
    vault: &dyn Vault,
    template: &str,
    source: &str,
    exempt: &[VaultPath],
) -> Result<PageTemplate, PageTemplateError> {
    let rooted = format!("/{}", template.trim().trim_start_matches('/'));
    let path = match locate_page_template(vault, &VaultPath::root(), &rooted, exempt) {
        Ok(path) => path,
        // Not on disk (yet): the lexical checks all passed before the lookup,
        // so the lexical path is the template's.
        Err(PageTemplateError::NotFound { .. }) => {
            VaultPath::new(&rooted[1..]).map_err(|_| PageTemplateError::OutsideRoot {
                template: rooted.clone(),
            })?
        }
        Err(e) => return Err(e),
    };
    if u64::try_from(source.len()).unwrap_or(u64::MAX) > MAX_TEMPLATE_BYTES {
        return Err(PageTemplateError::TooLarge { template: rooted });
    }
    let Siblings { read, pending } = read_siblings(vault, &path, exempt);
    Ok(PageTemplate {
        authored: rooted,
        path,
        source: source.to_string(),
        siblings: read,
        pending_siblings: pending,
    })
}

/// [`read_siblings`]' answer.
struct Siblings {
    read: Vec<(String, String)>,
    pending: Vec<(String, VaultPath)>,
}

enum ReadProblem {
    TooLarge,
    Vault(VaultError),
}

fn read_template(vault: &dyn Vault, path: &VaultPath) -> Result<String, ReadProblem> {
    // A prefix one byte over the cap tells "at the cap" from "over it" in one
    // read, without a separate `stat` whose size a vault may not know.
    let cap = usize::try_from(MAX_TEMPLATE_BYTES).unwrap_or(usize::MAX);
    let bytes = vault
        .read_prefix(path, cap.saturating_add(1))
        .map_err(ReadProblem::Vault)?;
    if bytes.len() > cap {
        return Err(ReadProblem::TooLarge);
    }
    String::from_utf8(bytes).map_err(|e| {
        ReadProblem::Vault(VaultError::Io {
            path: path.to_string(),
            source: std::io::Error::new(std::io::ErrorKind::InvalidData, e),
        })
    })
}

/// The other `.html` files directly in `template`'s folder: one
/// [`Vault::list_dir`], visible regular files only, at most
/// [`MAX_SIBLING_TEMPLATES`], each at most [`MAX_TEMPLATE_BYTES`]. A sibling
/// that cannot be read is left out (and named in the debug log): it only
/// matters if the template uses it, and then Tera says it is missing. One the
/// listing (or its read) reports not downloaded is `pending` instead — never
/// read and never requested here.
fn read_siblings(vault: &dyn Vault, template: &VaultPath, exempt: &[VaultPath]) -> Siblings {
    let folder = template.parent().unwrap_or_default();
    let Ok(entries) = vault.list_dir(&folder) else {
        return Siblings {
            read: Vec::new(),
            pending: Vec::new(),
        };
    };
    let mut candidates: Vec<(VaultPath, Availability)> = entries
        .into_iter()
        .filter(|entry| {
            entry.is_file()
                && entry.path != *template
                && is_html_name(entry.name())
                && !has_hidden_segment(&entry.path, exempt)
        })
        .map(|entry| (entry.path, entry.availability))
        .collect();
    candidates.sort_by(|(a, _), (b, _)| a.cmp(b));
    let mut siblings = Siblings {
        read: Vec::new(),
        pending: Vec::new(),
    };
    for (path, availability) in candidates.into_iter().take(MAX_SIBLING_TEMPLATES) {
        let Some(name) = path.file_name().map(str::to_string) else {
            continue;
        };
        if availability == Availability::NotDownloaded {
            siblings.pending.push((name, path));
            continue;
        }
        match read_template(vault, &path) {
            Ok(source) => siblings.read.push((name, source)),
            Err(ReadProblem::Vault(VaultError::NotDownloaded { .. })) => {
                siblings.pending.push((name, path));
            }
            Err(_) => tracing::debug!("page template sibling {path} not read"),
        }
    }
    siblings
}

/// Every file some note names as its page template, by index key
/// ([`Vault::key`] of its canonical vault path) — what is not published or
/// served. `notes` yields each note's index key and frontmatter.
///
/// Only notes with a `template` key cost anything beyond a map lookup: one
/// [`locate_page_template`] each. A note whose template does not resolve
/// contributes nothing (there is no file to hide).
pub fn page_template_files<'a>(
    vault: &dyn Vault,
    notes: impl IntoIterator<Item = (&'a std::path::Path, Option<&'a SimpleMetadata>)>,
    exempt: &[VaultPath],
) -> HashSet<PathBuf> {
    notes
        .into_iter()
        .filter_map(|(key, frontmatter)| {
            let template = requested_template(frontmatter?)?.ok()?;
            let note = vault.vault_path(key).ok()?;
            locate_page_template(vault, &note, template, exempt).ok()
        })
        .map(|path| vault.key(&path))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::LocalVault;
    use std::path::Path;

    /// A repository with `files` written under a canonical temp root.
    fn repo(files: &[(&str, &str)]) -> (tempfile::TempDir, LocalVault) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        for (path, contents) in files {
            let file = root.join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, contents).unwrap();
        }
        let vault = LocalVault::new(&root);
        (dir, vault)
    }

    fn note(path: &str) -> VaultPath {
        VaultPath::new(path).unwrap()
    }

    fn resolve(vault: &LocalVault, n: &str, t: &str) -> Result<PageTemplate, PageTemplateError> {
        resolve_page_template(vault, &note(n), t, &[])
    }

    #[test]
    fn resolves_relative_to_the_note_folder() {
        let (_d, vault) = repo(&[
            ("invoices/2026-01.md", "x"),
            ("invoices/invoice.html", "<p>{{ title }}</p>"),
        ]);
        let t = resolve(&vault, "invoices/2026-01.md", "invoice.html").unwrap();
        assert_eq!(t.path.as_str(), "invoices/invoice.html");
        assert_eq!(t.source, "<p>{{ title }}</p>");
        assert_eq!(t.authored, "invoice.html");
        assert_eq!(t.folder().as_str(), "invoices");
    }

    #[test]
    fn resolves_nested_parent_and_root_relative_paths() {
        let (_d, vault) = repo(&[
            ("a/b/note.md", "x"),
            ("a/tpl/page.html", "p"),
            ("a/b/sub/inner.html", "i"),
            ("shared/t.html", "s"),
        ]);
        assert_eq!(
            resolve(&vault, "a/b/note.md", "sub/inner.html")
                .unwrap()
                .path
                .as_str(),
            "a/b/sub/inner.html"
        );
        assert_eq!(
            resolve(&vault, "a/b/note.md", "../tpl/page.html")
                .unwrap()
                .path
                .as_str(),
            "a/tpl/page.html"
        );
        assert_eq!(
            resolve(&vault, "a/b/note.md", "../../shared/t.html")
                .unwrap()
                .path
                .as_str(),
            "shared/t.html"
        );
        assert_eq!(
            resolve(&vault, "a/b/note.md", "/shared/t.html")
                .unwrap()
                .path
                .as_str(),
            "shared/t.html"
        );
    }

    #[test]
    fn refuses_escapes_hidden_missing_and_non_html() {
        let (_d, vault) = repo(&[
            ("n.md", "x"),
            (".mbr/index.html", "secret"),
            ("dir/.hidden.html", "h"),
            ("notes.txt", "t"),
            ("folder.html/x.md", "x"),
        ]);
        assert!(matches!(
            resolve(&vault, "n.md", "../outside.html"),
            Err(PageTemplateError::OutsideRoot { .. })
        ));
        assert!(matches!(
            resolve(&vault, "n.md", ".mbr/index.html"),
            Err(PageTemplateError::Hidden { .. })
        ));
        assert!(matches!(
            resolve(&vault, "n.md", "dir/.hidden.html"),
            Err(PageTemplateError::Hidden { .. })
        ));
        assert!(matches!(
            resolve(&vault, "n.md", "missing.html"),
            Err(PageTemplateError::NotFound { .. })
        ));
        assert!(matches!(
            resolve(&vault, "n.md", "nowhere/missing.html"),
            Err(PageTemplateError::NotFound { .. })
        ));
        assert!(matches!(
            resolve(&vault, "n.md", "notes.txt"),
            Err(PageTemplateError::NotHtml { .. })
        ));
        assert!(matches!(
            resolve(&vault, "n.md", "folder.html"),
            Err(PageTemplateError::NotHtml { .. })
        ));
        assert!(matches!(
            resolve(&vault, "n.md", "  "),
            Err(PageTemplateError::Empty)
        ));
    }

    #[test]
    fn exempt_hidden_dirs_may_hold_templates() {
        let (_d, vault) = repo(&[(".scratch/n.md", "x"), (".scratch/t.html", "t")]);
        let exempt = [note(".scratch")];
        let t = resolve_page_template(&vault, &note(".scratch/n.md"), "t.html", &exempt).unwrap();
        assert_eq!(t.path.as_str(), ".scratch/t.html");
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_out_of_the_repository_is_refused() {
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("evil.html"), "x").unwrap();
        let (_d, vault) = repo(&[("n.md", "x")]);
        std::os::unix::fs::symlink(
            outside.path().join("evil.html"),
            vault.root().join("link.html"),
        )
        .unwrap();
        assert!(matches!(
            resolve(&vault, "n.md", "link.html"),
            Err(PageTemplateError::OutsideRoot { .. })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn a_visible_symlink_to_a_hidden_file_is_refused() {
        let (_d, vault) = repo(&[("n.md", "x"), (".mbr/index.html", "secret")]);
        std::os::unix::fs::symlink(
            vault.root().join(".mbr/index.html"),
            vault.root().join("t.html"),
        )
        .unwrap();
        assert!(matches!(
            resolve(&vault, "n.md", "t.html"),
            Err(PageTemplateError::Hidden { .. })
        ));
    }

    #[test]
    fn siblings_are_the_other_html_files_directly_beside_it() {
        let (_d, vault) = repo(&[
            ("inv/n.md", "x"),
            ("inv/invoice.html", "main"),
            ("inv/lines.html", "lines"),
            ("inv/base.HTML", "base"),
            ("inv/.secret.html", "no"),
            ("inv/notes.txt", "no"),
            ("inv/sub/deep.html", "no"),
        ]);
        let t = resolve(&vault, "inv/n.md", "invoice.html").unwrap();
        let names: Vec<&str> = t.siblings.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["base.HTML", "lines.html"]);
    }

    /// On a cloud vault the template is `NotDownloaded` (by availability, no
    /// read), and a not-downloaded sibling is pending: neither read nor
    /// requested.
    #[test]
    fn not_downloaded_template_and_siblings() {
        use crate::vault::{Availability, MemVault};
        let vault = MemVault::new()
            .with_file("inv/n.md", "x")
            .with_file("inv/t.html", "t")
            .with_file("inv/lines.html", "l")
            .with_file("inv/ok.html", "ok");
        vault.set_availability("inv/t.html", Availability::NotDownloaded);
        vault.set_availability("inv/lines.html", Availability::NotDownloaded);
        let err = resolve_page_template(&vault, &note("inv/n.md"), "t.html", &[]).unwrap_err();
        assert_eq!(err.not_downloaded_files(), Some(&[note("inv/t.html")][..]));
        assert!(err.to_string().contains("not downloaded"), "{err}");

        vault.set_availability("inv/t.html", Availability::Local);
        let t = resolve_page_template(&vault, &note("inv/n.md"), "t.html", &[]).unwrap();
        assert_eq!(t.siblings, [("ok.html".to_string(), "ok".to_string())]);
        assert_eq!(
            t.pending_siblings,
            [("lines.html".to_string(), note("inv/lines.html"))]
        );
        assert!(vault.download_requests().is_empty(), "core never requests");
    }

    #[test]
    fn an_oversized_template_is_refused() {
        let big = "x".repeat(usize::try_from(MAX_TEMPLATE_BYTES).unwrap() + 1);
        let (_d, vault) = repo(&[("n.md", "x"), ("t.html", &big)]);
        assert!(matches!(
            resolve(&vault, "n.md", "t.html"),
            Err(PageTemplateError::TooLarge { .. })
        ));
    }

    #[test]
    fn an_unsaved_template_takes_the_text_and_the_saved_siblings() {
        let (_d, vault) = repo(&[
            ("invoices/tpl/invoice.html", "saved"),
            ("invoices/tpl/lines.html", "lines"),
        ]);
        for given in ["invoices/tpl/invoice.html", "/invoices/tpl/invoice.html"] {
            let t = unsaved_page_template(&vault, given, "unsaved", &[]).unwrap();
            assert_eq!(t.path.as_str(), "invoices/tpl/invoice.html");
            assert_eq!(t.authored, "/invoices/tpl/invoice.html");
            assert_eq!(t.source, "unsaved");
            // The saved file is not a sibling of its own unsaved text.
            assert_eq!(
                t.siblings,
                [("lines.html".to_string(), "lines".to_string())]
            );
        }
        // A template not saved yet, even in a folder not made yet.
        let new = unsaved_page_template(&vault, "invoices/tpl/new.html", "n", &[]).unwrap();
        assert_eq!(new.path.as_str(), "invoices/tpl/new.html");
        assert_eq!(new.siblings.len(), 2);
        let nowhere = unsaved_page_template(&vault, "drafts/x.html", "n", &[]).unwrap();
        assert_eq!(nowhere.path.as_str(), "drafts/x.html");
        assert!(nowhere.siblings.is_empty());
    }

    #[test]
    fn an_unsaved_template_path_follows_the_location_rules() {
        let (_d, vault) = repo(&[("t.html", "x")]);
        let refused = |path: &str| unsaved_page_template(&vault, path, "x", &[]).unwrap_err();
        assert!(matches!(
            refused("../t.html"),
            PageTemplateError::OutsideRoot { .. }
        ));
        assert!(matches!(
            refused(".mbr/t.html"),
            PageTemplateError::Hidden { .. }
        ));
        assert!(matches!(
            refused("t.txt"),
            PageTemplateError::NotHtml { .. }
        ));
        assert!(matches!(refused(""), PageTemplateError::NotHtml { .. }));
        let big = "x".repeat(usize::try_from(MAX_TEMPLATE_BYTES).unwrap() + 1);
        assert!(matches!(
            unsaved_page_template(&vault, "t.html", &big, &[]),
            Err(PageTemplateError::TooLarge { .. })
        ));
    }

    #[test]
    fn requested_template_reads_the_frontmatter_value() {
        let mut fm = SimpleMetadata::new();
        assert!(requested_template(&fm).is_none());
        fm.insert("template".into(), "a.html".into());
        assert_eq!(requested_template(&fm).unwrap().unwrap(), "a.html");
        fm.insert("template".into(), "".into());
        assert!(matches!(
            requested_template(&fm),
            Some(Err(PageTemplateError::Empty))
        ));
        fm.insert("template".into(), serde_json::json!(5));
        assert!(matches!(
            requested_template(&fm),
            Some(Err(PageTemplateError::NotAString { .. }))
        ));
    }

    #[test]
    fn template_files_are_the_resolved_templates_of_notes() {
        let (_d, vault) = repo(&[
            ("a/n.md", "x"),
            ("a/t.html", "t"),
            ("b/m.md", "x"),
            ("plain.md", "x"),
        ]);
        let root = vault.root().to_path_buf();
        let fm = |t: &str| {
            let mut fm = SimpleMetadata::new();
            fm.insert("template".into(), t.into());
            fm
        };
        let a = fm("t.html");
        let b = fm("missing.html");
        let notes = [
            (root.join("a/n.md"), Some(&a)),
            (root.join("b/m.md"), Some(&b)),
            (root.join("plain.md"), None),
        ];
        let set = page_template_files(&vault, notes.iter().map(|(k, fm)| (Path::new(k), *fm)), &[]);
        assert_eq!(set, HashSet::from([root.join("a/t.html")]));
    }
}
