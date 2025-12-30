use std::{
    ops::Deref,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

use papaya::{HashMap, HashSet};
use rayon::prelude::*;
use serde::{
    ser::SerializeSeq,
    Serialize, Serializer,
};
use walkdir::WalkDir;

use crate::Config;

#[derive(Clone, Serialize)]
pub struct Repo {
    #[serde(skip)]
    root_dir: PathBuf,
    #[serde(skip)]
    static_folder: String,
    #[serde(skip)]
    markdown_extensions: Vec<String>,
    #[serde(skip)]
    index_file: String,
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
        if let Some(size) = self.metadata.file_size_bytes {
            if size > MAX_TEXT_EXTRACTION_SIZE {
                tracing::debug!(
                    "Skipping text extraction for {:?}: file too large ({} bytes)",
                    self.raw_path,
                    size
                );
                return None;
            }
        }

        match &self.metadata.kind {
            StaticFileKind::Pdf { .. } => self.extract_pdf_text(),
            StaticFileKind::Text => self.extract_plain_text(),
            _ => None,
        }
    }

    /// Extract text from a PDF file.
    /// Uses catch_unwind to handle panics from pdf-extract on malformed PDFs.
    fn extract_pdf_text(&self) -> Option<String> {
        let path = self.raw_path.clone();
        let result = std::panic::catch_unwind(|| pdf_extract::extract_text(&path));

        match result {
            Ok(Ok(text)) => {
                let text = text.trim().to_string();
                if text.is_empty() {
                    None
                } else {
                    Some(text)
                }
            }
            Ok(Err(e)) => {
                tracing::debug!("Failed to extract PDF text from {:?}: {}", self.raw_path, e);
                None
            }
            Err(_) => {
                tracing::warn!(
                    "PDF extraction panicked for {:?}, skipping text extraction",
                    self.raw_path
                );
                None
            }
        }
    }

    /// Extract text from a plain text file.
    fn extract_plain_text(&self) -> Option<String> {
        match std::fs::read_to_string(&self.raw_path) {
            Ok(text) => {
                let text = text.trim().to_string();
                if text.is_empty() {
                    None
                } else {
                    Some(text)
                }
            }
            Err(e) => {
                tracing::debug!(
                    "Failed to read text file {:?}: {}",
                    self.raw_path,
                    e
                );
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
        me.kind = match me.kind {
            /* StaticFileKind::Pdf {
                            ..me
                        }, // TODO: get PDF metadata using https://docs.rs/pdf-extract/latest/pdf_extract/ -- but see if there's a way to just process some of the file
            // */
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
        )
    }

    pub fn init<S: Into<String>, P: Into<std::path::PathBuf>>(
        root_dir: P,
        static_folder: S,
        markdown_extensions: &[String],
        ignore_dirs: &[String],
        ignore_globs: &[String],
        index_file: S,
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

        // Parallel processing: extract frontmatter from markdown files
        markdown
            .into_par_iter()
            .for_each(|(mdfile, mddetails): (PathBuf, MarkdownInfo)| {
                let metadata = crate::markdown::extract_metadata_from_file(&mdfile).ok();
                let details = if metadata.is_some() {
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
    let file_name = path
        .file_name()
        .and_then(|x| x.to_str())
        .unwrap_or("");

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
    let file_name = path
        .file_name()
        .and_then(|x| x.to_str())
        .unwrap_or("");

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
pub fn build_markdown_url_path(
    path: &Path,
    root_dir: &Path,
    index_file: &str,
) -> String {
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
pub fn build_static_url_path(
    path: &Path,
    root_dir: &Path,
    static_folder: &str,
) -> String {
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
        assert_eq!(build_markdown_url_path(path, root, "index.md"), "/docs/guide/");
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
        assert_eq!(build_static_url_path(path, root, "static"), "/assets/image.png");
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
