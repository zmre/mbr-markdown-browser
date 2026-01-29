use std::{
    ops::Deref,
    path::{Path, PathBuf},
    sync::Arc,
    time::UNIX_EPOCH,
};

use papaya::{HashMap, HashSet};
use rayon::prelude::*;
use serde::{Serialize, Serializer, ser::SerializeSeq};
use walkdir::WalkDir;

use crate::Config;
use crate::config::TagSource;
use crate::tag_index::{TagIndex, TaggedPage};

#[derive(Clone, Serialize)]
pub struct Repo {
    #[serde(skip)]
    root_dir: PathBuf,
    #[serde(skip)]
    static_folder: String,
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
    /// Configured tag sources for frontmatter extraction.
    #[serde(skip)]
    tag_sources: Vec<TagSource>,
}

#[derive(Clone)]
pub struct MarkdownFiles(HashMap<PathBuf, MarkdownInfo>);
impl Deref for MarkdownFiles {
    type Target = HashMap<PathBuf, MarkdownInfo>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Serialize for MarkdownFiles {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut s = serializer.serialize_seq(Some(self.len()))?;
        for (_, v) in self.pin().iter() {
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
impl Serialize for OtherFiles {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut s = serializer.serialize_seq(Some(self.len()))?;
        for (_, v) in self.pin().iter() {
            s.serialize_element(v)?;
        }
        s.end()
    }
}

#[derive(Clone, Serialize)]
pub struct MarkdownInfo {
    pub raw_path: PathBuf,
    pub url_path: String,
    pub created: u64,
    pub modified: u64,
    pub frontmatter: Option<crate::markdown::SimpleMetadata>,
}

#[derive(Clone, Serialize)]
pub struct OtherFileInfo {
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
    path: PathBuf,
    created: Option<u64>,
    modified: Option<u64>,
    file_size_bytes: Option<u64>,
    kind: StaticFileKind,
}

#[derive(Clone, Default, Serialize)]
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
                },
                ..Default::default()
            },
            Some("txt") | Some("css") | Some("vtt") | Some("toml") | Some("json") | Some("js")
            | Some("ts") => Self {
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

    pub fn populate(self) -> Self {
        let mut me = self;
        let (filesize, created, modified) = match file_details_from_path(&me.path).ok() {
            Some((fs, c, m)) => (Some(fs), Some(c), Some(m)),
            _ => (None, None, None),
        };
        me.file_size_bytes = filesize;
        me.created = created;
        me.modified = modified;
        // Extract media metadata when available (requires ffmpeg)
        #[cfg(feature = "media-metadata")]
        {
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
                    let metadata =
                        metadata::media_file::MediaFileMetadata::new(&me.path.as_path()).ok();

                    StaticFileKind::Image {
                        width: metadata.as_ref().and_then(|m| m.width),
                        height: metadata.as_ref().and_then(|m| m.height),
                    }
                }
                StaticFileKind::Audio { .. } => {
                    let metadata =
                        metadata::media_file::MediaFileMetadata::new(&me.path.as_path()).ok();
                    StaticFileKind::Audio {
                        duration: metadata.as_ref().and_then(|m| m.duration.clone()),
                        title: metadata.as_ref().and_then(|m| m.title.clone()),
                    }
                }
                StaticFileKind::Video { .. } => {
                    let metadata =
                        metadata::media_file::MediaFileMetadata::new(&me.path.as_path()).ok();

                    StaticFileKind::Video {
                        width: metadata.as_ref().and_then(|m| m.width),
                        height: metadata.as_ref().and_then(|m| m.height),
                        duration: metadata.as_ref().and_then(|m| m.duration.clone()),
                        title: metadata.as_ref().and_then(|m| m.title.clone()),
                    }
                }
                _ => me.kind,
            };
        }
        me
    }

    pub fn from<P: Into<std::path::PathBuf>>(file: P) -> Self {
        let empty = Self::empty(file);
        empty.populate()
    }
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
        )
    }

    pub fn init<S: Into<String>, P: Into<std::path::PathBuf>>(
        root_dir: P,
        static_folder: S,
        markdown_extensions: &[String],
        ignore_dirs: &[String],
        ignore_globs: &[String],
        index_file: S,
        tag_sources: &[TagSource],
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

        Self {
            root_dir: root_dir.into(),
            static_folder: static_folder.into(),
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
            tag_sources: tag_sources.to_vec(),
        }
    }

    pub fn scan_folder<P: AsRef<Path>>(
        &self,
        relative_folder_path: &P,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let relative_folder_path_ref = relative_folder_path.as_ref();
        let start_folder = self
            .root_dir
            .join(relative_folder_path_ref)
            .canonicalize()?;

        // Skip if already scanned
        if self.scanned_folders.pin().contains(&start_folder) {
            return Ok(());
        }
        tracing::debug!("Scanning folder: {:?}", relative_folder_path_ref);
        self.scanned_folders.pin().insert(start_folder.clone());

        // Walk directory with filtering (using pre-compiled patterns for efficiency)
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
                // Queue subdirectory for later scanning
                let relative_entry =
                    pathdiff::diff_paths(path, &self.root_dir).unwrap_or(path.to_path_buf());
                self.queued_folders
                    .pin()
                    .insert(path.to_path_buf(), relative_entry);
            } else if is_markdown_extension(extension, &self.markdown_extensions) {
                // Process markdown file
                if let Ok((_filesize, created, modified)) = file_details_from_path(path) {
                    let url = build_markdown_url_path(path, &self.root_dir, &self.index_file);
                    let mdfile = MarkdownInfo {
                        raw_path: path.to_path_buf(),
                        url_path: url,
                        created,
                        modified,
                        frontmatter: None,
                    };
                    markdown.insert(path.to_path_buf(), mdfile);
                } else {
                    tracing::warn!("Couldn't process markdown file at {:?}", path);
                }
            } else {
                // Process static file
                let url = build_static_url_path(path, &self.root_dir, &self.static_folder);
                let other_file = OtherFileInfo {
                    raw_path: path.to_path_buf(),
                    url_path: url,
                    metadata: StaticFileMetadata::empty(path),
                    extracted_text: None,
                };
                other.insert(path.to_path_buf(), other_file);
            }
        }

        // Parallel processing: extract frontmatter from markdown files and build tag index
        markdown
            .into_par_iter()
            .for_each(|(mdfile, mddetails): (PathBuf, MarkdownInfo)| {
                let metadata = crate::markdown::extract_metadata_from_file(&mdfile).ok();
                let details = if let Some(ref frontmatter) = metadata {
                    // Extract tags from frontmatter for each configured tag source
                    let title = get_page_title(frontmatter, &mddetails.raw_path);
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
                        frontmatter: metadata,
                        ..mddetails
                    }
                } else {
                    mddetails
                };
                self.markdown_files.pin().insert(mdfile, details);
            });

        // Parallel processing: populate static file metadata and extract text from searchable files
        other.into_par_iter().for_each(|(file, other_file)| {
            let mut other_file = OtherFileInfo {
                metadata: other_file.metadata.populate(),
                ..other_file
            };
            // Extract text from PDFs and text files for search indexing
            if other_file.is_searchable() {
                other_file.extracted_text = other_file.extract_text();
            }
            self.other_files.pin().insert(file, other_file);
        });

        Ok(())
    }

    pub fn scan_all(&self) -> Result<(), Box<dyn std::error::Error>> {
        self.scan_folder(&PathBuf::from("."))?; // the . is relative to the root_dir, so this scans the root dir

        // Only scan static folder if it exists
        let static_path = self.root_dir.join(&self.static_folder);
        if static_path.is_dir() {
            self.scan_folder(&PathBuf::from(&self.static_folder))?;
        }

        while !self.queued_folders.is_empty() {
            // TODO: make sure this doesn't deadlock
            let vec_folders: Vec<_> = self
                .queued_folders
                .pin()
                .iter()
                .map(|(_, relative)| relative.clone())
                .collect();
            self.queued_folders.pin().clear();
            assert!(self.queued_folders.is_empty());
            tracing::debug!("Parallel batch: {:?}", &vec_folders);
            vec_folders.into_par_iter().for_each(|rel_path| {
                self.scan_folder(&rel_path).unwrap_or_else(|e| {
                    tracing::error!("Failed to scan folder {:?}: {e}", &rel_path)
                }) // ignores errors
            });
        }
        Ok(())
    }

    pub fn to_json(&self) -> serde_json::Result<String> {
        serde_json::to_string(self)
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
    }
}

/// Returns file_size, created_secs, modified_secs
pub fn file_details_from_path<P: AsRef<Path>>(
    path: P,
) -> Result<(u64, u64, u64), Box<dyn std::error::Error>> {
    let path = path.as_ref();
    let metadata = std::fs::metadata(path)?;

    let file_size = metadata.len();

    // Modified time
    let modified = metadata.modified()?;
    let modified_secs = modified.duration_since(UNIX_EPOCH)?.as_secs();

    // Created time (might not be supported on all platforms)
    let created = metadata.created()?;
    let created_secs = created.duration_since(UNIX_EPOCH)?.as_secs();

    Ok((file_size, created_secs, modified_secs))
}

// ============================================================================
// Pure helper functions for repo scanning (extracted for testability)
// ============================================================================

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
    let mut url = pathdiff::diff_paths(path, root_dir)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();

    // Ensure leading slash
    if !url.starts_with('/') {
        url = "/".to_string() + &url;
    }

    // Remove index file from path
    if url.ends_with(index_file) {
        url = url.replace(index_file, "");
    }

    // Replace extension with trailing slash
    if let Some((base, extension)) = url.rsplit_once('.')
        && !extension.contains('/')
    {
        url = base.to_string() + "/";
    }

    url
}

/// Builds a URL path for a static file.
///
/// Converts a filesystem path relative to root into a URL path:
/// - Removes static folder prefix
/// - Ensures leading slash
pub fn build_static_url_path(path: &Path, root_dir: &Path, static_folder: &str) -> String {
    let mut url = pathdiff::diff_paths(path, root_dir)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default()
        .replace(static_folder, "");

    // Ensure leading slash
    if !url.starts_with('/') {
        url = "/".to_string() + &url;
    }

    url
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
        let mut frontmatter = std::collections::HashMap::new();
        frontmatter.insert(
            "title".to_string(),
            serde_json::Value::String("My Page Title".to_string()),
        );
        let path = Path::new("/docs/readme.md");
        assert_eq!(get_page_title(&frontmatter, path), "My Page Title");
    }

    #[test]
    fn test_get_page_title_from_filename() {
        let frontmatter = std::collections::HashMap::new();
        let path = Path::new("/docs/rust-guide.md");
        assert_eq!(get_page_title(&frontmatter, path), "rust-guide");
    }

    #[test]
    fn test_get_page_title_fallback() {
        let frontmatter = std::collections::HashMap::new();
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
        }
    }
}
