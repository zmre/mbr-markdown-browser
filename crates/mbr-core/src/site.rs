//! A library façade over a scanned repository: [`Site`].
//!
//! For applications that link mbr as a library and want the note index
//! without running a server — list notes, read one note's whole frontmatter,
//! find its backlinks, search. `Site` wraps the same [`Repo`] scan and the
//! same indexes (wikilinks, tags, relationships, backlinks) the server and
//! the static build use; nothing here is a second index.
//!
//! ```no_run
//! use std::path::Path;
//! use mbr_core::{Config, site::Site};
//!
//! # fn main() -> Result<(), mbr_core::MbrError> {
//! let site = Site::open(Path::new("/path/to/notes"), Config::default())?;
//!
//! // Every customer, then one customer's whole note and its invoices.
//! for customer in site.notes_of_type("organization") {
//!     let note = site.note(&customer.path)?;
//!     let terms = note.frontmatter.get("terms");
//!     let invoices = site.backlinks(&customer.path);
//!     println!("{} ({terms:?}): {} backlinks", customer.title, invoices.len());
//! }
//!
//! // After the application writes a note, tell the site.
//! site.refresh(&[Path::new("invoices/2026-11-17 Invoice 105.md").to_path_buf()])?;
//! # Ok(())
//! # }
//! ```
//!
//! # Contact fields are not stripped
//!
//! `site.json` leaves out a contact's `emails`, `phones`, `addresses`, … (see
//! [`crate::contact::is_public_frontmatter_key`]), which is right for a
//! published site. A caller of this API is in-process and owns the files, so
//! [`NoteData::frontmatter`], [`NoteData::contact`] and
//! [`Site::field_values`] include them. The HTTP equivalents
//! (`GET /.mbr/notes`, `GET /.mbr/fields/{key}/values`) only show them to a
//! request that passes the editing checks.
//!
//! # Threads and cost
//!
//! `Site` is `Send + Sync`; share it behind an `Arc`. [`Site::open`] scans the
//! repository (parallel, frontmatter only). The backlink index is built on the
//! first [`Site::backlinks`] call (one parse of every note), then kept up to
//! date by [`Site::refresh`]. All methods block; call them from
//! `spawn_blocking` in async code.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use serde_json::Value;

use crate::Config;
use crate::backlinks::{self, BacklinkIndexConfig};
use crate::change_event::{ChangeEventType, FileChangeEvent};
use crate::contact::Contact;
use crate::errors::{MarkdownError, MbrError, SearchError};
use crate::frontmatter_links::{
    FrontmatterLink, FrontmatterLinkForm, FrontmatterLinkSource, ResolvedLink,
};
use crate::link_index::{InboundIndex, InboundLink};
use crate::repo::{MarkdownInfo, Repo};
use crate::search::{SearchEngine, SearchQuery, SearchResponse};

/// One note in the index: where it is, what it is called, what kind it is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NoteSummary {
    /// Repository-relative path (`customers/Customer X.md`), `/`-separated on
    /// the wire.
    #[serde(serialize_with = "crate::url_path::serialize_as_url")]
    pub path: PathBuf,
    /// The note's site URL (`/customers/Customer X/`).
    pub url: String,
    /// Frontmatter `title`, else the first H1, else the file name.
    pub title: String,
    /// Frontmatter `type`, trimmed, as written (`organization`).
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub note_type: Option<String>,
    /// The `tags` field's values.
    pub tags: Vec<String>,
}

/// One note, read whole: its frontmatter exactly as written, its contact
/// card model, its frontmatter links and its body.
#[derive(Debug, Clone, Serialize)]
pub struct NoteData {
    #[serde(flatten)]
    pub summary: NoteSummary,
    /// The whole frontmatter, **lossless** ([`crate::markdown::lossless_frontmatter`]):
    /// nested maps stay maps, lists of maps stay lists, numbers stay numbers.
    /// Contact fields included. An empty object without frontmatter.
    pub frontmatter: Value,
    /// The contact model for a `type: person` / `type: organization` note.
    pub contact: Option<Contact>,
    /// Its frontmatter values that are links, resolved against the site.
    pub frontmatter_links: Vec<FrontmatterLink>,
    /// The markdown after the frontmatter block.
    pub body: String,
    /// [`crate::edit_auth::content_hash`] of the file as read: what
    /// `POST /.mbr/edit` takes as `base_hash`, to refuse a write over a
    /// version this caller never saw.
    pub content_hash: String,
}

/// A repository opened as a library. See the [module docs](self).
pub struct Site {
    repo: Arc<Repo>,
    inbound: Arc<InboundIndex>,
    backlink_config: BacklinkIndexConfig,
    /// Serializes the backlink index's first build against
    /// [`Site::refresh`], which an interleaving edit could be undone by.
    index_lock: Mutex<()>,
}

impl Site {
    /// Scans `root` (which replaces `config.root_dir`) and builds the
    /// indexes: wikilinks, tags, and relationships unless
    /// `relationship_tracking` is off. Backlinks are built on first use.
    ///
    /// `config` is the usual configuration; `Config::read` gives one with
    /// the repository's `.mbr/config.toml` and `MBR_*` applied.
    pub fn open(root: &Path, mut config: Config) -> Result<Site, MbrError> {
        config.root_dir = root.to_path_buf();
        let repo = Repo::init_from_config(&config);
        repo.scan_all()?;
        repo.scan_static_folder()?;
        if config.relationship_tracking {
            repo.build_relationship_index();
        }
        repo.build_wikilink_index();
        repo.mark_scan_complete();
        Ok(Self::from_repo(
            Arc::new(repo),
            Arc::new(InboundIndex::new()),
        ))
    }

    /// A `Site` over a repository that is already scanned and indexed, and
    /// the backlink index kept beside it — what a running server has
    /// (`mbr_server::server::Server::site`), so an application embedding the
    /// server shares its scan rather than making a second one. If `inbound`
    /// is not ready yet, it is built on the first [`Self::backlinks`].
    pub fn from_repo(repo: Arc<Repo>, inbound: Arc<InboundIndex>) -> Site {
        let backlink_config = BacklinkIndexConfig::for_repo(
            &repo,
            crate::outside_link::OutsideLinkContext::for_repo(&repo),
        );
        Site {
            repo,
            inbound,
            backlink_config,
            index_lock: Mutex::new(()),
        }
    }

    /// The scanned repository, for anything this façade does not cover.
    pub fn repo(&self) -> &Arc<Repo> {
        &self.repo
    }

    /// Applies changes to the files at `changed` (repository-relative, or
    /// absolute under the root): each is re-read if it exists and dropped if
    /// it does not, and every index follows — incrementally, file by file,
    /// as the server applies its watcher's events. Paths outside the root
    /// are ignored.
    pub fn refresh(&self, changed: &[PathBuf]) -> Result<(), MbrError> {
        let root = self.repo.vault().root().to_path_buf();
        let events: Vec<FileChangeEvent> = changed
            .iter()
            .filter_map(|path| {
                let relative = if path.is_absolute() {
                    path.strip_prefix(&root).ok()?.to_path_buf()
                } else {
                    path.clone()
                };
                let key = self.key_for(&relative)?;
                let (vault, vault_path) = self.repo.locate(&key)?;
                let entry = vault.stat(&vault_path).ok().flatten();
                // A folder is not a file change (its files are listed one by one).
                if entry.as_ref().is_some_and(|e| !e.is_file()) {
                    return None;
                }
                let exists = entry.is_some();
                let known = self.repo.markdown_files.pin().contains_key(&key)
                    || self.repo.other_files.pin().contains_key(&key);
                let event = match (exists, known) {
                    (true, true) => ChangeEventType::Modified,
                    (true, false) => ChangeEventType::Created,
                    (false, _) => ChangeEventType::Deleted,
                };
                Some(FileChangeEvent {
                    path: key.to_string_lossy().into_owned(),
                    relative_path: crate::url_path::path_to_url(&relative),
                    event,
                })
            })
            .collect();
        if events.is_empty() {
            return Ok(());
        }
        let _guard = self.lock_indexes();
        backlinks::apply_file_changes(
            &self.repo,
            &self.inbound,
            &self.backlink_config,
            &events,
            |_, _| {},
        );
        Ok(())
    }

    /// Every note, in path order.
    pub fn notes(&self) -> impl Iterator<Item = NoteSummary> + use<> {
        summaries(&self.repo).into_iter()
    }

    /// The notes whose `type` is `ty`, compared case-insensitively and
    /// trimmed, in path order.
    pub fn notes_of_type(&self, ty: &str) -> Vec<NoteSummary> {
        self.notes().filter(|note| is_of_type(note, ty)).collect()
    }

    /// The notes of type `ty` (every note for `None`), each with the
    /// frontmatter `fields` named (lossless; a dotted name reads a nested
    /// map: `billing_portal.url`). Reads the frontmatter of each listed note
    /// when `fields` is not empty. Contact fields included; see the
    /// [module docs](self).
    pub fn notes_with_fields(&self, ty: Option<&str>, fields: &[String]) -> Vec<NoteWithFields> {
        notes_with_fields(&self.repo, ty, fields)
    }

    /// One note, read whole. `rel` is repository-relative. An unknown note is
    /// [`MarkdownError::ReadFailed`] with `NotFound`.
    pub fn note(&self, rel: &Path) -> Result<NoteData, MbrError> {
        let (key, info) = self.note_info(rel)?;
        let source = crate::markdown::read_source(self.repo.vault().as_ref(), &key)?;
        let parsed = crate::markdown::parse_frontmatter_block(&source);
        let frontmatter = crate::markdown::lossless_frontmatter(&parsed.yaml);
        let contact = parsed.yaml.as_ref().and_then(Contact::from_yaml);
        Ok(NoteData {
            frontmatter_links: self.repo.frontmatter_links_of(&info),
            summary: summary(&info),
            frontmatter,
            contact,
            body: source[parsed.body_start..].to_string(),
            content_hash: crate::edit_auth::content_hash(source.as_bytes()),
        })
    }

    /// The whole frontmatter of the note `rel`, lossless, read from the
    /// first bytes of the file only ([`crate::markdown::lossless_frontmatter_from_vault`]).
    /// Cheaper than [`Self::note`] when the body is not needed.
    pub fn frontmatter(&self, rel: &Path) -> Result<Value, MbrError> {
        let (key, _) = self.note_info(rel)?;
        Ok(crate::markdown::lossless_frontmatter_from_vault(
            self.repo.vault().as_ref(),
            &key,
        )?)
    }

    /// The distinct values of the frontmatter field `key` across every note,
    /// with how many notes use each, most used first (then by value): for
    /// autocomplete. A list value counts each element. Read from the index's
    /// simplified frontmatter, so it costs no I/O: a nested field is
    /// `parent.child`, and a list of maps (a contact's `emails`) has no
    /// values here — read it with [`Self::note`].
    pub fn field_values(&self, key: &str) -> Vec<(Value, usize)> {
        field_values(&self.repo, key)
    }

    /// The notes linking to `rel` (its backlinks), frontmatter links
    /// included ([`InboundLink::origin`]), in `from` order. Builds the
    /// backlink index on first use.
    pub fn backlinks(&self, rel: &Path) -> Vec<InboundLink> {
        let Ok((_, info)) = self.note_info(rel) else {
            return Vec::new();
        };
        self.ensure_backlinks();
        self.inbound.get(&info.url_path)
    }

    /// Resolves `target` — a wikilink target (`Customer X`, `[[Customer X]]`,
    /// `People/Ann#Bio`) — as a wikilink written in the note `from` would
    /// resolve: same folder first, then by title, alias and file name.
    /// `None` when `from` is not a note or nothing answers to the name.
    pub fn resolve_wikilink(&self, from: &Path, target: &str) -> Option<ResolvedLink> {
        let (_, info) = self.note_info(from).ok()?;
        let target = target.trim();
        let target = target
            .strip_prefix("[[")
            .and_then(|t| t.strip_suffix("]]"))
            .unwrap_or(target);
        let destination = target.split('|').next().unwrap_or(target).trim();
        if destination.is_empty() {
            return None;
        }
        let source = FrontmatterLinkSource {
            key: String::new(),
            raw: format!("[[{target}]]"),
            position: None,
            form: FrontmatterLinkForm::Wikilink,
            destination: destination.to_string(),
            label: None,
        };
        self.repo
            .resolve_link_sources(&info, std::slice::from_ref(&source))
            .into_iter()
            .next()?
            .resolved
    }

    /// Searches the site exactly as `POST /.mbr/search` does (metadata,
    /// content, and other files when the query asks for them).
    pub fn search(&self, query: &SearchQuery) -> Result<SearchResponse, SearchError> {
        SearchEngine::new(Arc::clone(&self.repo)).search_with_other_paths(query)
    }

    fn lock_indexes(&self) -> std::sync::MutexGuard<'_, ()> {
        self.index_lock.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn ensure_backlinks(&self) {
        if self.inbound.is_ready() {
            return;
        }
        let _guard = self.lock_indexes();
        if !self.inbound.is_ready() {
            backlinks::populate_inbound_index(&self.repo, &self.inbound, &self.backlink_config);
        }
    }

    /// The index key of a repository-relative path, refused when it is not a
    /// valid vault path (absolute, `..` past the root).
    fn key_for(&self, rel: &Path) -> Option<PathBuf> {
        let path = crate::vault::VaultPath::from_relative_native(rel).ok()?;
        Some(self.repo.vault().key(&path))
    }

    fn note_info(&self, rel: &Path) -> Result<(PathBuf, MarkdownInfo), MbrError> {
        let not_found = || {
            MbrError::Markdown(MarkdownError::ReadFailed {
                path: rel.to_path_buf(),
                source: std::io::Error::new(std::io::ErrorKind::NotFound, "no such note"),
            })
        };
        let key = self.key_for(rel).ok_or_else(not_found)?;
        let info = self
            .repo
            .markdown_files
            .pin()
            .get(&key)
            .cloned()
            .ok_or_else(not_found)?;
        Ok((key, info))
    }
}

/// A note and some of its frontmatter fields: [`Site::notes_with_fields`],
/// the body of `GET /.mbr/notes`.
#[derive(Debug, Clone, Serialize)]
pub struct NoteWithFields {
    #[serde(flatten)]
    pub summary: NoteSummary,
    /// The requested fields the note has, lossless; absent ones are left out.
    pub fields: serde_json::Map<String, Value>,
}

/// Every note of `repo` as a [`NoteSummary`], in path order.
pub fn summaries(repo: &Repo) -> Vec<NoteSummary> {
    let mut notes: Vec<NoteSummary> = repo
        .markdown_files
        .pin()
        .iter()
        .map(|(_, info)| summary(info))
        .collect();
    notes.sort_by(|a, b| a.path.cmp(&b.path));
    notes
}

/// Whether `note`'s `type` is `ty`: trimmed, case-insensitive.
pub fn is_of_type(note: &NoteSummary, ty: &str) -> bool {
    note.note_type
        .as_deref()
        .is_some_and(|t| t.to_lowercase() == ty.trim().to_lowercase())
}

/// See [`Site::notes_with_fields`].
pub fn notes_with_fields(repo: &Repo, ty: Option<&str>, fields: &[String]) -> Vec<NoteWithFields> {
    summaries(repo)
        .into_iter()
        .filter(|note| ty.is_none_or(|ty| is_of_type(note, ty)))
        .map(|summary| {
            let frontmatter = if fields.is_empty() {
                Value::Null
            } else {
                crate::vault::VaultPath::from_relative_native(&summary.path)
                    .ok()
                    .map(|path| repo.vault().key(&path))
                    .and_then(|key| {
                        crate::markdown::lossless_frontmatter_from_vault(
                            repo.vault().as_ref(),
                            &key,
                        )
                        .ok()
                    })
                    .unwrap_or(Value::Null)
            };
            let fields = fields
                .iter()
                .filter_map(|field| Some((field.clone(), field_of(&frontmatter, field)?.clone())))
                .collect();
            NoteWithFields { summary, fields }
        })
        .collect()
}

/// `field` of a lossless frontmatter object: the key as written, else a
/// dotted path into nested maps (`billing_portal.url`).
fn field_of<'v>(frontmatter: &'v Value, field: &str) -> Option<&'v Value> {
    frontmatter.get(field).or_else(|| {
        field
            .split('.')
            .try_fold(frontmatter, |value, part| value.get(part))
    })
}

/// See [`Site::field_values`].
pub fn field_values(repo: &Repo, key: &str) -> Vec<(Value, usize)> {
    let mut counts: BTreeMap<String, (Value, usize)> = BTreeMap::new();
    let mut count = |value: &Value| {
        if matches!(value, Value::Null) || value.as_str().is_some_and(|s| s.trim().is_empty()) {
            return;
        }
        let entry = counts
            .entry(value.to_string())
            .or_insert_with(|| (value.clone(), 0));
        entry.1 += 1;
    };
    for (_, info) in repo.markdown_files.pin().iter() {
        match info.frontmatter.as_ref().and_then(|fm| fm.get(key)) {
            Some(Value::Array(items)) => items.iter().for_each(&mut count),
            Some(value) => count(value),
            None => {}
        }
    }
    let mut values: Vec<(Value, usize)> = counts.into_values().collect();
    values.sort_by(|(a, an), (b, bn)| bn.cmp(an).then_with(|| value_text(a).cmp(&value_text(b))));
    values
}

/// [`NoteSummary`] of an indexed note.
pub fn summary(info: &MarkdownInfo) -> NoteSummary {
    let frontmatter = info.frontmatter.as_ref();
    let text = |key: &str| {
        frontmatter
            .and_then(|fm| fm.get(key))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    NoteSummary {
        path: info.raw_path.clone(),
        url: info.url_path.clone(),
        title: text("title").unwrap_or_else(|| {
            info.raw_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_string()
        }),
        note_type: text("type"),
        tags: frontmatter
            .and_then(|fm| fm.get("tags"))
            .map(crate::repo::extract_tag_values)
            .unwrap_or_default(),
    }
}

/// How a value sorts among equally common ones: its text, not its JSON.
fn value_text(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write(root: &Path, rel: &str, text: &str) {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    /// A repository with two customers, invoices linking them, and a PDF.
    fn fixture() -> (tempfile::TempDir, Site) {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().canonicalize().unwrap();
        write(
            &root,
            "customers/Customer X.md",
            concat!(
                "---\n",
                "title: Customer X\n",
                "type: organization\n",
                "tags: [customer]\n",
                "terms: 30\n",
                "rate: 1.5\n",
                "emails:\n  - billing: ar@x.example\n",
                "open_pos:\n  - reference: PO 1\n    amount: 100\n",
                "---\n",
                "Fred is the champion.\n"
            ),
        );
        write(
            &root,
            "customers/Customer Y.md",
            "---\ntitle: Customer Y\ntype: organization\ntags: [customer, prospect]\nterms: 45\n---\n",
        );
        write(
            &root,
            "people/Ann.md",
            "---\ntype: person\ncompany: \"[[Customer X]]\"\n---\n",
        );
        write(
            &root,
            "invoices/inv-104.md",
            "---\ntype: invoice\nstatus: sent\ncustomer: \"[[Customer X]]\"\n---\nBody.\n",
        );
        write(
            &root,
            "invoices/inv-105.md",
            "---\ntype: invoice\nstatus: draft\ncustomer: \"[[Customer X]]\"\n---\nSee [[Customer Y]].\n",
        );
        write(&root, "customers/contracts/msa.pdf", "%PDF-1.4\n");
        let site = Site::open(&root, Config::default()).unwrap();
        (dir, site)
    }

    #[test]
    fn notes_lists_every_note_in_path_order() {
        let (_dir, site) = fixture();
        let paths: Vec<String> = site
            .notes()
            .map(|n| crate::url_path::path_to_url(&n.path))
            .collect();
        assert_eq!(
            paths,
            vec![
                "customers/Customer X.md",
                "customers/Customer Y.md",
                "invoices/inv-104.md",
                "invoices/inv-105.md",
                "people/Ann.md",
            ]
        );
        let x = site.notes().next().unwrap();
        assert_eq!(x.url, "/customers/Customer X/");
        assert_eq!(x.title, "Customer X");
        assert_eq!(x.note_type.as_deref(), Some("organization"));
        assert_eq!(x.tags, vec!["customer"]);
    }

    /// Frontmatter longer than the 8 KB the scan reads first (an invoice with
    /// many lines, a customer with many contacts) is read whole: the keys
    /// after the cut are in the index, not silently dropped.
    #[test]
    fn frontmatter_longer_than_the_scan_cap_is_indexed_whole() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let lines: String = (0..300)
            .map(|i| format!("line_{i:03}: \"Widget {i}, 2 hours at 150.00\"\n"))
            .collect();
        let text =
            format!("---\ntitle: Big invoice\n{lines}status: sent\ntype: invoice\n---\nBody.\n");
        assert!(text.find("status: sent").unwrap() > 8 * 1024);
        write(&root, "invoices/big.md", &text);
        let site = Site::open(&root, Config::default()).unwrap();

        let invoices = site.notes_of_type("invoice");
        assert_eq!(invoices.len(), 1);
        assert_eq!(invoices[0].title, "Big invoice");
        let with_fields = site.notes_with_fields(Some("invoice"), &["status".to_string()]);
        assert_eq!(with_fields[0].fields["status"], "sent");
        let frontmatter = site.frontmatter(Path::new("invoices/big.md")).unwrap();
        assert_eq!(frontmatter["type"], "invoice");
        assert_eq!(frontmatter["line_299"], "Widget 299, 2 hours at 150.00");
    }

    #[test]
    fn notes_of_type_is_case_insensitive() {
        let (_dir, site) = fixture();
        let titles: Vec<String> = site
            .notes_of_type(" ORGANIZATION ")
            .into_iter()
            .map(|n| n.title)
            .collect();
        assert_eq!(titles, vec!["Customer X", "Customer Y"]);
        assert_eq!(site.notes_of_type("invoice").len(), 2);
        assert!(site.notes_of_type("nothing").is_empty());
    }

    #[test]
    fn note_is_lossless_keeps_contact_fields_and_has_the_body() {
        let (_dir, site) = fixture();
        let note = site.note(Path::new("customers/Customer X.md")).unwrap();
        assert_eq!(note.frontmatter["terms"], 30);
        assert_eq!(note.frontmatter["rate"], 1.5);
        assert_eq!(note.frontmatter["open_pos"][0]["amount"], 100);
        assert_eq!(note.frontmatter["emails"][0]["billing"], "ar@x.example");
        let contact = note.contact.expect("an organization has a contact model");
        assert_eq!(
            serde_json::to_value(&contact).unwrap()["emails"][0]["value"],
            "ar@x.example"
        );
        assert_eq!(note.body, "Fred is the champion.\n");
        assert_eq!(note.content_hash.len(), 64);

        let invoice = site.note(Path::new("invoices/inv-104.md")).unwrap();
        assert_eq!(invoice.frontmatter_links.len(), 1);
        assert_eq!(
            invoice.frontmatter_links[0].resolved.as_ref().unwrap().url,
            "/customers/Customer X/"
        );
        assert!(site.note(Path::new("nope.md")).is_err());
        assert!(site.note(Path::new("../outside.md")).is_err());
        assert_eq!(
            site.frontmatter(Path::new("customers/Customer X.md"))
                .unwrap(),
            note.frontmatter
        );
    }

    #[test]
    fn field_values_counts_distinct_values_most_used_first() {
        let (_dir, site) = fixture();
        assert_eq!(
            site.field_values("tags"),
            vec![(Value::from("customer"), 2), (Value::from("prospect"), 1)]
        );
        assert_eq!(
            site.field_values("status"),
            vec![(Value::from("draft"), 1), (Value::from("sent"), 1)]
        );
        assert_eq!(
            site.field_values("customer"),
            vec![(Value::from("[[Customer X]]"), 2)]
        );
        assert!(site.field_values("missing").is_empty());
    }

    #[test]
    fn backlinks_include_frontmatter_links_with_their_key() {
        let (_dir, site) = fixture();
        let links = site.backlinks(Path::new("customers/Customer X.md"));
        let from: Vec<(&str, Option<&str>)> = links
            .iter()
            .map(|l| {
                (
                    l.from.as_str(),
                    l.origin.as_ref().map(|o| match o {
                        crate::link_index::LinkOrigin::Frontmatter { key } => key.as_str(),
                    }),
                )
            })
            .collect();
        assert_eq!(
            from,
            vec![
                ("/invoices/inv-104/", Some("customer")),
                ("/invoices/inv-105/", Some("customer")),
                ("/people/Ann/", Some("company")),
            ]
        );
        // A body link is a backlink too, without an origin.
        let y = site.backlinks(Path::new("customers/Customer Y.md"));
        assert_eq!(y.len(), 1);
        assert_eq!(y[0].origin, None);
        assert!(site.backlinks(Path::new("nope.md")).is_empty());
    }

    #[test]
    fn refresh_applies_creates_edits_and_deletes() {
        let (dir, site) = fixture();
        let root = dir.path().canonicalize().unwrap();
        assert_eq!(
            site.backlinks(Path::new("customers/Customer X.md")).len(),
            3
        );

        write(
            &root,
            "invoices/inv-106.md",
            "---\ntype: invoice\ncustomer: \"[[Customer X]]\"\n---\n",
        );
        write(
            &root,
            "invoices/inv-104.md",
            "---\ntype: invoice\ncustomer: \"[[Customer Y]]\"\n---\n",
        );
        fs::remove_file(root.join("invoices/inv-105.md")).unwrap();
        site.refresh(&[
            PathBuf::from("invoices/inv-106.md"),
            root.join("invoices/inv-104.md"),
            PathBuf::from("invoices/inv-105.md"),
            PathBuf::from("/elsewhere/ignored.md"),
        ])
        .unwrap();

        let from: Vec<String> = site
            .backlinks(Path::new("customers/Customer X.md"))
            .into_iter()
            .map(|l| l.from)
            .collect();
        assert_eq!(from, vec!["/invoices/inv-106/", "/people/Ann/"]);
        let invoices: Vec<String> = site
            .notes_of_type("invoice")
            .into_iter()
            .map(|n| n.url)
            .collect();
        assert_eq!(invoices, vec!["/invoices/inv-104/", "/invoices/inv-106/"]);
        assert_eq!(
            site.field_values("customer"),
            vec![
                (Value::from("[[Customer X]]"), 1),
                (Value::from("[[Customer Y]]"), 1)
            ]
        );
        assert!(site.refresh(&[]).is_ok());
    }

    #[test]
    fn resolve_wikilink_resolves_as_a_note_would() {
        let (_dir, site) = fixture();
        let from = Path::new("invoices/inv-104.md");
        let found = site.resolve_wikilink(from, "customer x").unwrap();
        assert_eq!(found.url, "/customers/Customer X/");
        assert_eq!(found.title, "Customer X");
        assert_eq!(
            site.resolve_wikilink(from, "[[Customer Y|Y]]").unwrap().url,
            "/customers/Customer Y/"
        );
        assert!(site.resolve_wikilink(from, "Nobody").is_none());
        assert!(
            site.resolve_wikilink(Path::new("nope.md"), "Customer X")
                .is_none()
        );
        assert!(site.resolve_wikilink(from, "[[]]").is_none());
    }

    #[test]
    fn search_finds_notes() {
        let (_dir, site) = fixture();
        let query: SearchQuery =
            serde_json::from_value(serde_json::json!({"q": "champion", "scope": "all"})).unwrap();
        let results = site.search(&query).unwrap();
        assert!(
            results
                .results
                .iter()
                .any(|r| r.url_path == "/customers/Customer X/"),
            "{:?}",
            results.results
        );
    }

    #[test]
    fn notes_with_fields_reads_lossless_fields_including_nested_ones() {
        let (_dir, site) = fixture();
        let fields = vec![
            "terms".to_string(),
            "emails".to_string(),
            "open_pos".to_string(),
            "missing".to_string(),
        ];
        let notes = site.notes_with_fields(Some("organization"), &fields);
        assert_eq!(notes.len(), 2);
        let x = &notes[0].fields;
        assert_eq!(x["terms"], 30);
        assert_eq!(x["emails"][0]["billing"], "ar@x.example");
        assert_eq!(x["open_pos"][0]["amount"], 100);
        assert!(!x.contains_key("missing"));
        assert_eq!(notes[1].fields["terms"], 45);
        // No fields asked: no reads, no fields; no type: every note.
        let all = site.notes_with_fields(None, &[]);
        assert_eq!(all.len(), 5);
        assert!(all.iter().all(|n| n.fields.is_empty()));
        assert_eq!(
            field_of(&serde_json::json!({"a": {"b": 1}}), "a.b"),
            Some(&Value::from(1))
        );
    }

    #[test]
    fn a_site_over_an_existing_repo_shares_it() {
        let (_dir, site) = fixture();
        let shared = Site::from_repo(Arc::clone(site.repo()), Arc::new(InboundIndex::new()));
        assert_eq!(shared.notes().count(), 5);
        assert_eq!(
            shared.backlinks(Path::new("customers/Customer X.md")).len(),
            3
        );
    }
}
