use std::{
    ops::Deref,
    path::{Component, Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Instant, UNIX_EPOCH},
};

use papaya::{HashMap, HashSet};
use rayon::prelude::*;
use serde::{Serialize, Serializer, ser::SerializeSeq};
use walkdir::WalkDir;

use crate::Config;
use crate::config::{RelationType, TagSource};
use crate::errors::RepoError;
use crate::relationships::{NoteRelInput, RawRelationship, RelationshipIndex};
use crate::tag_index::{TagIndex, TaggedPage};
use crate::wikilink_index::WikilinkIndex;

#[derive(Clone, Serialize)]
pub struct Repo {
    /// Repository root, **always canonicalized** (see [`canonicalize_root`]).
    ///
    /// This must stay canonical. `scan_folder` walks from
    /// `canonical_root.join(rel).canonicalize()`, and every path that WalkDir
    /// yields is relativized back against this same value. If the two were
    /// allowed to differ — a raw root here and a canonicalized one there —
    /// `pathdiff::diff_paths` would find no common prefix and hand back an
    /// effectively absolute path, so every `url_path` would embed the whole
    /// filesystem path.
    ///
    /// That is not hypothetical: on Windows `canonicalize` returns a verbatim
    /// prefix (`\\?\D:\x`), and `Prefix::VerbatimDisk != Prefix::Disk`, so a
    /// raw `D:\x` base silently failed to match. On Unix the same thing happens
    /// through a symlinked temp dir (`/tmp` vs `/private/tmp`).
    ///
    /// Note this is deliberately *not* the same value as `Config::root_dir`,
    /// which stays as the user supplied it (it feeds template globs and
    /// user-facing output, where a verbatim prefix would cause its own
    /// problems).
    #[serde(skip)]
    canonical_root: PathBuf,
    #[serde(skip)]
    static_folder: String,
    /// The static overlay's own directory, when it legitimately resolves
    /// *outside* `canonical_root` — `static_folder = "../static"` for the
    /// `repo/content` + `repo/static` layout, or `"../../static"` for a
    /// framework layout whose markdown root is pinned two levels down
    /// (SvelteKit's `<project>/src/routes` alongside `<project>/static`).
    ///
    /// `None` when the overlay is disabled, resolves inside the root, or was
    /// refused by the policy — so this is the only directory besides
    /// `canonical_root` that the scanner may descend into. Computed once, from
    /// [`crate::config::resolve_static_overlay`], which is the same call
    /// `Config::validate` makes: the scanner must never accept a root the
    /// validator would have refused.
    #[serde(skip)]
    canonical_static_root: Option<PathBuf>,
    #[serde(skip)]
    markdown_extensions: Vec<String>,
    /// The configured index file name (e.g., "index.md" or "_index.md").
    /// Exposed in site.json for frontend use.
    pub index_file: String,
    #[serde(skip)]
    ignore_dirs: Vec<String>,
    #[serde(skip)]
    #[allow(dead_code)] // Kept for debugging/logging; compiled version used for matching
    ignore_globs: Vec<String>,
    #[serde(skip)]
    compiled_ignore_globs: Vec<glob::Pattern>,
    #[serde(skip)]
    pub scanned_folders: HashSet<PathBuf>,
    #[serde(skip)]
    pub queued_folders: HashMap<PathBuf, PathBuf>,
    pub markdown_files: MarkdownFiles,
    pub other_files: OtherFiles,
    /// Thread-safe index of tagged pages.
    #[serde(skip)]
    pub tag_index: Arc<TagIndex>,
    /// Thread-safe index of typed relationships between notes.
    #[serde(skip)]
    pub relationship_index: Arc<RelationshipIndex>,
    /// Thread-safe global name index for body-wikilink (`[[Name]]`) resolution.
    /// Always built (independent of relationship tracking).
    #[serde(skip)]
    pub wikilink_index: Arc<WikilinkIndex>,
    /// Configured tag sources for frontmatter extraction.
    #[serde(skip)]
    tag_sources: Vec<TagSource>,
    /// Whether text extraction has been performed for searchable files.
    #[serde(skip)]
    text_extracted: Arc<AtomicBool>,
    /// Whether media metadata population has *finished* (phase 2 of scan).
    ///
    /// Readers (`is_media_populated`, `wait_for_media`, the `media.json`
    /// handler) treat this as "the metadata is there", so it is published only
    /// once every entry has been probed.
    #[serde(skip)]
    media_populated: Arc<AtomicBool>,
    /// Whether media metadata population has *started* (run-once guard).
    ///
    /// Deliberately separate from `media_populated`: using one flag for both
    /// meant the very first instruction of `populate_media_metadata` announced
    /// completion, so a `media.json` request arriving during the probing window
    /// skipped the wait and returned entries with no duration or dimensions.
    #[serde(skip)]
    media_population_started: Arc<AtomicBool>,
    /// Whether initial scan_all has completed.
    #[serde(skip)]
    scan_complete: Arc<AtomicBool>,
    /// Notifier for scan completion (signals waiters when scan_all finishes).
    #[serde(skip)]
    scan_notify: Arc<tokio::sync::Notify>,
    /// Notifier for media metadata population (signals waiters when populate_media_metadata finishes).
    #[serde(skip)]
    media_notify: Arc<tokio::sync::Notify>,
}

#[derive(Clone)]
pub struct MarkdownFiles(HashMap<PathBuf, MarkdownInfo>);
impl Deref for MarkdownFiles {
    type Target = HashMap<PathBuf, MarkdownInfo>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// Serializes in `url_path` order rather than map order.
///
/// `papaya::HashMap` uses a randomly seeded `RandomState`, so iteration order
/// differs between processes: two builds of the same repository produced
/// byte-different `site.json`/`media.json`, which churns committed `build/`
/// directories and invalidates content-hash/ETag caches on a no-op rebuild.
/// `raw_path` breaks ties, because distinct files can share a `url_path`
/// (`docs/index.md` and `docs.md` both render at `/docs/`).
impl Serialize for MarkdownFiles {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let pin = self.pin();
        let mut entries: Vec<&MarkdownInfo> = pin.iter().map(|(_, v)| v).collect();
        entries.sort_unstable_by(|a, b| {
            a.url_path
                .cmp(&b.url_path)
                .then_with(|| a.raw_path.cmp(&b.raw_path))
        });

        let mut s = serializer.serialize_seq(Some(entries.len()))?;
        for v in entries {
            s.serialize_element(v)?;
        }
        s.end()
    }
}

#[derive(Clone)]
pub struct OtherFiles(HashMap<PathBuf, OtherFileInfo>);
impl Deref for OtherFiles {
    type Target = HashMap<PathBuf, OtherFileInfo>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
/// Serializes in `url_path` order — see [`MarkdownFiles`]'s impl for why.
impl Serialize for OtherFiles {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let pin = self.pin();
        let mut entries: Vec<&OtherFileInfo> = pin.iter().map(|(_, v)| v).collect();
        entries.sort_unstable_by(|a, b| {
            a.url_path
                .cmp(&b.url_path)
                .then_with(|| a.raw_path.cmp(&b.raw_path))
        });

        let mut s = serializer.serialize_seq(Some(entries.len()))?;
        for v in entries {
            s.serialize_element(v)?;
        }
        s.end()
    }
}

#[derive(Clone, Serialize)]
pub struct MarkdownInfo {
    /// Path to the source file, **relative to the repo root**.
    ///
    /// Relative rather than absolute for two reasons: it is serialized into
    /// `site.json`, where an absolute path would leak the build machine's
    /// directory layout into every published static site; and consumers
    /// (including `.mbr/` customizations) split it on `/` to recover the file
    /// name. It is serialized through [`crate::url_path::serialize_as_url`] so
    /// the JSON stays `/`-separated even on Windows.
    ///
    /// Join it with the repo root before doing any file I/O — see
    /// `SearchEngine::search_file_content`. The `markdown_files` map is still
    /// keyed by absolute path.
    #[serde(serialize_with = "crate::url_path::serialize_as_url")]
    pub raw_path: PathBuf,
    pub url_path: String,
    pub created: u64,
    pub modified: u64,
    pub frontmatter: Option<crate::markdown::SimpleMetadata>,
    /// Typed relationships declared in frontmatter (unresolved endpoints).
    /// Skipped in serialization — resolved relationships are exposed via the
    /// relationship index in site.json/links.json instead.
    #[serde(skip)]
    pub relationships: Vec<RawRelationship>,
}

#[derive(Clone, Serialize)]
pub struct OtherFileInfo {
    #[serde(skip)]
    pub raw_path: PathBuf,
    pub url_path: String,
    metadata: StaticFileMetadata,
    /// Extracted text content for searchable files (PDFs, text files).
    /// Only populated for files under the size limit.
    #[serde(skip)]
    pub extracted_text: Option<String>,
}

/// Maximum file size (in bytes) for text extraction (10 MB).
const MAX_TEXT_EXTRACTION_SIZE: u64 = 10 * 1024 * 1024;

impl OtherFileInfo {
    /// Returns the file type as a string for search results.
    pub fn filetype(&self) -> &'static str {
        match &self.metadata.kind {
            StaticFileKind::Pdf { .. } => "pdf",
            StaticFileKind::Image { .. } => "image",
            StaticFileKind::Video { .. } => "video",
            StaticFileKind::Audio { .. } => "audio",
            StaticFileKind::Text => "text",
            StaticFileKind::Other => "other",
        }
    }

    /// Returns true if this file type is searchable (has extractable text).
    pub fn is_searchable(&self) -> bool {
        matches!(
            &self.metadata.kind,
            StaticFileKind::Pdf { .. } | StaticFileKind::Text
        )
    }

    /// Extract text content from the file if it's a searchable type.
    /// Respects file size limit for performance.
    fn extract_text(&self) -> Option<String> {
        // Check file size first
        if let Some(size) = self.metadata.file_size_bytes
            && size > MAX_TEXT_EXTRACTION_SIZE
        {
            tracing::debug!(
                "Skipping text extraction for {:?}: file too large ({} bytes)",
                self.raw_path,
                size
            );
            return None;
        }

        match &self.metadata.kind {
            StaticFileKind::Pdf { .. } => self.extract_pdf_text(),
            StaticFileKind::Text => self.extract_plain_text(),
            _ => None,
        }
    }

    /// Extract text from a PDF file using lopdf.
    fn extract_pdf_text(&self) -> Option<String> {
        let doc = match lopdf::Document::load(&self.raw_path) {
            Ok(doc) => doc,
            Err(e) => {
                tracing::debug!("Failed to load PDF {:?}: {}", self.raw_path, e);
                return None;
            }
        };

        let page_numbers: Vec<u32> = doc.get_pages().keys().copied().collect();
        if page_numbers.is_empty() {
            return None;
        }

        match doc.extract_text(&page_numbers) {
            Ok(text) => {
                let text = text.trim().to_string();
                if text.is_empty() { None } else { Some(text) }
            }
            Err(e) => {
                tracing::debug!("Failed to extract PDF text from {:?}: {}", self.raw_path, e);
                None
            }
        }
    }

    /// Extract text from a plain text file.
    fn extract_plain_text(&self) -> Option<String> {
        match std::fs::read_to_string(&self.raw_path) {
            Ok(text) => {
                let text = text.trim().to_string();
                if text.is_empty() { None } else { Some(text) }
            }
            Err(e) => {
                tracing::debug!("Failed to read text file {:?}: {}", self.raw_path, e);
                None
            }
        }
    }
}

#[derive(Clone, Default, Serialize)]
pub struct StaticFileMetadata {
    /// Absolute path to the source file.
    ///
    /// Used internally to probe the file for kind-specific metadata (see
    /// `populate_basic` / `populate_full`), so it must stay absolute.
    ///
    /// **Not serialized.** It reaches both `site.json` (static builds) and
    /// `/.mbr/media.json` (server mode), where an absolute path would publish
    /// the build machine's directory layout to every visitor. Nothing consumes
    /// it on the wire — the frontend reads only `kind`, `created`, `modified`
    /// and `file_size_bytes`.
    #[serde(skip)]
    path: PathBuf,
    created: Option<u64>,
    modified: Option<u64>,
    file_size_bytes: Option<u64>,
    kind: StaticFileKind,
}

#[derive(Clone, Default, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum StaticFileKind {
    Pdf {
        description: Option<String>,
        title: Option<String>,
        author: Option<String>,
        subject: Option<String>,
        num_pages: Option<usize>,
    },
    Image {
        width: Option<u32>,
        height: Option<u32>,
    },
    Video {
        width: Option<u32>,
        height: Option<u32>,
        duration: Option<String>,
        title: Option<String>,
        genre: Option<String>,
        album: Option<String>,
    },
    Audio {
        duration: Option<String>,
        title: Option<String>,
    },
    Text,
    #[default]
    Other,
}

/* impl Serialize for Repo {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut s = serializer.serialize_struct("Site", 2)?;
        s.serialize_field("markdown", &self.markdown_files)?;
        s.serialize_field("other", &self.other_files)?;
        s.end()
    }
}

impl Serialize for papaya::HashMap<PathBuf, MarkdownInfo> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(self.x.len()))?;
        for (k, v) in &self.x {
            map.serialize_entry(&k.to_string(), &v)?;
        }
        map.end()
    }
} */

impl StaticFileMetadata {
    pub fn empty<P: Into<std::path::PathBuf>>(file: P) -> Self {
        let file: PathBuf = file.into();
        // We'll silently ignore errors and always return something
        match file
            .extension()
            .map(|x| x.to_ascii_lowercase().to_string_lossy().to_string())
            .as_deref()
        {
            Some("pdf") => Self {
                path: file,
                kind: StaticFileKind::Pdf {
                    description: None,
                    title: None,
                    author: None,
                    subject: None,
                    num_pages: None,
                },
                ..Default::default()
            },
            Some("jpg") | Some("jpeg") | Some("png") | Some("webp") | Some("gif") | Some("bmp")
            | Some("tif") | Some("tiff") => Self {
                path: file,
                kind: StaticFileKind::Image {
                    width: None,
                    height: None,
                },
                ..Default::default()
            },
            Some("aiff") | Some("mp3") | Some("aac") | Some("m4a") | Some("ogg") | Some("oga")
            | Some("opus") | Some("wma") | Some("flac") | Some("wav") | Some("aif") | Some("") => {
                Self {
                    path: file,
                    kind: StaticFileKind::Audio {
                        duration: None,
                        title: None,
                    },
                    ..Default::default()
                }
            }
            Some("mp4") | Some("m4v") | Some("mov") | Some("webm") | Some("flv") | Some("mpg")
            | Some("mpeg") | Some("avi") | Some("3gp") | Some("wmv") => Self {
                path: file,
                kind: StaticFileKind::Video {
                    width: None,
                    height: None,
                    duration: None,
                    title: None,
                    genre: None,
                    album: None,
                },
                ..Default::default()
            },
            Some("txt") | Some("css") | Some("vtt") | Some("srt") | Some("toml") | Some("json")
            | Some("js") | Some("ts") => Self {
                path: file,
                kind: StaticFileKind::Text,
                ..Default::default()
            },
            _ => Self {
                path: file,
                kind: StaticFileKind::Other,
                ..Default::default()
            },
        }
    }

    /// Populate basic file metadata (size, timestamps) without expensive media extraction.
    pub fn populate_basic(self) -> Self {
        let mut me = self;
        let file_details_start = Instant::now();
        let (filesize, created, modified) = match file_details_from_path(&me.path).ok() {
            Some((fs, c, m)) => (Some(fs), Some(c), Some(m)),
            _ => (None, None, None),
        };
        tracing::debug!(
            "populate file_details for {:?}: {:?}",
            me.path,
            file_details_start.elapsed()
        );
        me.file_size_bytes = filesize;
        me.created = created;
        me.modified = modified;
        me
    }

    /// Populate media-specific metadata (ffmpeg, lopdf) - expensive operation.
    #[cfg(feature = "media-metadata")]
    pub fn populate_media(self) -> Self {
        let mut me = self;
        let media_start = Instant::now();
        let kind_name = match &me.kind {
            StaticFileKind::Pdf { .. } => "pdf",
            StaticFileKind::Image { .. } => "image",
            StaticFileKind::Video { .. } => "video",
            StaticFileKind::Audio { .. } => "audio",
            StaticFileKind::Text => "text",
            StaticFileKind::Other => "other",
        };
        me.kind = match me.kind {
            StaticFileKind::Pdf { .. } => match crate::pdf_metadata::probe_pdf(&me.path) {
                Ok(meta) => StaticFileKind::Pdf {
                    title: meta.title,
                    author: meta.author,
                    subject: meta.subject,
                    description: None,
                    num_pages: Some(meta.num_pages as usize),
                },
                Err(e) => {
                    tracing::debug!("Failed to extract PDF metadata from {:?}: {}", me.path, e);
                    me.kind
                }
            },
            StaticFileKind::Image { .. } => {
                let metadata = metadata::media_file::MediaFileMetadata::new(&me.path).ok();

                StaticFileKind::Image {
                    width: metadata.as_ref().and_then(|m| m.width),
                    height: metadata.as_ref().and_then(|m| m.height),
                }
            }
            StaticFileKind::Audio { .. } => {
                let metadata = metadata::media_file::MediaFileMetadata::new(&me.path).ok();
                StaticFileKind::Audio {
                    duration: metadata.as_ref().and_then(|m| m.duration.clone()),
                    title: metadata.as_ref().and_then(|m| m.title.clone()),
                }
            }
            StaticFileKind::Video { .. } => {
                let metadata = metadata::media_file::MediaFileMetadata::new(&me.path).ok();

                // Extract genre from tags (case-insensitive search)
                let genre = metadata.as_ref().and_then(|m| {
                    m.tags
                        .iter()
                        .find(|(k, _)| k.eq_ignore_ascii_case("genre"))
                        .map(|(_, v)| v.clone())
                });

                // Extract album from tags (case-insensitive search)
                let album = metadata.as_ref().and_then(|m| {
                    m.tags
                        .iter()
                        .find(|(k, _)| k.eq_ignore_ascii_case("album"))
                        .map(|(_, v)| v.clone())
                });

                StaticFileKind::Video {
                    width: metadata.as_ref().and_then(|m| m.width),
                    height: metadata.as_ref().and_then(|m| m.height),
                    duration: metadata.as_ref().and_then(|m| m.duration.clone()),
                    title: metadata.as_ref().and_then(|m| m.title.clone()),
                    genre,
                    album,
                }
            }
            _ => me.kind,
        };
        tracing::debug!(
            "populate media metadata ({}) for {:?}: {:?}",
            kind_name,
            me.path,
            media_start.elapsed()
        );
        me
    }

    /// Full populate: basic + media metadata (used by build mode).
    pub fn populate(self) -> Self {
        let me = self.populate_basic();
        #[cfg(feature = "media-metadata")]
        let me = me.populate_media();
        me
    }

    pub fn from<P: Into<std::path::PathBuf>>(file: P) -> Self {
        let empty = Self::empty(file);
        empty.populate()
    }
}

/// Canonicalizes a repository root for use as [`Repo::canonical_root`].
///
/// Falls back to the path as given when canonicalization fails (a root that
/// does not exist yet). That is safe: `scan_folder` canonicalizes the same path
/// and surfaces the real error, and with both sides raw the relativization
/// invariant still holds.
fn canonicalize_root(root_dir: PathBuf) -> PathBuf {
    root_dir.canonicalize().unwrap_or(root_dir)
}

impl Repo {
    pub fn init_from_config(c: &Config) -> Self {
        Self::init(
            c.root_dir.clone(),
            c.static_folder.clone(),
            &c.markdown_extensions[..],
            &c.ignore_dirs[..],
            &c.ignore_globs[..],
            c.index_file.clone(),
            &c.tag_sources[..],
            &c.relationship_types[..],
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn init<S: Into<String>, P: Into<std::path::PathBuf>>(
        root_dir: P,
        static_folder: S,
        markdown_extensions: &[String],
        ignore_dirs: &[String],
        ignore_globs: &[String],
        index_file: S,
        tag_sources: &[TagSource],
        relationship_types: &[RelationType],
    ) -> Self {
        // Pre-compile glob patterns for efficient matching during scans
        let compiled_ignore_globs: Vec<glob::Pattern> = ignore_globs
            .iter()
            .filter_map(|pat| {
                glob::Pattern::new(pat)
                    .map_err(|e| tracing::warn!("Invalid ignore glob pattern '{}': {}", pat, e))
                    .ok()
            })
            .collect();

        let canonical_root = canonicalize_root(root_dir.into());
        let static_folder = static_folder.into();
        // Ask the config policy, rather than re-deriving containment here, so a
        // refused overlay is never scannable. A policy error is not re-reported:
        // `Config::validate` already aborted startup on it, and the only callers
        // that reach here with a bad value are tests constructing a `Repo`
        // directly.
        let canonical_static_root =
            match crate::config::resolve_static_overlay(&canonical_root, &static_folder) {
                Ok(crate::config::StaticOverlay::External(dir)) => Some(dir),
                Ok(crate::config::StaticOverlay::WithinRoot) => None,
                Err(e) => {
                    tracing::warn!("Not indexing static_folder {static_folder:?}: {e}");
                    None
                }
            };

        Self {
            canonical_root,
            static_folder,
            canonical_static_root,
            markdown_extensions: markdown_extensions.to_vec(),
            ignore_dirs: ignore_dirs.to_vec(),
            ignore_globs: ignore_globs.to_vec(),
            compiled_ignore_globs,
            index_file: index_file.into(),
            scanned_folders: HashSet::new(),
            queued_folders: HashMap::new(),
            markdown_files: MarkdownFiles(HashMap::new()),
            other_files: OtherFiles(HashMap::new()),
            tag_index: Arc::new(TagIndex::new()),
            relationship_index: Arc::new(RelationshipIndex::from_relation_types(
                relationship_types,
            )),
            wikilink_index: Arc::new(WikilinkIndex::new()),
            tag_sources: tag_sources.to_vec(),
            text_extracted: Arc::new(AtomicBool::new(false)),
            media_populated: Arc::new(AtomicBool::new(false)),
            media_population_started: Arc::new(AtomicBool::new(false)),
            scan_complete: Arc::new(AtomicBool::new(false)),
            scan_notify: Arc::new(tokio::sync::Notify::new()),
            media_notify: Arc::new(tokio::sync::Notify::new()),
        }
    }

    /// Converts an absolute file path into the repo-relative form stored in
    /// [`MarkdownInfo::raw_path`].
    ///
    /// Uses `diff_paths` rather than `strip_prefix` so a file that somehow sits
    /// outside the root still yields a *relative* result (`../outside.md`)
    /// instead of an absolute one — `site.json` must never contain an absolute
    /// host path. Both inputs are absolute in practice, so the fallback is
    /// unreachable.
    fn relative_to_root(&self, abs_path: &Path) -> PathBuf {
        pathdiff::diff_paths(abs_path, &self.canonical_root)
            .unwrap_or_else(|| abs_path.to_path_buf())
    }

    /// Decides whether the scanner may descend into `abs_path`, and says which
    /// of the two legitimate roots it belongs to.
    ///
    /// Returns the path's *root-relative* form alongside, because that is what
    /// `scan_folder` re-joins onto `canonical_root` on the next hop. For the
    /// overlay that form keeps its `../static/…` shape, which round-trips
    /// through `join` correctly and which `build_static_url_path` strips back
    /// off when it builds the URL.
    ///
    /// `None` means "outside everything the scanner may walk" — the escaping
    /// directory symlink case, which must stay refused.
    fn scannable(&self, abs_path: &Path) -> Option<(ScanLocation, PathBuf)> {
        if let Some(relative) = repo_relative_within_root(&self.canonical_root, abs_path) {
            return Some((ScanLocation::WithinRoot, relative));
        }

        let overlay = self.canonical_static_root.as_deref()?;
        if !abs_path.starts_with(overlay) {
            return None;
        }
        let relative = pathdiff::diff_paths(abs_path, &self.canonical_root)?;
        Some((ScanLocation::StaticOverlay, relative))
    }

    pub fn scan_folder<P: AsRef<Path>>(&self, relative_folder_path: &P) -> Result<(), RepoError> {
        let relative_folder_path_ref = relative_folder_path.as_ref();
        let joined = self.canonical_root.join(relative_folder_path_ref);
        let start_folder =
            joined
                .canonicalize()
                .map_err(|source| RepoError::CanonicalizeFailed {
                    path: joined.clone(),
                    source,
                })?;

        // A directory symlink can resolve outside the repository root, and the
        // `canonicalize` above re-roots the walk at its target. Every file found
        // below it would then relativize to `../…`, which `url_path::path_to_url`
        // deliberately preserves — so the URL escapes the site and, in build
        // mode, `output_dir.join(url_path)` writes pages outside `--output`.
        // Refuse to descend instead.
        //
        // The validated static overlay is the one exception, and a narrow one:
        // it is a specific directory the config policy already approved, not a
        // general loosening. Every other out-of-root path is still refused.
        let Some((location, _)) = self.scannable(&start_folder) else {
            tracing::warn!(
                "Skipping {:?}: it resolves to {}, outside the repository root {}",
                relative_folder_path_ref,
                start_folder.display(),
                self.canonical_root.display()
            );
            return Ok(());
        };

        // Skip if already scanned
        if self.scanned_folders.pin().contains(&start_folder) {
            return Ok(());
        }
        tracing::debug!("Scanning folder: {:?}", relative_folder_path_ref);
        self.scanned_folders.pin().insert(start_folder.clone());

        // Walk directory with filtering (using pre-compiled patterns for efficiency)
        let walkdir_start = Instant::now();
        let dir_walker = WalkDir::new(start_folder.clone())
            .follow_links(true)
            .min_depth(1)
            .max_depth(1)
            .into_iter()
            .filter_entry(|e| {
                !should_ignore_compiled(e.path(), &self.ignore_dirs, &self.compiled_ignore_globs)
            });

        let mut markdown = std::collections::HashMap::new();
        let mut other = std::collections::HashMap::new();

        for entry in dir_walker.filter_map(|e| e.ok()) {
            let path = entry.path();
            let extension = path.extension().and_then(|x| x.to_str()).unwrap_or("");

            if path.is_dir() {
                // Queue subdirectory for later scanning. Anything outside both
                // the root and the validated static overlay must not be walked
                // (the recursive call would re-root the walk there).
                match self.scannable(path) {
                    Some((_, relative_entry)) => {
                        self.queued_folders
                            .pin()
                            .insert(path.to_path_buf(), relative_entry);
                    }
                    None => tracing::debug!(
                        "Not queueing {}: outside the repository root {}",
                        path.display(),
                        self.canonical_root.display()
                    ),
                }
            } else if location == ScanLocation::StaticOverlay
                && is_markdown_extension(extension, &self.markdown_extensions)
            {
                // Markdown inside an *external* static overlay has no
                // representable URL: `build_markdown_url_path` relativizes it to
                // `../static/…`, `url_path::path_to_url` preserves the `..` on
                // purpose, and build mode then joins that onto `output_dir` —
                // writing the page outside `--output`. That escape is exactly
                // what this pass fixed in `build.rs`, so the file is skipped
                // rather than indexed with a broken URL. Static assets are
                // unaffected: `build_static_url_path` strips the overlay prefix.
                tracing::warn!(
                    "Skipping markdown file {} in the external static folder {:?}: \
                     markdown outside the repository root has no valid URL",
                    path.display(),
                    self.static_folder
                );
            } else if is_markdown_extension(extension, &self.markdown_extensions) {
                // Process markdown file
                if let Ok((_filesize, created, modified)) = file_details_from_path(path) {
                    let url = build_markdown_url_path(path, &self.canonical_root, &self.index_file);
                    let mdfile = MarkdownInfo {
                        raw_path: self.relative_to_root(path),
                        url_path: url,
                        created,
                        modified,
                        frontmatter: None,
                        relationships: Vec::new(),
                    };
                    markdown.insert(path.to_path_buf(), mdfile);
                } else {
                    tracing::warn!("Couldn't process markdown file at {:?}", path);
                }
            } else {
                // Process static file
                //
                // Inside the overlay, strip the *resolved* directory rather than
                // the configured string. `build_static_url_path` strips the raw
                // value with a component-wise `strip_prefix`, so `../../static`
                // and the equivalent `./../../static` — both accepted by the
                // config policy — do not both match, and the second would leave
                // a `..` in the URL. `path` is canonical here: `scan_folder`
                // canonicalizes the walk root and `WalkDir` prefixes every entry
                // with it.
                let url = match (location, self.canonical_static_root.as_deref()) {
                    (ScanLocation::StaticOverlay, Some(overlay)) => format!(
                        "/{}",
                        crate::url_path::path_to_url(path.strip_prefix(overlay).unwrap_or(path))
                    ),
                    _ => build_static_url_path(path, &self.canonical_root, &self.static_folder),
                };
                let other_file = OtherFileInfo {
                    raw_path: path.to_path_buf(),
                    url_path: url,
                    metadata: StaticFileMetadata::empty(path),
                    extracted_text: None,
                };
                other.insert(path.to_path_buf(), other_file);
            }
        }
        tracing::debug!(
            "scan_folder WalkDir for {:?}: {} markdown, {} other files in {:?}",
            relative_folder_path_ref,
            markdown.len(),
            other.len(),
            walkdir_start.elapsed()
        );

        // Parallel processing: extract frontmatter from markdown files and build tag index
        let frontmatter_start = Instant::now();
        markdown
            .into_par_iter()
            .for_each(|(mdfile, mddetails): (PathBuf, MarkdownInfo)| {
                let file_meta = crate::markdown::extract_metadata_from_file(&mdfile).ok();
                let details = if let Some(file_meta) = file_meta {
                    let frontmatter = file_meta.metadata;
                    let relationships = file_meta.relationships;
                    // Extract tags from frontmatter for each configured tag source
                    let title = get_page_title(&frontmatter, &mddetails.raw_path);
                    let description = frontmatter
                        .get("description")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());

                    for tag_source in &self.tag_sources {
                        // Look up the field (supports dot notation like "taxonomy.tags")
                        if let Some(tag_value_json) = frontmatter.get(&tag_source.field) {
                            // Extract tag values (handles both arrays and comma-separated strings)
                            for tag_value in extract_tag_values(tag_value_json) {
                                let page = if let Some(ref desc) = description {
                                    TaggedPage::with_description(
                                        &mddetails.url_path,
                                        &title,
                                        desc,
                                        &tag_value,
                                    )
                                } else {
                                    TaggedPage::new(&mddetails.url_path, &title, &tag_value)
                                };
                                self.tag_index.add_page(&tag_source.field, &tag_value, page);
                            }
                        }
                    }

                    MarkdownInfo {
                        frontmatter: Some(frontmatter),
                        relationships,
                        ..mddetails
                    }
                } else {
                    mddetails
                };
                self.markdown_files.pin().insert(mdfile, details);
            });
        tracing::debug!(
            "scan_folder frontmatter extraction for {:?}: {:?}",
            relative_folder_path_ref,
            frontmatter_start.elapsed()
        );

        // Register other files without stat calls — basic metadata (size, timestamps)
        // is deferred to populate_basic_metadata() to avoid blocking scan_all() completion.
        let static_insert_start = Instant::now();
        for (file, other_file) in other {
            self.other_files.pin().insert(file, other_file);
        }
        tracing::debug!(
            "scan_folder static file registration for {:?}: {:?}",
            relative_folder_path_ref,
            static_insert_start.elapsed()
        );

        Ok(())
    }

    pub fn scan_all(&self) -> Result<(), RepoError> {
        let scan_all_start = Instant::now();

        // Pre-mark static folder as scanned to skip it during content scan.
        // This defers static file registration to scan_static_folder() so
        // mark_scan_complete() fires faster (search only needs markdown).
        let static_deferred = self
            .canonical_root
            .join(&self.static_folder)
            .canonicalize()
            .ok()
            .inspect(|p| {
                self.scanned_folders.pin().insert(p.clone());
            });

        self.scan_folder(&PathBuf::from("."))?; // the . is relative to the root_dir, so this scans the root dir

        while !self.queued_folders.is_empty() {
            // TODO: make sure this doesn't deadlock
            let vec_folders: Vec<_> = self
                .queued_folders
                .pin()
                .iter()
                .map(|(_, relative)| relative.clone())
                .collect();
            self.queued_folders.pin().clear();
            tracing::debug!("Parallel batch: {:?}", &vec_folders);
            vec_folders.into_par_iter().for_each(|rel_path| {
                self.scan_folder(&rel_path).unwrap_or_else(|e| {
                    tracing::error!("Failed to scan folder {:?}: {e}", &rel_path)
                }) // ignores errors
            });
        }

        // Un-mark static folder so scan_static_folder() can scan it later
        if let Some(ref sp) = static_deferred {
            self.scanned_folders.pin().remove(sp);
        }

        // Log file counts and type breakdown
        let markdown_count = self.markdown_files.len();
        let other_count = self.other_files.len();
        let other_pin = self.other_files.pin();
        let mut pdf_count: usize = 0;
        let mut image_count: usize = 0;
        let mut video_count: usize = 0;
        let mut audio_count: usize = 0;
        let mut text_count: usize = 0;
        let mut misc_count: usize = 0;
        for (_, info) in other_pin.iter() {
            match info.filetype() {
                "pdf" => pdf_count += 1,
                "image" => image_count += 1,
                "video" => video_count += 1,
                "audio" => audio_count += 1,
                "text" => text_count += 1,
                _ => misc_count += 1,
            }
        }

        tracing::info!(
            "scan_all completed in {:?}: {} markdown files, {} other files (pdf={}, image={}, video={}, audio={}, text={}, other={})",
            scan_all_start.elapsed(),
            markdown_count,
            other_count,
            pdf_count,
            image_count,
            video_count,
            audio_count,
            text_count,
            misc_count,
        );

        Ok(())
    }

    /// Scan the static folder and its subdirectories.
    /// Deferred from scan_all() so mark_scan_complete() fires faster.
    pub fn scan_static_folder(&self) -> Result<(), RepoError> {
        let start = Instant::now();
        let static_path = self.canonical_root.join(&self.static_folder);
        if !static_path.is_dir() {
            return Ok(());
        }

        self.scan_folder(&PathBuf::from(&self.static_folder))?;

        while !self.queued_folders.is_empty() {
            let vec_folders: Vec<_> = self
                .queued_folders
                .pin()
                .iter()
                .map(|(_, relative)| relative.clone())
                .collect();
            self.queued_folders.pin().clear();
            vec_folders.into_par_iter().for_each(|rel_path| {
                self.scan_folder(&rel_path).unwrap_or_else(|e| {
                    tracing::error!("Failed to scan folder {:?}: {e}", &rel_path)
                })
            });
        }

        let other_count = self.other_files.len();
        tracing::info!(
            "scan_static_folder completed in {:?}: {} other files total",
            start.elapsed(),
            other_count,
        );
        Ok(())
    }

    /// Mark the initial scan as complete and notify any waiters.
    pub fn mark_scan_complete(&self) {
        self.scan_complete.store(true, Ordering::SeqCst);
        self.scan_notify.notify_waiters();
    }

    /// Drop every cached view of the repository and rebuild it from disk.
    ///
    /// Used by the watcher when a change batch is too large to invalidate file
    /// by file. Blocking; call from `spawn_blocking`.
    ///
    /// **No step may return early.** `clear()` resets `media_populated`, so any
    /// path that skips [`Repo::notify_media_populated`] leaves every later
    /// [`Repo::wait_for_media`] — that is, `/.mbr/media.json` — blocked for the
    /// life of the process. A failed scan is logged and the pass continues on
    /// whatever was indexed: `clear()` already discarded the previous contents,
    /// so the real choice is between serving partial data and hanging.
    pub fn full_rescan(&self) {
        self.clear();
        if let Err(e) = self.scan_all() {
            tracing::error!("Rescan failed, continuing with partial repository state: {e}");
        }
        self.build_relationship_index();
        self.build_wikilink_index();
        if let Err(e) = self.scan_static_folder() {
            tracing::error!("Static folder rescan failed: {e}");
        }
        self.populate_basic_metadata();
        self.populate_media_metadata();
        self.notify_media_populated();
        self.ensure_text_extracted();
    }

    /// Populate basic file metadata (size, timestamps) for all other files.
    /// Deferred from scan_folder() to avoid blocking scan_all() completion,
    /// so search (which only needs markdown files) can proceed sooner.
    pub fn populate_basic_metadata(&self) {
        let start = Instant::now();
        let pin = self.other_files.pin();
        let keys: Vec<PathBuf> = pin
            .iter()
            .filter(|(_, info)| info.metadata.file_size_bytes.is_none())
            .map(|(k, _)| k.clone())
            .collect();
        let count = keys.len();
        drop(pin);

        keys.into_par_iter().for_each(|key| {
            let pin = self.other_files.pin();
            if let Some(info) = pin.get(&key) {
                let updated = OtherFileInfo {
                    metadata: info.metadata.clone().populate_basic(),
                    ..info.clone()
                };
                drop(pin);
                self.other_files.pin().insert(key, updated);
            }
        });

        tracing::info!(
            "populate_basic_metadata completed for {} files in {:?}",
            count,
            start.elapsed()
        );
    }

    /// Returns true if the initial scan has completed.
    pub fn is_scan_complete(&self) -> bool {
        self.scan_complete.load(Ordering::SeqCst)
    }

    /// Wait for the initial scan to complete. Returns immediately if already done.
    ///
    /// `mark_scan_complete()` fires exactly once and `Notify::notify_waiters()`
    /// stores no permit, so a notification observed between a flag check and the
    /// registration of the waiter would be lost forever — and `/.mbr/site.json`
    /// would block for the life of the process. Registering interest *before*
    /// re-checking closes that window (same pattern as the in-flight waits in
    /// `server.rs`).
    pub async fn wait_for_scan(&self) {
        loop {
            let notified = self.scan_notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();

            if self.is_scan_complete() {
                return;
            }

            // Re-checked by the loop: `notified()` also returns on spurious wakeups.
            notified.await;
        }
    }

    /// Returns true if media metadata has been populated.
    pub fn is_media_populated(&self) -> bool {
        self.media_populated.load(Ordering::Acquire)
    }

    /// Notify waiters that media metadata population is complete.
    pub fn notify_media_populated(&self) {
        self.media_notify.notify_waiters();
    }

    /// Wait for media metadata to be populated. Returns immediately if already done.
    ///
    /// Registers interest before re-checking the flag, for the same reason as
    /// [`Repo::wait_for_scan`].
    pub async fn wait_for_media(&self) {
        loop {
            let notified = self.media_notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();

            if self.is_media_populated() {
                return;
            }

            notified.await;
        }
    }

    pub fn to_json(&self) -> serde_json::Result<String> {
        serde_json::to_string(self)
    }

    /// Populate media metadata (ffmpeg/lopdf) for all static files.
    /// This is deferred from initial scan to avoid blocking the first site.json response.
    pub fn populate_media_metadata(&self) {
        if self.media_population_started.swap(true, Ordering::SeqCst) {
            return; // Already running, or already finished
        }
        let start = Instant::now();
        let pin = self.other_files.pin();
        let entries: Vec<_> = pin.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        drop(pin);

        #[cfg(feature = "media-metadata")]
        {
            entries.into_par_iter().for_each(|(key, info)| {
                let updated = OtherFileInfo {
                    metadata: info.metadata.populate_media(),
                    ..info
                };
                self.other_files.pin().insert(key, updated);
            });
        }
        #[cfg(not(feature = "media-metadata"))]
        let _ = entries;

        // Publish "finished" only now that every entry carries its metadata.
        // Callers invoke `notify_media_populated()` immediately after this
        // returns; waiters re-read the flag after being woken, so the store must
        // happen first.
        self.media_populated.store(true, Ordering::Release);

        tracing::info!("populate_media_metadata completed in {:?}", start.elapsed());
    }

    /// Extract text from searchable files (PDFs, text files) for search indexing.
    /// Deferred from initial scan since text is only needed for search.
    pub fn ensure_text_extracted(&self) {
        if self.text_extracted.swap(true, Ordering::SeqCst) {
            return; // Already extracted
        }
        let start = Instant::now();
        let pin = self.other_files.pin();
        let entries: Vec<_> = pin
            .iter()
            .filter(|(_, info)| info.is_searchable() && info.extracted_text.is_none())
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        drop(pin);

        entries.into_par_iter().for_each(|(key, mut info)| {
            info.extracted_text = info.extract_text();
            self.other_files.pin().insert(key, info);
        });

        tracing::info!("ensure_text_extracted completed in {:?}", start.elapsed());
    }

    /// Clear all cached data, forcing a full rescan on next scan_all() call.
    ///
    /// Call this when files are added, removed, or modified to ensure
    /// the next scan picks up the changes.
    pub fn clear(&self) {
        self.scanned_folders.pin().clear();
        self.markdown_files.pin().clear();
        self.other_files.pin().clear();
        self.queued_folders.pin().clear();
        self.tag_index.clear();
        self.relationship_index.clear();
        self.wikilink_index.clear();
        self.text_extracted.store(false, Ordering::SeqCst);
        self.media_populated.store(false, Ordering::SeqCst);
        // Must be reset too, or the run-once guard would make the rescan's
        // `populate_media_metadata()` a no-op and leave `media_populated` false
        // forever, hanging every later `wait_for_media()`.
        self.media_population_started.store(false, Ordering::SeqCst);
        // Note: scan_complete is NOT reset here. It tracks whether the initial background
        // scan finished. After clear(), scan_all() will re-scan synchronously in handlers.
    }

    /// Surgically invalidate a single file, updating only the affected cache entries.
    ///
    /// Much cheaper than `clear()` + `scan_all()` for small batches of file changes.
    pub fn invalidate_file(&self, abs_path: &Path, event: &crate::watcher::ChangeEventType) {
        let extension = abs_path.extension().and_then(|x| x.to_str()).unwrap_or("");
        let is_markdown = is_markdown_extension(extension, &self.markdown_extensions);

        match event {
            crate::watcher::ChangeEventType::Deleted => {
                if is_markdown {
                    self.markdown_files.pin().remove(abs_path);
                } else {
                    self.other_files.pin().remove(abs_path);
                }
            }
            crate::watcher::ChangeEventType::Created => {
                if is_markdown {
                    if let Ok((_filesize, created, modified)) = file_details_from_path(abs_path) {
                        let url = build_markdown_url_path(
                            abs_path,
                            &self.canonical_root,
                            &self.index_file,
                        );
                        let file_meta = crate::markdown::extract_metadata_from_file(abs_path).ok();
                        let (frontmatter, relationships) = match file_meta {
                            Some(fm) => (Some(fm.metadata), fm.relationships),
                            None => (None, Vec::new()),
                        };

                        // Add tags from frontmatter
                        if let Some(ref fm) = frontmatter {
                            let title = get_page_title(fm, abs_path);
                            let description = fm
                                .get("description")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string());
                            for tag_source in &self.tag_sources {
                                if let Some(tag_value_json) = fm.get(&tag_source.field) {
                                    for tag_value in extract_tag_values(tag_value_json) {
                                        let page = if let Some(ref desc) = description {
                                            TaggedPage::with_description(
                                                &url, &title, desc, &tag_value,
                                            )
                                        } else {
                                            TaggedPage::new(&url, &title, &tag_value)
                                        };
                                        self.tag_index.add_page(
                                            &tag_source.field,
                                            &tag_value,
                                            page,
                                        );
                                    }
                                }
                            }
                        }

                        let info = MarkdownInfo {
                            raw_path: self.relative_to_root(abs_path),
                            url_path: url,
                            created,
                            modified,
                            frontmatter,
                            relationships,
                        };
                        self.markdown_files
                            .pin()
                            .insert(abs_path.to_path_buf(), info);
                    }
                } else {
                    let url =
                        build_static_url_path(abs_path, &self.canonical_root, &self.static_folder);
                    let info = OtherFileInfo {
                        raw_path: abs_path.to_path_buf(),
                        url_path: url,
                        metadata: StaticFileMetadata::empty(abs_path).populate_basic(),
                        extracted_text: None,
                    };
                    self.other_files.pin().insert(abs_path.to_path_buf(), info);
                }
            }
            crate::watcher::ChangeEventType::Modified => {
                if is_markdown {
                    // Re-extract frontmatter and update
                    if let Ok((_filesize, created, modified)) = file_details_from_path(abs_path) {
                        let url = build_markdown_url_path(
                            abs_path,
                            &self.canonical_root,
                            &self.index_file,
                        );
                        let file_meta = crate::markdown::extract_metadata_from_file(abs_path).ok();
                        let (frontmatter, relationships) = match file_meta {
                            Some(fm) => (Some(fm.metadata), fm.relationships),
                            None => (None, Vec::new()),
                        };
                        let info = MarkdownInfo {
                            raw_path: self.relative_to_root(abs_path),
                            url_path: url,
                            created,
                            modified,
                            frontmatter,
                            relationships,
                        };
                        self.markdown_files
                            .pin()
                            .insert(abs_path.to_path_buf(), info);
                    }
                } else {
                    // Update basic metadata for modified static files
                    let url =
                        build_static_url_path(abs_path, &self.canonical_root, &self.static_folder);
                    let info = OtherFileInfo {
                        raw_path: abs_path.to_path_buf(),
                        url_path: url,
                        metadata: StaticFileMetadata::empty(abs_path).populate_basic(),
                        extracted_text: None,
                    };
                    self.other_files.pin().insert(abs_path.to_path_buf(), info);
                }
            }
        }
    }

    /// Rebuild the tag index from the current cached markdown files.
    ///
    /// Call this after surgical file invalidation when tags may have changed
    /// (e.g., after deleting or modifying markdown files).
    /// Returns the configured tag sources.
    pub fn tag_sources(&self) -> &[TagSource] {
        &self.tag_sources
    }

    /// Rebuild the relationship index from the current cached markdown files.
    ///
    /// Must run after a scan (or file-change invalidation) once all note titles
    /// are known, since endpoint resolution matches on titles and filename
    /// stems across the whole repo. Modelled on [`Self::rebuild_tag_index`].
    pub fn build_relationship_index(&self) {
        // Short-circuit when no note declares any relationship: avoid the
        // per-note NoteRelInput cloning and the O(n log n) sort / name-index /
        // rebuild in `build_relationship_map` (mirrors the gated tag-index
        // rebuild). An empty rebuild is otherwise pure overhead.
        if !self
            .markdown_files
            .pin()
            .iter()
            .any(|(_, info)| !info.relationships.is_empty())
        {
            self.relationship_index.clear();
            return;
        }
        let notes = self.collect_note_inputs();
        self.relationship_index
            .rebuild(&notes, &self.markdown_extensions);
    }

    /// Rebuild the global wikilink name index from the current cached markdown
    /// files.
    ///
    /// Unlike [`Self::build_relationship_index`] this is **ungated** — the index
    /// powers Obsidian-style `[[Name]]` body-link resolution for every repo, so
    /// it is rebuilt whenever the repo is (re)scanned or a file changes. Must run
    /// after a scan once all note titles/stems are known.
    pub fn build_wikilink_index(&self) {
        let notes = self.collect_note_inputs();
        self.wikilink_index.rebuild(&notes);
    }

    /// Assembles one [`NoteRelInput`] per cached markdown file (url, title,
    /// filename stem, frontmatter aliases, index flag, declared relationships).
    ///
    /// Shared by [`Self::build_relationship_index`] and
    /// [`Self::build_wikilink_index`] so the per-note title/stem/alias derivation
    /// lives in one place.
    fn collect_note_inputs(&self) -> Vec<NoteRelInput> {
        let pin = self.markdown_files.pin();
        pin.iter()
            .map(|(_, info)| {
                let title = info
                    .frontmatter
                    .as_ref()
                    .map(|fm| get_page_title(fm, &info.raw_path))
                    .unwrap_or_else(|| {
                        info.raw_path
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or("Untitled")
                            .to_string()
                    });
                let stem = info
                    .raw_path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or_default()
                    .to_string();
                // Alternate names (e.g. maiden names) that also resolve to this
                // note. Read from a frontmatter `aliases` array of strings;
                // non-string elements and wrong types are ignored (empty vec).
                let aliases = info
                    .frontmatter
                    .as_ref()
                    .and_then(|fm| fm.get("aliases"))
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(str::to_string))
                            .collect::<Vec<String>>()
                    })
                    .unwrap_or_default();
                let is_index = info
                    .raw_path
                    .file_name()
                    .and_then(|f| f.to_str())
                    .is_some_and(|f| f == self.index_file);
                NoteRelInput {
                    url: info.url_path.clone(),
                    title,
                    stem,
                    aliases,
                    is_index,
                    relationships: info.relationships.clone(),
                }
            })
            .collect()
    }

    pub fn rebuild_tag_index(&self) {
        self.tag_index.clear();
        let pin = self.markdown_files.pin();
        for (_, info) in pin.iter() {
            if let Some(ref fm) = info.frontmatter {
                let title = get_page_title(fm, &info.raw_path);
                let description = fm
                    .get("description")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                for tag_source in &self.tag_sources {
                    if let Some(tag_value_json) = fm.get(&tag_source.field) {
                        for tag_value in extract_tag_values(tag_value_json) {
                            let page = if let Some(ref desc) = description {
                                TaggedPage::with_description(
                                    &info.url_path,
                                    &title,
                                    desc,
                                    &tag_value,
                                )
                            } else {
                                TaggedPage::new(&info.url_path, &title, &tag_value)
                            };
                            self.tag_index.add_page(&tag_source.field, &tag_value, page);
                        }
                    }
                }
            }
        }
    }
}

/// Returns file_size, created_secs, modified_secs
pub fn file_details_from_path<P: AsRef<Path>>(path: P) -> Result<(u64, u64, u64), RepoError> {
    let path = path.as_ref();
    let metadata = std::fs::metadata(path).map_err(|source| RepoError::MetadataFailed {
        path: path.to_path_buf(),
        source,
    })?;

    let file_size = metadata.len();

    // Modified time
    let modified = metadata
        .modified()
        .map_err(|source| RepoError::MetadataFailed {
            path: path.to_path_buf(),
            source,
        })?;
    let modified_secs = modified.duration_since(UNIX_EPOCH)?.as_secs();

    // Created time (birth time) is not available on all filesystems (e.g. older Linux
    // kernels, some NFS mounts). Fall back to the modified time rather than failing, so
    // the file is never silently dropped from listings/search when btime is unavailable.
    let created_secs = metadata
        .created()
        .ok()
        .and_then(|created| created.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(modified_secs);

    Ok((file_size, created_secs, modified_secs))
}

// ============================================================================
// Pure helper functions for repo scanning (extracted for testability)
// ============================================================================

/// Returns the repo-relative form of `abs_path`, or `None` when it resolves
/// *outside* `root`.
///
/// Containment is decided component-wise, not with `Path::starts_with`: the
/// relative path is what the rest of the pipeline consumes, and a single
/// `ParentDir` in it means the file is not in the repository at all. A
/// `RootDir`/`Prefix` component means [`pathdiff::diff_paths`] could not
/// relativize the two at all (mixed absolute/relative inputs, or different
/// Windows drives) and handed back something absolute.
///
/// This is the scan-side half of the containment guard: `url_path::path_to_url`
/// preserves `..` on purpose, so an escaping path stays escaped all the way into
/// `site.json` URLs and into `output_dir.join(url_path)` during a static build.
/// Which of the scanner's two legitimate roots a path belongs to.
///
/// There are exactly two: the repository root, and the validated external static
/// overlay. Anything else is not scannable at all, so this enum has no third
/// variant by design.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScanLocation {
    WithinRoot,
    StaticOverlay,
}

pub fn repo_relative_within_root(root: &Path, abs_path: &Path) -> Option<PathBuf> {
    let relative = pathdiff::diff_paths(abs_path, root)?;

    let escapes = relative.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    });

    (!escapes).then_some(relative)
}

/// Checks if a path should be ignored based on the given rules.
///
/// A path is ignored if:
/// - Its name starts with '.'
/// - It's a directory matching one of the ignore_dirs
/// - It matches one of the ignore_globs patterns
pub fn should_ignore(path: &Path, ignore_dirs: &[String], ignore_globs: &[String]) -> bool {
    let file_name = path.file_name().and_then(|x| x.to_str()).unwrap_or("");

    // Hidden files/dirs (starting with .)
    if file_name.starts_with('.') {
        return true;
    }

    // Directory matching ignore list
    if path.is_dir() && ignore_dirs.iter().any(|x| x.as_str() == file_name) {
        return true;
    }

    // Glob pattern match
    ignore_globs.iter().any(|pat| {
        glob::Pattern::new(pat)
            .map(|pat| pat.matches_path(path))
            .unwrap_or(false)
    })
}

/// Checks if a path should be ignored using pre-compiled glob patterns.
/// This is more efficient than `should_ignore` when processing many files.
fn should_ignore_compiled(
    path: &Path,
    ignore_dirs: &[String],
    compiled_patterns: &[glob::Pattern],
) -> bool {
    let file_name = path.file_name().and_then(|x| x.to_str()).unwrap_or("");

    // Hidden files/dirs (starting with .)
    if file_name.starts_with('.') {
        return true;
    }

    // Directory matching ignore list
    if path.is_dir() && ignore_dirs.iter().any(|x| x.as_str() == file_name) {
        return true;
    }

    // Pre-compiled glob pattern match
    compiled_patterns.iter().any(|pat| pat.matches_path(path))
}

/// Builds a URL path for a markdown file.
///
/// Converts a filesystem path relative to root into a URL path:
/// - Ensures leading slash
/// - Removes index file from path (e.g., /docs/index.md → /docs/)
/// - Replaces file extension with trailing slash
pub fn build_markdown_url_path(path: &Path, root_dir: &Path, index_file: &str) -> String {
    // `path_to_url` (not `to_string_lossy`) so the separators below are `/` on
    // every platform; otherwise the `rsplit('/')` and `contains('/')` checks
    // that follow silently stop matching on Windows.
    let relative = pathdiff::diff_paths(path, root_dir).unwrap_or_default();
    let mut url = format!("/{}", crate::url_path::path_to_url(&relative));

    // Remove index file from path — only when the final path component (file name)
    // exactly equals the index file, not merely when it's a suffix substring.
    // Otherwise "docs/myindex.md" would be wrongly truncated for index_file "index.md".
    if url.rsplit('/').next() == Some(index_file) {
        url.truncate(url.len() - index_file.len());
    }

    // Replace extension with trailing slash
    if let Some(dot_pos) = url.rfind('.')
        && !url[dot_pos..].contains('/')
    {
        url.truncate(dot_pos);
        url.push('/');
    }

    url
}

/// Builds a URL path for a static file.
///
/// Converts a filesystem path relative to root into a URL path:
/// - Removes static folder prefix
/// - Ensures leading slash
pub fn build_static_url_path(path: &Path, root_dir: &Path, static_folder: &str) -> String {
    let relative = pathdiff::diff_paths(path, root_dir).unwrap_or_default();

    // Strip only a *leading* `{static_folder}/` path component, not any substring
    // occurrence of the folder name. A plain `.replacen` would corrupt paths like
    // "notes/static-analysis/img.png" (with static_folder="static").
    let stripped = relative.strip_prefix(static_folder).unwrap_or(&relative);

    format!("/{}", crate::url_path::path_to_url(stripped))
}

/// Checks if a file has a markdown extension.
pub fn is_markdown_extension(extension: &str, markdown_extensions: &[String]) -> bool {
    markdown_extensions.iter().any(|x| x.as_str() == extension)
}

/// Parses a comma-separated string of tag values into individual tags.
///
/// Handles whitespace around commas and filters empty values.
///
/// # Examples
///
/// ```
/// use mbr::repo::parse_tag_values;
///
/// let tags: Vec<String> = parse_tag_values("rust, programming, web dev").collect();
/// assert_eq!(tags, vec!["rust", "programming", "web dev"]);
/// ```
pub fn parse_tag_values(values: &str) -> impl Iterator<Item = String> + '_ {
    values
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Extract tag values from a serde_json::Value (supports both arrays and comma-separated strings).
///
/// # Examples
///
/// ```
/// use mbr::repo::extract_tag_values;
///
/// // From array
/// let val = serde_json::json!(["rust", "python"]);
/// assert_eq!(extract_tag_values(&val), vec!["rust", "python"]);
///
/// // From comma-separated string
/// let val = serde_json::json!("rust, python");
/// assert_eq!(extract_tag_values(&val), vec!["rust", "python"]);
/// ```
pub fn extract_tag_values(value: &serde_json::Value) -> Vec<String> {
    match value {
        serde_json::Value::String(s) => parse_tag_values(s).collect(),
        serde_json::Value::Array(arr) => arr
            .iter()
            .filter_map(|v| v.as_str())
            .map(|s| s.to_string())
            .collect(),
        _ => vec![],
    }
}

/// Gets the page title from frontmatter or falls back to filename.
///
/// Priority:
/// 1. `title` field in frontmatter
/// 2. Filename stem (without extension)
fn get_page_title(frontmatter: &crate::markdown::SimpleMetadata, path: &Path) -> String {
    frontmatter
        .get("title")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("Untitled")
                .to_string()
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `Repo` must canonicalize the root it is handed, so that the base used to
    /// relativize scanned paths is identical to the one `WalkDir` starts from.
    /// If these drift, `diff_paths` returns an effectively absolute path and
    /// every `url_path` embeds the whole filesystem path.
    #[test]
    fn test_repo_canonicalizes_its_root() {
        let tmp = tempfile::tempdir().unwrap();
        let canonical = tmp.path().canonicalize().unwrap();

        let repo = Repo::init(
            tmp.path().to_path_buf(),
            "static".to_string(),
            &["md".to_string()],
            &[],
            &[],
            "index.md".to_string(),
            &[],
            &[],
        );

        assert_eq!(
            repo.canonical_root, canonical,
            "Repo must store the canonical root, not the path as supplied"
        );
    }

    /// Canonicalization must not lose a root that does not exist yet; falling
    /// back to the given path keeps the two sides consistent.
    #[test]
    fn test_canonicalize_root_falls_back_when_missing() {
        let missing = PathBuf::from("/definitely/does/not/exist/anywhere");
        assert_eq!(canonicalize_root(missing.clone()), missing);
    }

    #[test]
    fn test_should_ignore_hidden_file() {
        let path = Path::new(".hidden");
        assert!(should_ignore(path, &[], &[]));
    }

    #[test]
    fn test_should_ignore_hidden_dir() {
        let path = Path::new(".git");
        assert!(should_ignore(path, &[], &[]));
    }

    #[test]
    fn test_should_ignore_normal_file() {
        let path = Path::new("readme.md");
        assert!(!should_ignore(path, &[], &[]));
    }

    #[test]
    fn test_should_ignore_glob_pattern() {
        let path = Path::new("test.log");
        let globs = vec!["*.log".to_string()];
        assert!(should_ignore(path, &[], &globs));
    }

    #[test]
    fn test_should_ignore_glob_no_match() {
        let path = Path::new("test.md");
        let globs = vec!["*.log".to_string()];
        assert!(!should_ignore(path, &[], &globs));
    }

    #[test]
    fn test_build_markdown_url_path_simple() {
        let root = Path::new("/root");
        let path = Path::new("/root/readme.md");
        assert_eq!(build_markdown_url_path(path, root, "index.md"), "/readme/");
    }

    #[test]
    fn test_build_markdown_url_path_nested() {
        let root = Path::new("/root");
        let path = Path::new("/root/docs/guide.md");
        assert_eq!(
            build_markdown_url_path(path, root, "index.md"),
            "/docs/guide/"
        );
    }

    #[test]
    fn test_build_markdown_url_path_index() {
        let root = Path::new("/root");
        let path = Path::new("/root/docs/index.md");
        assert_eq!(build_markdown_url_path(path, root, "index.md"), "/docs/");
    }

    #[test]
    fn test_build_markdown_url_path_root_index() {
        let root = Path::new("/root");
        let path = Path::new("/root/index.md");
        assert_eq!(build_markdown_url_path(path, root, "index.md"), "/");
    }

    #[test]
    fn test_build_static_url_path_in_static() {
        let root = Path::new("/root");
        let path = Path::new("/root/static/image.png");
        assert_eq!(build_static_url_path(path, root, "static"), "/image.png");
    }

    #[test]
    fn test_build_static_url_path_not_in_static() {
        let root = Path::new("/root");
        let path = Path::new("/root/assets/image.png");
        assert_eq!(
            build_static_url_path(path, root, "static"),
            "/assets/image.png"
        );
    }

    // Regression (Bug #8): the static folder name appearing as a substring of a
    // path component must NOT be stripped. Only a leading `{static_folder}/`
    // component is removed. Previously `.replacen` corrupted these paths.
    #[test]
    fn test_build_static_url_path_preserves_static_substring() {
        let root = Path::new("/root");

        // "static" appears inside a directory name, not as a leading component.
        let path = Path::new("/root/notes/static-analysis/img.png");
        assert_eq!(
            build_static_url_path(path, root, "static"),
            "/notes/static-analysis/img.png"
        );

        // "static" appears inside a nested (non-leading) directory name.
        let nested = Path::new("/root/my-static/image.png");
        assert_eq!(
            build_static_url_path(nested, root, "static"),
            "/my-static/image.png"
        );

        // Correctly-prefixed paths still have the leading component stripped.
        let prefixed = Path::new("/root/static/static-report.png");
        assert_eq!(
            build_static_url_path(prefixed, root, "static"),
            "/static-report.png"
        );
    }

    // Regression (Bug #9): a file whose name merely ends with the index file
    // name (e.g. "myindex.md" for index_file "index.md") must NOT be treated as
    // an index file. Previously `ends_with` truncated it to "/docs/my/".
    #[test]
    fn test_build_markdown_url_path_myindex_not_treated_as_index() {
        let root = Path::new("/root");

        let path = Path::new("/root/docs/myindex.md");
        assert_eq!(
            build_markdown_url_path(path, root, "index.md"),
            "/docs/myindex/"
        );

        // A genuine index file is still collapsed to a trailing slash.
        let real_index = Path::new("/root/docs/index.md");
        assert_eq!(
            build_markdown_url_path(real_index, root, "index.md"),
            "/docs/"
        );
    }

    // Regression (Bug #10): file_details_from_path must not fail (which would drop
    // the file from listings/search) when birth time is unavailable. It falls back
    // to the modified time. On platforms that do support btime this still succeeds.
    #[test]
    fn test_created_falls_back_to_modified() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let file_path = dir.path().join("note.md");
        std::fs::write(&file_path, b"# hello").expect("write temp file");

        let (size, created, modified) =
            file_details_from_path(&file_path).expect("file details must not fail");

        assert_eq!(size, 7);
        // Both timestamps must be populated; created is either the real btime or,
        // when unavailable, the modified time (never causing an error/drop).
        assert!(modified > 0, "modified time should be populated");
        assert!(
            created > 0,
            "created time should be populated (btime or fallback)"
        );
    }

    #[test]
    fn test_is_markdown_extension_true() {
        let extensions = vec!["md".to_string(), "markdown".to_string()];
        assert!(is_markdown_extension("md", &extensions));
        assert!(is_markdown_extension("markdown", &extensions));
    }

    #[test]
    fn test_is_markdown_extension_false() {
        let extensions = vec!["md".to_string()];
        assert!(!is_markdown_extension("txt", &extensions));
        assert!(!is_markdown_extension("html", &extensions));
    }

    #[test]
    fn test_parse_tag_values_basic() {
        let tags: Vec<String> = parse_tag_values("rust, programming, web dev").collect();
        assert_eq!(tags, vec!["rust", "programming", "web dev"]);
    }

    #[test]
    fn test_parse_tag_values_single() {
        let tags: Vec<String> = parse_tag_values("rust").collect();
        assert_eq!(tags, vec!["rust"]);
    }

    #[test]
    fn test_parse_tag_values_whitespace() {
        let tags: Vec<String> = parse_tag_values("  rust  ,  python  ").collect();
        assert_eq!(tags, vec!["rust", "python"]);
    }

    #[test]
    fn test_parse_tag_values_empty() {
        let tags: Vec<String> = parse_tag_values("").collect();
        assert!(tags.is_empty());
    }

    #[test]
    fn test_parse_tag_values_empty_between() {
        let tags: Vec<String> = parse_tag_values("rust,,python").collect();
        assert_eq!(tags, vec!["rust", "python"]);
    }

    #[test]
    fn test_get_page_title_from_frontmatter() {
        let mut frontmatter = crate::markdown::SimpleMetadata::new();
        frontmatter.insert(
            "title".to_string(),
            serde_json::Value::String("My Page Title".to_string()),
        );
        let path = Path::new("/docs/readme.md");
        assert_eq!(get_page_title(&frontmatter, path), "My Page Title");
    }

    #[test]
    fn test_get_page_title_from_filename() {
        let frontmatter = crate::markdown::SimpleMetadata::new();
        let path = Path::new("/docs/rust-guide.md");
        assert_eq!(get_page_title(&frontmatter, path), "rust-guide");
    }

    #[test]
    fn test_get_page_title_fallback() {
        let frontmatter = crate::markdown::SimpleMetadata::new();
        let path = Path::new("/");
        assert_eq!(get_page_title(&frontmatter, path), "Untitled");
    }

    #[test]
    fn test_extract_tag_values_from_array() {
        let val = serde_json::json!(["rust", "python"]);
        let tags = extract_tag_values(&val);
        assert_eq!(tags, vec!["rust", "python"]);
    }

    #[test]
    fn test_extract_tag_values_from_comma_string() {
        let val = serde_json::json!("rust, python");
        let tags = extract_tag_values(&val);
        assert_eq!(tags, vec!["rust", "python"]);
    }

    #[test]
    fn test_extract_tag_values_from_single_string() {
        let val = serde_json::json!("rust");
        let tags = extract_tag_values(&val);
        assert_eq!(tags, vec!["rust"]);
    }

    #[test]
    fn test_extract_tag_values_from_number() {
        let val = serde_json::json!(42);
        let tags = extract_tag_values(&val);
        assert!(tags.is_empty());
    }

    // ==================== Root containment ====================

    /// Builds a `Repo` rooted at `root` with the defaults these tests need.
    fn test_repo(root: &Path) -> Repo {
        Repo::init(
            root.to_path_buf(),
            "static".to_string(),
            &["md".to_string()],
            &[],
            &[],
            "index.md".to_string(),
            &[],
            &[],
        )
    }

    #[test]
    fn test_repo_relative_within_root_accepts_contained_paths() {
        let root = Path::new("/repo");

        assert_eq!(
            repo_relative_within_root(root, &root.join("docs").join("guide.md")),
            Some(PathBuf::from("docs").join("guide.md"))
        );
        // The root itself relativizes to the empty path, which is contained.
        assert_eq!(repo_relative_within_root(root, root), Some(PathBuf::new()));
    }

    #[test]
    fn test_repo_relative_within_root_rejects_escapes() {
        let root = Path::new("/repo");

        // A sibling directory: relativizes to `../elsewhere/secret.md`.
        assert_eq!(
            repo_relative_within_root(root, Path::new("/elsewhere/secret.md")),
            None
        );
        // An unresolved upward traversal.
        assert_eq!(
            repo_relative_within_root(root, &root.join("..").join("secret.md")),
            None
        );
        // Mixed absolute/relative inputs: `diff_paths` hands back something
        // absolute, which must not be mistaken for a repo-relative path.
        assert_eq!(
            repo_relative_within_root(Path::new("repo"), Path::new("/repo/a.md")),
            None
        );
    }

    /// A directory symlinked out of the repository must contribute nothing.
    ///
    /// `scan_folder` canonicalizes each queued directory and `WalkDir` follows
    /// links, so the walk used to be re-rooted at the symlink target: every file
    /// underneath relativized to `../…`, `path_to_url` preserved that, and the
    /// static builder joined it onto `--output` — writing pages outside the
    /// requested output directory.
    #[cfg(unix)]
    #[test]
    fn test_scan_folder_skips_directory_symlinked_outside_root() {
        let outside = tempfile::tempdir().expect("temp dir");
        let outside_docs = outside.path().join("docs");
        std::fs::create_dir_all(&outside_docs).expect("create outside dir");
        std::fs::write(outside_docs.join("secret.md"), "# Secret").expect("write secret");
        std::fs::write(outside_docs.join("secret.png"), b"not really a png").expect("write asset");

        let root = tempfile::tempdir().expect("temp dir");
        std::fs::write(root.path().join("index.md"), "# Home").expect("write index");
        std::os::unix::fs::symlink(&outside_docs, root.path().join("work")).expect("symlink");

        let repo = test_repo(root.path());
        repo.scan_all().expect("scan must succeed");

        let markdown = repo.markdown_files.pin();
        for (_, info) in markdown.iter() {
            assert!(
                !info.url_path.contains(".."),
                "markdown url_path escaped the root: {}",
                info.url_path
            );
            assert!(
                !info
                    .raw_path
                    .components()
                    .any(|c| matches!(c, Component::ParentDir)),
                "markdown raw_path escaped the root: {}",
                info.raw_path.display()
            );
        }
        assert!(
            markdown.iter().all(|(_, i)| !i.url_path.contains("secret")),
            "a file outside the root must not be indexed at all"
        );
        assert!(
            markdown.iter().any(|(_, i)| i.url_path == "/"),
            "the in-root index.md must still be scanned"
        );

        let other = repo.other_files.pin();
        for (_, info) in other.iter() {
            assert!(
                !info.url_path.contains(".."),
                "static url_path escaped the root: {}",
                info.url_path
            );
        }
        assert!(
            other.iter().all(|(_, i)| !i.url_path.contains("secret")),
            "a static file outside the root must not be indexed at all"
        );
    }

    // ==================== External static overlay ====================

    /// Builds `<tmp>/project/{content,static}` and a `Repo` rooted at `content`
    /// with `static_folder = "../static"` — the `repo/content` + `repo/static`
    /// layout. Returns the temp dir (kept alive), the project dir and the repo.
    fn peer_overlay_repo() -> (tempfile::TempDir, PathBuf, Repo) {
        let tmp = tempfile::tempdir().expect("temp dir");
        let project = tmp.path().to_path_buf();
        let content = project.join("content");
        std::fs::create_dir_all(content.join(".mbr")).expect("create content");
        std::fs::write(content.join("README.md"), "# Home").expect("write readme");
        std::fs::create_dir_all(project.join("static/videos")).expect("create static");
        std::fs::write(project.join("static/pic.png"), b"PNG bytes").expect("write pic");
        std::fs::write(project.join("static/videos/demo.mp4"), b"MP4 bytes").expect("write video");

        let repo = Repo::init(
            content,
            "../static".to_string(),
            &["md".to_string()],
            &[],
            &[],
            "index.md".to_string(),
            &[],
            &[],
        );
        (tmp, project, repo)
    }

    /// The regression: a peer static folder serves over HTTP but used to index
    /// nothing, so its assets were invisible to `media.json`, the media browser,
    /// the editor's media picker, search, and the media-metadata pass.
    #[test]
    fn test_scan_indexes_assets_in_external_static_overlay() {
        let (_tmp, _project, repo) = peer_overlay_repo();

        repo.scan_all().expect("scan must succeed");
        repo.scan_static_folder().expect("static scan must succeed");

        let other = repo.other_files.pin();
        let mut urls: Vec<_> = other.iter().map(|(_, i)| i.url_path.clone()).collect();
        urls.sort();
        assert_eq!(
            urls,
            vec!["/pic.png".to_string(), "/videos/demo.mp4".to_string()],
            "a peer static folder's assets must be indexed, with the overlay prefix stripped"
        );
    }

    /// Markdown inside an external overlay has no representable URL — it would
    /// relativize to `../static/…`, which `path_to_url` preserves and the static
    /// builder joins onto `--output`. It is skipped rather than indexed with a
    /// broken URL, so no `..` can reach `site.json`.
    #[test]
    fn test_scan_skips_markdown_in_external_static_overlay() {
        let (_tmp, project, repo) = peer_overlay_repo();
        std::fs::write(project.join("static/stray.md"), "# Stray").expect("write stray");

        repo.scan_all().expect("scan must succeed");
        repo.scan_static_folder().expect("static scan must succeed");

        let markdown = repo.markdown_files.pin();
        assert!(
            markdown.iter().all(|(_, i)| !i.url_path.contains("stray")),
            "markdown in an external static overlay must not be indexed"
        );
        for (_, info) in markdown.iter() {
            assert!(
                !info.url_path.contains(".."),
                "markdown url_path escaped the root: {}",
                info.url_path
            );
            assert!(
                !info
                    .raw_path
                    .components()
                    .any(|c| matches!(c, Component::ParentDir)),
                "markdown raw_path escaped the root: {}",
                info.raw_path.display()
            );
        }
        assert!(
            markdown.iter().any(|(_, i)| i.url_path == "/README/"),
            "the in-root markdown must still be indexed"
        );

        // The assets alongside it are still indexed, with clean URLs.
        let other = repo.other_files.pin();
        for (_, info) in other.iter() {
            assert!(
                !info.url_path.contains(".."),
                "static url_path escaped: {}",
                info.url_path
            );
        }
    }

    /// The overlay exception must stay an exception. A directory symlinked out
    /// of the root is still refused even when an external overlay is configured,
    /// so the "build writes pages outside --output" fix cannot regress through
    /// the new second scan root.
    #[cfg(unix)]
    #[test]
    fn test_external_overlay_does_not_admit_other_escaping_directories() {
        let (_tmp, project, repo) = peer_overlay_repo();

        let elsewhere = tempfile::tempdir().expect("temp dir");
        std::fs::write(elsewhere.path().join("secret.md"), "# Secret").expect("write secret");
        std::fs::write(elsewhere.path().join("secret.png"), b"secret").expect("write asset");
        // One escape from inside the root, one from inside the overlay itself.
        std::os::unix::fs::symlink(elsewhere.path(), project.join("content/work"))
            .expect("symlink into root");
        std::os::unix::fs::symlink(elsewhere.path(), project.join("static/leak"))
            .expect("symlink into overlay");

        repo.scan_all().expect("scan must succeed");
        repo.scan_static_folder().expect("static scan must succeed");

        let markdown = repo.markdown_files.pin();
        assert!(
            markdown.iter().all(|(_, i)| !i.url_path.contains("secret")),
            "a directory symlinked out of the repo must contribute no markdown"
        );
        let other = repo.other_files.pin();
        assert!(
            other.iter().all(|(_, i)| !i.url_path.contains("secret")),
            "a directory symlinked out of the repo (or out of the overlay) must contribute no assets"
        );
        for (_, info) in other.iter() {
            assert!(
                !info.url_path.contains(".."),
                "static url_path escaped: {}",
                info.url_path
            );
        }
    }

    /// A `static_folder` the config policy would refuse must not become a scan
    /// root either — the scanner and the validator share one decision.
    #[test]
    fn test_refused_static_folder_is_not_scannable() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let content = tmp.path().join("project/content");
        std::fs::create_dir_all(&content).expect("create content");

        // Two shapes the policy refuses for different reasons: an ancestor of
        // the markdown root, and a target past the ascent budget. Both must
        // leave the scanner with nothing extra to walk.
        for value in ["../..", "../../../elsewhere"] {
            let repo = Repo::init(
                content.clone(),
                value.to_string(),
                &["md".to_string()],
                &[],
                &[],
                "index.md".to_string(),
                &[],
                &[],
            );

            assert_eq!(
                repo.canonical_static_root, None,
                "a static_folder the validator refuses must not be a scan root: {value:?}"
            );
        }
    }

    /// Builds `<tmp>/project/{src/routes,static}` and a `Repo` rooted at
    /// `src/routes` — the SvelteKit shape, where the overlay is two levels up.
    fn two_deep_overlay_repo(static_folder: &str) -> (tempfile::TempDir, PathBuf, Repo) {
        let tmp = tempfile::tempdir().expect("temp dir");
        let project = tmp.path().join("project");
        let routes = project.join("src/routes");
        std::fs::create_dir_all(routes.join(".mbr")).expect("create routes");
        std::fs::write(routes.join("README.md"), "# Home").expect("write readme");
        std::fs::create_dir_all(project.join("static/videos")).expect("create static");
        std::fs::write(project.join("static/pic.png"), b"PNG bytes").expect("write pic");
        std::fs::write(project.join("static/videos/demo.mp4"), b"MP4 bytes").expect("write video");

        let repo = Repo::init(
            routes,
            static_folder.to_string(),
            &["md".to_string()],
            &[],
            &[],
            "index.md".to_string(),
            &[],
            &[],
        );
        (tmp, project, repo)
    }

    /// A two-level overlay must index exactly like a peer one: the whole
    /// overlay prefix stripped, no `..` left in any URL. This is what catches a
    /// `strip_prefix` regression on a two-component prefix, which a one-level
    /// fixture cannot distinguish from a working implementation.
    #[test]
    fn test_two_deep_overlay_is_scannable_with_clean_urls() {
        let (_tmp, project, repo) = two_deep_overlay_repo("../../static");

        assert_eq!(
            repo.canonical_static_root,
            Some(project.join("static").canonicalize().expect("canonicalize")),
            "a two-level overlay the validator accepts must be a scan root"
        );

        repo.scan_all().expect("scan must succeed");
        repo.scan_static_folder().expect("static scan must succeed");

        let other = repo.other_files.pin();
        let mut urls: Vec<_> = other.iter().map(|(_, i)| i.url_path.clone()).collect();
        urls.sort();
        assert_eq!(
            urls,
            vec!["/pic.png".to_string(), "/videos/demo.mp4".to_string()],
            "a two-level static folder's assets must be indexed with the overlay prefix stripped"
        );
    }

    /// `../../static` and `./../../static` name the same directory, and the
    /// config policy accepts both — so indexing must not depend on which
    /// spelling was written. Stripping the raw configured string does depend on
    /// it, because `Path::strip_prefix` compares components and keeps a leading
    /// `.`, which used to leave a `..` in the URL.
    #[test]
    fn test_overlay_urls_survive_a_noncanonical_static_folder_spelling() {
        let (_tmp, _project, repo) = two_deep_overlay_repo("./../../static");

        repo.scan_all().expect("scan must succeed");
        repo.scan_static_folder().expect("static scan must succeed");

        let other = repo.other_files.pin();
        let mut urls: Vec<_> = other.iter().map(|(_, i)| i.url_path.clone()).collect();
        urls.sort();
        assert_eq!(
            urls,
            vec!["/pic.png".to_string(), "/videos/demo.mp4".to_string()],
            "URLs must come from the resolved overlay, not the configured spelling"
        );
    }

    // ==================== Media metadata flags ====================

    /// Claiming the run-once guard must not announce completion.
    ///
    /// `media_populated` is what `is_media_populated()`/`wait_for_media()` and
    /// the `media.json` handler read as "the metadata is there". While it
    /// doubled as the "already started" guard, a request arriving during the
    /// probing window skipped the wait and got entries with no duration or
    /// dimensions.
    #[test]
    fn test_in_flight_media_population_does_not_report_as_complete() {
        let dir = tempfile::tempdir().expect("temp dir");
        let repo = test_repo(dir.path());

        // Exactly the state a reader sees mid-population: guard claimed, work
        // not finished.
        repo.media_population_started.store(true, Ordering::SeqCst);

        assert!(
            !repo.is_media_populated(),
            "population in flight must not be reported as complete"
        );
    }

    #[test]
    fn test_media_populated_is_published_after_population_and_reset_by_clear() {
        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::write(dir.path().join("index.md"), "# Home").expect("write index");
        std::fs::write(dir.path().join("paper.pdf"), b"not really a pdf").expect("write paper");

        let repo = test_repo(dir.path());
        repo.scan_all().expect("scan must succeed");

        assert!(!repo.is_media_populated());
        assert!(!repo.media_population_started.load(Ordering::SeqCst));

        repo.populate_media_metadata();
        assert!(repo.media_population_started.load(Ordering::SeqCst));
        assert!(
            repo.is_media_populated(),
            "flag must be published once population finished"
        );

        // Run-once guard: a repeat call is a no-op and must not unset "finished".
        repo.populate_media_metadata();
        assert!(repo.is_media_populated());

        // `clear()` must reset both flags, otherwise the full-rescan path would
        // skip population and leave every later `wait_for_media()` blocked.
        repo.clear();
        assert!(!repo.is_media_populated());
        assert!(!repo.media_population_started.load(Ordering::SeqCst));

        repo.populate_media_metadata();
        assert!(
            repo.is_media_populated(),
            "a rescan must be able to re-populate media metadata"
        );
    }

    // ==================== Scan/media waiters ====================

    /// `Notify::notify_waiters()` stores no permit, so a waiter created after the
    /// single completion notification has already fired must still return.
    #[tokio::test]
    async fn test_wait_for_scan_returns_immediately_when_already_complete() {
        let dir = tempfile::tempdir().expect("temp dir");
        let repo = test_repo(dir.path());
        repo.mark_scan_complete();

        tokio::time::timeout(std::time::Duration::from_secs(5), repo.wait_for_scan())
            .await
            .expect("a waiter started after completion must not block");
    }

    #[tokio::test]
    async fn test_wait_for_scan_is_woken_by_later_completion() {
        let dir = tempfile::tempdir().expect("temp dir");
        let repo = Arc::new(test_repo(dir.path()));

        let waiting = Arc::clone(&repo);
        let waiter = tokio::spawn(async move { waiting.wait_for_scan().await });
        // Let the waiter register its interest before completion fires.
        tokio::task::yield_now().await;

        repo.mark_scan_complete();

        tokio::time::timeout(std::time::Duration::from_secs(5), waiter)
            .await
            .expect("waiter must be woken by mark_scan_complete")
            .expect("waiter task panicked");
    }

    #[tokio::test]
    async fn test_wait_for_media_returns_immediately_when_already_populated() {
        let dir = tempfile::tempdir().expect("temp dir");
        let repo = test_repo(dir.path());
        repo.populate_media_metadata();
        repo.notify_media_populated();

        tokio::time::timeout(std::time::Duration::from_secs(5), repo.wait_for_media())
            .await
            .expect("a waiter started after population must not block");
    }

    /// A notification that arrives with the flag still unset is spurious: the
    /// waiter must go back to waiting rather than report populated metadata.
    #[tokio::test]
    async fn test_wait_for_media_ignores_spurious_notify_then_wakes() {
        let dir = tempfile::tempdir().expect("temp dir");
        let repo = Arc::new(test_repo(dir.path()));

        let waiting = Arc::clone(&repo);
        let mut waiter = tokio::spawn(async move { waiting.wait_for_media().await });
        tokio::task::yield_now().await;

        // Notification without population: must not release the waiter.
        repo.notify_media_populated();
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(100), &mut waiter)
                .await
                .is_err(),
            "a notification with the flag unset must not release the waiter"
        );

        repo.populate_media_metadata();
        repo.notify_media_populated();

        tokio::time::timeout(std::time::Duration::from_secs(5), waiter)
            .await
            .expect("waiter must be woken once media metadata is populated")
            .expect("waiter task panicked");
    }

    /// A rescan whose scan step fails must still release `wait_for_media()`.
    ///
    /// `full_rescan()` calls `clear()` first, which resets `media_populated`.
    /// When the scan error caused an early return, `notify_media_populated()`
    /// never ran and `/.mbr/media.json` blocked for the life of the process —
    /// one unreadable repository root wedged the endpoint permanently.
    #[tokio::test]
    async fn test_full_rescan_releases_media_waiters_even_when_scan_fails() {
        let dir = tempfile::tempdir().expect("temp dir");
        let repo = Arc::new(test_repo(dir.path()));

        // Populate once so the flag starts set, as it would on a live server.
        repo.populate_media_metadata();
        repo.notify_media_populated();
        assert!(repo.is_media_populated());

        // Delete the root out from under the repo so `scan_all()` errors. The
        // canonical root was resolved at construction, so it still points here.
        dir.close().expect("remove repo root");

        let rescanning = Arc::clone(&repo);
        let rescan = tokio::task::spawn_blocking(move || rescanning.full_rescan());

        tokio::time::timeout(std::time::Duration::from_secs(5), rescan)
            .await
            .expect("rescan must not hang")
            .expect("rescan task panicked");

        tokio::time::timeout(std::time::Duration::from_secs(5), repo.wait_for_media())
            .await
            .expect("a failed rescan must still release media waiters");
    }

    // ==================== Deterministic serialization ====================

    /// Two builds of an unchanged repository must serialize to identical bytes.
    ///
    /// `papaya::HashMap` seeds its `RandomState` per instance, so iteration
    /// order differs between processes: `site.json` churned in `git diff` and
    /// content-hash/ETag caches invalidated on no-op rebuilds. Two
    /// independently built maps stand in for two builds.
    #[test]
    fn test_markdown_files_serialize_deterministically_in_url_path_order() {
        let build_files = || {
            let files = MarkdownFiles(HashMap::new());
            let pin = files.pin();
            for i in 0..64 {
                pin.insert(
                    PathBuf::from(format!("/repo/notes/note{i:02}.md")),
                    MarkdownInfo {
                        raw_path: PathBuf::from(format!("notes/note{i:02}.md")),
                        url_path: format!("/notes/note{i:02}/"),
                        created: 1,
                        modified: 2,
                        frontmatter: None,
                        relationships: Vec::new(),
                    },
                );
            }
            drop(pin);
            files
        };

        let first = serde_json::to_string(&build_files()).expect("serialize");
        let second = serde_json::to_string(&build_files()).expect("serialize");
        assert_eq!(
            first, second,
            "serialized bytes must not depend on map iteration order"
        );

        assert_eq!(
            url_paths_of(&first),
            sorted(url_paths_of(&first)),
            "entries must be emitted in url_path order"
        );
    }

    #[test]
    fn test_other_files_serialize_deterministically_in_url_path_order() {
        let build_files = || {
            let files = OtherFiles(HashMap::new());
            let pin = files.pin();
            for i in 0..64 {
                let raw_path = PathBuf::from(format!("/repo/images/img{i:02}.png"));
                pin.insert(
                    raw_path.clone(),
                    OtherFileInfo {
                        url_path: format!("/images/img{i:02}.png"),
                        metadata: StaticFileMetadata::empty(&raw_path),
                        raw_path,
                        extracted_text: None,
                    },
                );
            }
            drop(pin);
            files
        };

        let first = serde_json::to_string(&build_files()).expect("serialize");
        let second = serde_json::to_string(&build_files()).expect("serialize");
        assert_eq!(
            first, second,
            "serialized bytes must not depend on map iteration order"
        );

        assert_eq!(
            url_paths_of(&first),
            sorted(url_paths_of(&first)),
            "entries must be emitted in url_path order"
        );
    }

    /// Extracts the `url_path` of every entry in a serialized file sequence.
    fn url_paths_of(json: &str) -> Vec<String> {
        serde_json::from_str::<Vec<serde_json::Value>>(json)
            .expect("serialized files must be a JSON array")
            .iter()
            .map(|entry| {
                entry["url_path"]
                    .as_str()
                    .expect("every entry has a url_path")
                    .to_string()
            })
            .collect()
    }

    fn sorted(mut values: Vec<String>) -> Vec<String> {
        values.sort();
        values
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    // Strategy for valid file/directory names (no path separators or special chars)
    fn valid_name_strategy() -> impl Strategy<Value = String> {
        "[a-zA-Z0-9_-]{1,20}"
    }

    // Strategy for valid extensions
    fn extension_strategy() -> impl Strategy<Value = String> {
        "[a-z]{1,10}"
    }

    proptest! {
        /// should_ignore is deterministic - same input always gives same output
        #[test]
        fn prop_should_ignore_deterministic(
            name in valid_name_strategy(),
            ignore_dirs in proptest::collection::vec(valid_name_strategy(), 0..3),
            ignore_globs in proptest::collection::vec("[*][.][a-z]{1,5}", 0..3),
        ) {
            let path = Path::new(&name);
            let result1 = should_ignore(path, &ignore_dirs, &ignore_globs);
            let result2 = should_ignore(path, &ignore_dirs, &ignore_globs);
            prop_assert_eq!(result1, result2);
        }

        /// Hidden files (starting with .) are always ignored
        #[test]
        fn prop_hidden_files_always_ignored(name in "[.][a-zA-Z0-9]{1,15}") {
            let path = Path::new(&name);
            prop_assert!(should_ignore(path, &[], &[]));
        }

        /// Non-hidden files without matching globs are not ignored
        #[test]
        fn prop_normal_files_not_ignored(name in "[a-zA-Z][a-zA-Z0-9]{0,15}") {
            let path = Path::new(&name);
            // No ignore patterns configured
            prop_assert!(!should_ignore(path, &[], &[]));
        }

        /// is_markdown_extension is deterministic
        #[test]
        fn prop_is_markdown_extension_deterministic(
            ext in extension_strategy(),
            extensions in proptest::collection::vec(extension_strategy(), 1..5)
        ) {
            let result1 = is_markdown_extension(&ext, &extensions);
            let result2 = is_markdown_extension(&ext, &extensions);
            prop_assert_eq!(result1, result2);
        }

        /// Extension in list returns true
        #[test]
        fn prop_extension_in_list_returns_true(
            extensions in proptest::collection::vec(extension_strategy(), 1..5)
        ) {
            // Pick the first extension from the list
            if let Some(ext) = extensions.first() {
                prop_assert!(is_markdown_extension(ext, &extensions));
            }
        }

        /// build_markdown_url_path always returns path starting with /
        #[test]
        fn prop_markdown_url_starts_with_slash(
            subpath in proptest::collection::vec(valid_name_strategy(), 1..4),
            filename in valid_name_strategy(),
        ) {
            let root = PathBuf::from("/root");
            let mut full_path = root.clone();
            for component in &subpath {
                full_path.push(component);
            }
            full_path.push(format!("{}.md", filename));

            let url = build_markdown_url_path(&full_path, &root, "index.md");
            prop_assert!(url.starts_with('/'), "URL should start with /: {}", url);
            prop_assert!(!url.contains('\\'), "URL must not contain a backslash: {}", url);
        }

        /// build_markdown_url_path always returns path ending with /
        #[test]
        fn prop_markdown_url_ends_with_slash(
            subpath in proptest::collection::vec(valid_name_strategy(), 0..4),
            filename in valid_name_strategy(),
        ) {
            let root = PathBuf::from("/root");
            let mut full_path = root.clone();
            for component in &subpath {
                full_path.push(component);
            }
            full_path.push(format!("{}.md", filename));

            let url = build_markdown_url_path(&full_path, &root, "index.md");
            prop_assert!(url.ends_with('/'), "URL should end with /: {}", url);
            prop_assert!(!url.contains('\\'), "URL must not contain a backslash: {}", url);
        }

        /// build_static_url_path always returns path starting with /
        #[test]
        fn prop_static_url_starts_with_slash(
            subpath in proptest::collection::vec(valid_name_strategy(), 0..4),
            filename in valid_name_strategy(),
            ext in extension_strategy(),
        ) {
            let root = PathBuf::from("/root");
            let mut full_path = root.clone();
            for component in &subpath {
                full_path.push(component);
            }
            full_path.push(format!("{}.{}", filename, ext));

            let url = build_static_url_path(&full_path, &root, "static");
            prop_assert!(url.starts_with('/'), "URL should start with /: {}", url);
            prop_assert!(!url.contains('\\'), "URL must not contain a backslash: {}", url);
        }

        /// URL paths don't contain double slashes
        #[test]
        fn prop_no_double_slashes_in_markdown_url(
            subpath in proptest::collection::vec(valid_name_strategy(), 0..4),
            filename in valid_name_strategy(),
        ) {
            let root = PathBuf::from("/root");
            let mut full_path = root.clone();
            for component in &subpath {
                full_path.push(component);
            }
            full_path.push(format!("{}.md", filename));

            let url = build_markdown_url_path(&full_path, &root, "index.md");
            prop_assert!(!url.contains("//"), "URL should not contain //: {}", url);
            prop_assert!(!url.contains('\\'), "URL must not contain a backslash: {}", url);
        }
    }
}
