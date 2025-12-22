//! Search functionality for the mbr markdown browser.
//!
//! Provides two search modes:
//! - **Metadata search**: Fast fuzzy matching of titles, paths, and frontmatter
//!   using nucleo-matcher. Data is already in memory from repo scanning.
//! - **Content search**: Full-text search through markdown file contents
//!   using grep-searcher with SIMD acceleration.
//!
//! Both modes use rayon for parallel processing across files.

use std::sync::Arc;

use grep_regex::RegexMatcherBuilder;
use grep_searcher::sinks::UTF8;
use grep_searcher::{BinaryDetection, SearcherBuilder};
use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::errors::SearchError;
use crate::repo::{MarkdownInfo, Repo};

/// Maximum number of search results to return by default.
pub const DEFAULT_RESULT_LIMIT: usize = 50;

/// Maximum snippet length in characters.
const MAX_SNIPPET_LENGTH: usize = 200;

/// Context lines to show around content matches.
const CONTENT_CONTEXT_LINES: usize = 1;

/// Search query with optional facets for filtering.
#[derive(Debug, Clone, Deserialize)]
pub struct SearchQuery {
    /// The search query string.
    pub q: String,

    /// Maximum number of results to return.
    #[serde(default = "default_limit")]
    pub limit: usize,

    /// Search scope: "metadata", "content", or "all" (default: "all").
    #[serde(default = "default_scope")]
    pub scope: SearchScope,

    /// File type filter: "markdown", "pdf", "image", "video", "audio", "all".
    #[serde(default)]
    pub filetype: Option<String>,

    /// Folder scope: only search within this folder path.
    #[serde(default)]
    pub folder: Option<String>,
}

fn default_limit() -> usize {
    DEFAULT_RESULT_LIMIT
}

fn default_scope() -> SearchScope {
    SearchScope::All
}

/// Search scope determines what fields are searched.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchScope {
    /// Search only metadata (title, path, tags, frontmatter).
    Metadata,
    /// Search only file content (markdown body).
    Content,
    /// Search both metadata and content.
    #[default]
    All,
}

/// A single search result with matched file information.
#[derive(Debug, Clone, Serialize)]
pub struct SearchResult {
    /// URL path to the file.
    pub url_path: String,

    /// File title (from frontmatter or filename).
    pub title: Option<String>,

    /// File description (from frontmatter).
    pub description: Option<String>,

    /// Tags from frontmatter.
    pub tags: Option<String>,

    /// Match score (higher is better).
    pub score: u32,

    /// Snippet of matched content with context.
    pub snippet: Option<String>,

    /// Whether this is a content match (vs metadata match).
    pub is_content_match: bool,

    /// File type category.
    pub filetype: String,
}

/// Search response containing results and metadata.
#[derive(Debug, Clone, Serialize)]
pub struct SearchResponse {
    /// The original query string.
    pub query: String,

    /// Total number of matches found (before limit applied).
    pub total_matches: usize,

    /// Search results, ordered by score descending.
    pub results: Vec<SearchResult>,

    /// Time taken to search in milliseconds.
    pub duration_ms: u64,
}

/// Search engine that combines metadata and content search.
pub struct SearchEngine {
    repo: Arc<Repo>,
    #[allow(dead_code)] // Reserved for future use (e.g., relative path resolution)
    root_dir: std::path::PathBuf,
}

impl SearchEngine {
    /// Create a new search engine with access to the repository.
    pub fn new(repo: Arc<Repo>, root_dir: std::path::PathBuf) -> Self {
        Self { repo, root_dir }
    }

    /// Execute a search query and return results.
    pub fn search(&self, query: &SearchQuery) -> Result<SearchResponse, SearchError> {
        let start = std::time::Instant::now();

        // Early return for empty queries
        if query.q.trim().is_empty() {
            return Ok(SearchResponse {
                query: query.q.clone(),
                total_matches: 0,
                results: Vec::new(),
                duration_ms: 0,
            });
        }

        let mut all_results = Vec::new();

        // Metadata search (always fast, data in memory)
        if query.scope != SearchScope::Content {
            let metadata_results = self.search_metadata(query)?;
            all_results.extend(metadata_results);
        }

        // Content search (requires file I/O)
        if query.scope != SearchScope::Metadata {
            let content_results = self.search_content(query)?;
            all_results.extend(content_results);
        }

        // Deduplicate results (same file might match both metadata and content)
        all_results = Self::deduplicate_results(all_results);

        // Sort by score descending
        all_results.sort_by(|a, b| b.score.cmp(&a.score));

        let total_matches = all_results.len();

        // Apply limit
        all_results.truncate(query.limit);

        let duration_ms = start.elapsed().as_millis() as u64;

        Ok(SearchResponse {
            query: query.q.clone(),
            total_matches,
            results: all_results,
            duration_ms,
        })
    }

    /// Search metadata (titles, paths, tags, frontmatter) using nucleo-matcher.
    fn search_metadata(&self, query: &SearchQuery) -> Result<Vec<SearchResult>, SearchError> {
        let pattern = Pattern::parse(
            &query.q,
            CaseMatching::Ignore,
            Normalization::Smart,
        );

        // Collect files to search, applying folder filter
        let files: Vec<_> = self
            .repo
            .markdown_files
            .pin()
            .iter()
            .filter(|(_, info)| self.matches_folder_filter(info, &query.folder))
            .filter(|(_, info)| self.matches_filetype_filter(info, &query.filetype, true))
            .map(|(_, info)| info.clone())
            .collect();

        // Parallel fuzzy matching with nucleo-matcher
        let results: Vec<SearchResult> = files
            .into_par_iter()
            .filter_map(|info| {
                // Each thread gets its own matcher (thread-local)
                thread_local! {
                    static MATCHER: std::cell::RefCell<Matcher> =
                        std::cell::RefCell::new(Matcher::new(Config::DEFAULT.match_paths()));
                }

                MATCHER.with(|matcher| {
                    let mut matcher = matcher.borrow_mut();
                    self.match_metadata(&pattern, &info, &mut matcher)
                })
            })
            .collect();

        Ok(results)
    }

    /// Match a single file's metadata against the pattern.
    fn match_metadata(
        &self,
        pattern: &Pattern,
        info: &MarkdownInfo,
        matcher: &mut Matcher,
    ) -> Option<SearchResult> {
        let mut best_score: u32 = 0;

        // Match against title (highest priority - 3x boost)
        if let Some(ref fm) = info.frontmatter {
            if let Some(title) = fm.get("title") {
                if let Some(score) = self.fuzzy_match(pattern, title, matcher) {
                    best_score = best_score.max(score.saturating_mul(3));
                }
            }
        }

        // Match against URL path (high priority - 2x boost)
        if let Some(score) = self.fuzzy_match(pattern, &info.url_path, matcher) {
            best_score = best_score.max(score.saturating_mul(2));
        }

        // Match against filename
        if let Some(filename) = info.raw_path.file_stem().and_then(|s| s.to_str()) {
            if let Some(score) = self.fuzzy_match(pattern, filename, matcher) {
                best_score = best_score.max(score.saturating_mul(2));
            }
        }

        // Match against tags (medium priority)
        if let Some(ref fm) = info.frontmatter {
            if let Some(tags) = fm.get("tags") {
                if let Some(score) = self.fuzzy_match(pattern, tags, matcher) {
                    best_score = best_score.max(score);
                }
            }
        }

        // Match against description
        if let Some(ref fm) = info.frontmatter {
            if let Some(desc) = fm.get("description") {
                if let Some(score) = self.fuzzy_match(pattern, desc, matcher) {
                    best_score = best_score.max(score / 2);
                }
            }
        }

        // Match against other frontmatter fields
        if let Some(ref fm) = info.frontmatter {
            for (key, value) in fm.iter() {
                if key != "title" && key != "tags" && key != "description" {
                    if let Some(score) = self.fuzzy_match(pattern, value, matcher) {
                        best_score = best_score.max(score / 3);
                    }
                }
            }
        }

        if best_score > 0 {
            Some(SearchResult {
                url_path: info.url_path.clone(),
                title: info
                    .frontmatter
                    .as_ref()
                    .and_then(|fm| fm.get("title").cloned()),
                description: info
                    .frontmatter
                    .as_ref()
                    .and_then(|fm| fm.get("description").cloned()),
                tags: info
                    .frontmatter
                    .as_ref()
                    .and_then(|fm| fm.get("tags").cloned()),
                score: best_score,
                snippet: None,
                is_content_match: false,
                filetype: "markdown".to_string(),
            })
        } else {
            None
        }
    }

    /// Perform fuzzy matching and return score.
    fn fuzzy_match(&self, pattern: &Pattern, haystack: &str, matcher: &mut Matcher) -> Option<u32> {
        let mut buf = Vec::new();
        let haystack = Utf32Str::new(haystack, &mut buf);
        pattern.score(haystack, matcher)
    }

    /// Search file contents using grep-searcher.
    fn search_content(&self, query: &SearchQuery) -> Result<Vec<SearchResult>, SearchError> {
        // Build regex pattern (escape special characters for literal search)
        let regex_pattern = regex::escape(&query.q);
        let matcher = RegexMatcherBuilder::new()
            .case_insensitive(true)
            .build(&regex_pattern)
            .map_err(|e| SearchError::PatternInvalid {
                pattern: query.q.clone(),
                reason: e.to_string(),
            })?;

        // Collect files to search
        let files: Vec<_> = self
            .repo
            .markdown_files
            .pin()
            .iter()
            .filter(|(_, info)| self.matches_folder_filter(info, &query.folder))
            .filter(|(_, info)| self.matches_filetype_filter(info, &query.filetype, true))
            .map(|(_, info)| info.clone())
            .collect();

        // Parallel content search
        let results: Vec<SearchResult> = files
            .into_par_iter()
            .filter_map(|info| {
                self.search_file_content(&matcher, &info).ok().flatten()
            })
            .collect();

        Ok(results)
    }

    /// Search a single file's content.
    fn search_file_content(
        &self,
        matcher: &grep_regex::RegexMatcher,
        info: &MarkdownInfo,
    ) -> Result<Option<SearchResult>, SearchError> {
        let path = &info.raw_path;

        // Skip if file doesn't exist
        if !path.exists() {
            return Ok(None);
        }

        let mut matches: Vec<(u64, String)> = Vec::new();
        let mut match_count = 0u32;

        let mut searcher = SearcherBuilder::new()
            .binary_detection(BinaryDetection::quit(0x00))
            .line_number(true)
            .before_context(CONTENT_CONTEXT_LINES)
            .after_context(CONTENT_CONTEXT_LINES)
            .build();

        // Search the file
        let search_result = searcher.search_path(
            matcher,
            path,
            UTF8(|line_num, line| {
                match_count += 1;
                if matches.len() < 3 {
                    // Keep first few matches for snippet
                    matches.push((line_num, line.trim().to_string()));
                }
                Ok(true)
            }),
        );

        if let Err(e) = search_result {
            tracing::debug!("Search error in {:?}: {}", path, e);
            return Ok(None);
        }

        if match_count > 0 {
            // Build snippet from first match
            let snippet = matches
                .first()
                .map(|(line_num, line)| {
                    let mut s = format!("Line {}: ", line_num);
                    if line.len() > MAX_SNIPPET_LENGTH {
                        s.push_str(&line[..MAX_SNIPPET_LENGTH]);
                        s.push_str("...");
                    } else {
                        s.push_str(line);
                    }
                    s
                });

            // Score based on match count (more matches = higher score)
            // Content matches are weighted lower than metadata matches
            let score = match_count.min(100);

            Ok(Some(SearchResult {
                url_path: info.url_path.clone(),
                title: info
                    .frontmatter
                    .as_ref()
                    .and_then(|fm| fm.get("title").cloned()),
                description: info
                    .frontmatter
                    .as_ref()
                    .and_then(|fm| fm.get("description").cloned()),
                tags: info
                    .frontmatter
                    .as_ref()
                    .and_then(|fm| fm.get("tags").cloned()),
                score,
                snippet,
                is_content_match: true,
                filetype: "markdown".to_string(),
            }))
        } else {
            Ok(None)
        }
    }

    /// Check if a file matches the folder filter.
    fn matches_folder_filter(&self, info: &MarkdownInfo, folder: &Option<String>) -> bool {
        match folder {
            Some(folder_path) => {
                let normalized = if folder_path.starts_with('/') {
                    folder_path.clone()
                } else {
                    format!("/{}", folder_path)
                };
                info.url_path.starts_with(&normalized)
            }
            None => true,
        }
    }

    /// Check if a file matches the filetype filter.
    fn matches_filetype_filter(
        &self,
        _info: &MarkdownInfo,
        filetype: &Option<String>,
        is_markdown: bool,
    ) -> bool {
        match filetype {
            Some(ft) => {
                let ft_lower = ft.to_lowercase();
                match ft_lower.as_str() {
                    "markdown" | "md" => is_markdown,
                    "all" => true,
                    _ => false, // For now, only markdown files are searchable
                }
            }
            None => true,
        }
    }

    /// Deduplicate results, preferring higher-scoring matches.
    fn deduplicate_results(mut results: Vec<SearchResult>) -> Vec<SearchResult> {
        // Sort by URL path, then by score descending
        results.sort_by(|a, b| {
            a.url_path
                .cmp(&b.url_path)
                .then_with(|| b.score.cmp(&a.score))
        });

        // Deduplicate keeping highest score, merging content match info
        let mut seen = std::collections::HashMap::new();
        for result in results {
            seen.entry(result.url_path.clone())
                .and_modify(|existing: &mut SearchResult| {
                    // Keep the higher score
                    if result.score > existing.score {
                        existing.score = result.score;
                    }
                    // If either is a content match, keep the snippet
                    if result.is_content_match && result.snippet.is_some() {
                        existing.snippet = result.snippet.clone();
                        existing.is_content_match = true;
                    }
                })
                .or_insert(result);
        }

        seen.into_values().collect()
    }
}

/// Search other files (PDFs, images, etc.) by path only.
/// This is a lighter-weight search that doesn't read file contents.
pub fn search_other_files(
    repo: &Repo,
    query: &str,
    folder: Option<&str>,
    filetype: Option<&str>,
    limit: usize,
) -> Vec<SearchResult> {
    let pattern = Pattern::parse(query, CaseMatching::Ignore, Normalization::Smart);

    let files: Vec<_> = repo
        .other_files
        .pin()
        .iter()
        .filter(|(_, info)| {
            // Folder filter
            if let Some(folder_path) = folder {
                let normalized = if folder_path.starts_with('/') {
                    folder_path.to_string()
                } else {
                    format!("/{}", folder_path)
                };
                if !info.url_path.starts_with(&normalized) {
                    return false;
                }
            }

            // Filetype filter
            if let Some(ft) = filetype {
                let ft_lower = ft.to_lowercase();
                if ft_lower != "all" {
                    let ext = info
                        .raw_path
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("")
                        .to_lowercase();
                    let matches = match ft_lower.as_str() {
                        "pdf" => ext == "pdf",
                        "image" => matches!(ext.as_str(), "jpg" | "jpeg" | "png" | "gif" | "webp" | "svg"),
                        "video" => matches!(ext.as_str(), "mp4" | "webm" | "mov" | "avi"),
                        "audio" => matches!(ext.as_str(), "mp3" | "wav" | "ogg" | "m4a" | "flac"),
                        _ => true,
                    };
                    if !matches {
                        return false;
                    }
                }
            }

            true
        })
        .map(|(_, info)| info.clone())
        .collect();

    // Parallel fuzzy matching
    let mut results: Vec<SearchResult> = files
        .into_par_iter()
        .filter_map(|info| {
            thread_local! {
                static MATCHER: std::cell::RefCell<Matcher> =
                    std::cell::RefCell::new(Matcher::new(Config::DEFAULT.match_paths()));
            }

            MATCHER.with(|matcher| {
                let mut matcher = matcher.borrow_mut();
                let mut buf = Vec::new();
                let haystack = Utf32Str::new(&info.url_path, &mut buf);

                pattern.score(haystack, &mut matcher).map(|score| {
                    let ext = info
                        .raw_path
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("")
                        .to_lowercase();

                    let filetype = match ext.as_str() {
                        "pdf" => "pdf",
                        "jpg" | "jpeg" | "png" | "gif" | "webp" | "svg" => "image",
                        "mp4" | "webm" | "mov" | "avi" => "video",
                        "mp3" | "wav" | "ogg" | "m4a" | "flac" => "audio",
                        _ => "other",
                    };

                    SearchResult {
                        url_path: info.url_path.clone(),
                        title: None,
                        description: None,
                        tags: None,
                        score,
                        snippet: None,
                        is_content_match: false,
                        filetype: filetype.to_string(),
                    }
                })
            })
        })
        .collect();

    // Sort by score and limit
    results.sort_by(|a, b| b.score.cmp(&a.score));
    results.truncate(limit);

    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_search_query_defaults() {
        let json = r#"{"q": "test"}"#;
        let query: SearchQuery = serde_json::from_str(json).unwrap();

        assert_eq!(query.q, "test");
        assert_eq!(query.limit, DEFAULT_RESULT_LIMIT);
        assert_eq!(query.scope, SearchScope::All);
        assert!(query.filetype.is_none());
        assert!(query.folder.is_none());
    }

    #[test]
    fn test_search_query_with_options() {
        let json = r#"{"q": "test", "limit": 10, "scope": "metadata", "folder": "/docs"}"#;
        let query: SearchQuery = serde_json::from_str(json).unwrap();

        assert_eq!(query.q, "test");
        assert_eq!(query.limit, 10);
        assert_eq!(query.scope, SearchScope::Metadata);
        assert_eq!(query.folder, Some("/docs".to_string()));
    }

    #[test]
    fn test_search_scope_deserialization() {
        assert_eq!(
            serde_json::from_str::<SearchScope>(r#""metadata""#).unwrap(),
            SearchScope::Metadata
        );
        assert_eq!(
            serde_json::from_str::<SearchScope>(r#""content""#).unwrap(),
            SearchScope::Content
        );
        assert_eq!(
            serde_json::from_str::<SearchScope>(r#""all""#).unwrap(),
            SearchScope::All
        );
    }

    #[test]
    fn test_deduplicate_results() {
        let results = vec![
            SearchResult {
                url_path: "/test/".to_string(),
                title: Some("Test".to_string()),
                description: None,
                tags: None,
                score: 50,
                snippet: None,
                is_content_match: false,
                filetype: "markdown".to_string(),
            },
            SearchResult {
                url_path: "/test/".to_string(),
                title: Some("Test".to_string()),
                description: None,
                tags: None,
                score: 100,
                snippet: Some("content match".to_string()),
                is_content_match: true,
                filetype: "markdown".to_string(),
            },
        ];

        let deduped = SearchEngine::deduplicate_results(results);
        assert_eq!(deduped.len(), 1);

        let result = &deduped[0];
        assert_eq!(result.score, 100); // Higher score kept
        assert!(result.snippet.is_some()); // Content snippet kept
    }
}
