//! Search functionality for the mbr markdown browser.
//!
//! Provides two search modes:
//! - **Metadata search**: Fast fuzzy matching of titles, paths, and frontmatter
//!   using nucleo-matcher. Data is already in memory from repo scanning.
//! - **Content search**: Full-text search through markdown file contents
//!   using grep-searcher with SIMD acceleration.
//!
//! Supports faceted search with `key:value` syntax for filtering on specific
//! frontmatter fields (e.g., `category:rust` or `tags:async`).
//!
//! Both modes use sequential iteration to avoid rayon thread pool contention
//! when multiple searches run concurrently (e.g., from per-keystroke queries).

use std::sync::Arc;

use grep_regex::RegexMatcherBuilder;
use grep_searcher::sinks::UTF8;
use grep_searcher::{BinaryDetection, SearcherBuilder};
use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use serde::{Deserialize, Serialize};

use crate::errors::SearchError;
use crate::repo::{MarkdownInfo, OtherFileInfo, Repo};

/// Maximum number of search results to return by default.
pub const DEFAULT_RESULT_LIMIT: usize = 50;

/// Maximum snippet length in characters.
const MAX_SNIPPET_LENGTH: usize = 200;

/// Context lines to show around content matches.
const CONTENT_CONTEXT_LINES: usize = 1;

/// Base score for facet (key:value) matches against metadata.
const FACET_MATCH_BASE_SCORE: u32 = 100;

/// Maximum score for content (full-text) matches.
const MAX_CONTENT_MATCH_SCORE: u32 = 100;

/// Maximum bytes of extracted text to sample for fuzzy matching.
const MAX_TEXT_SAMPLE_BYTES: usize = 5000;

/// Parsed query with separated terms and facets.
///
/// A query like `rust category:programming author:alice` is parsed into:
/// - terms: `["rust"]`
/// - facets: `[("category", "programming"), ("author", "alice")]`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedQuery {
    /// Plain search terms (AND'd together for matching).
    pub terms: Vec<String>,
    /// Facet filters as (field_name, value) pairs.
    pub facets: Vec<(String, String)>,
}

impl ParsedQuery {
    /// Returns true if the query is empty (no terms and no facets).
    pub fn is_empty(&self) -> bool {
        self.terms.is_empty() && self.facets.is_empty()
    }
}

/// Parse a query string into terms and facets.
///
/// Facets use `key:value` syntax. URLs (containing `://`) are preserved as terms.
///
/// # Examples
///
/// ```
/// use mbr::search::parse_query;
///
/// let parsed = parse_query("rust async");
/// assert_eq!(parsed.terms, vec!["rust", "async"]);
/// assert!(parsed.facets.is_empty());
///
/// let parsed = parse_query("category:rust guide");
/// assert_eq!(parsed.terms, vec!["guide"]);
/// assert_eq!(parsed.facets, vec![("category".to_string(), "rust".to_string())]);
/// ```
pub fn parse_query(q: &str) -> ParsedQuery {
    let mut terms = Vec::new();
    let mut facets = Vec::new();

    for token in q.split_whitespace() {
        // Check if this looks like a facet (contains : but not ://)
        if let Some(colon_pos) = token.find(':') {
            // Skip if it's a URL (contains ://)
            if token.contains("://") {
                terms.push(token.to_string());
                continue;
            }

            // Skip if colon is at start or end (not a valid facet)
            if colon_pos == 0 || colon_pos == token.len() - 1 {
                terms.push(token.to_string());
                continue;
            }

            let (key, value) = token.split_at(colon_pos);
            let value = &value[1..]; // Skip the colon

            // Only add if both key and value are non-empty
            if !key.is_empty() && !value.is_empty() {
                facets.push((key.to_string(), value.to_string()));
            } else {
                terms.push(token.to_string());
            }
        } else {
            terms.push(token.to_string());
        }
    }

    ParsedQuery { terms, facets }
}

/// Check if a frontmatter field value contains the facet value (case-insensitive).
/// Handles both string values and array values (for array tags).
fn facet_matches(
    frontmatter: Option<&std::collections::HashMap<String, serde_json::Value>>,
    field: &str,
    value: &str,
) -> bool {
    match frontmatter.and_then(|fm| fm.get(field)) {
        Some(serde_json::Value::String(s)) => s.to_lowercase().contains(&value.to_lowercase()),
        Some(serde_json::Value::Array(arr)) => arr.iter().any(|v| {
            v.as_str()
                .map(|s| s.to_lowercase().contains(&value.to_lowercase()))
                .unwrap_or(false)
        }),
        _ => false,
    }
}

/// Get the scoring weight for a frontmatter field.
fn field_weight(field: &str) -> u32 {
    match field {
        "title" => 3,
        "tags" | "keywords" | "categories" | "category" => 2,
        "description" | "summary" => 1,
        _ => 1, // All other fields get base weight
    }
}

/// Folder scope determines whether to search everywhere or just current folder.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FolderScope {
    /// Search only within the specified folder and subfolders.
    Current,
    /// Search the entire repository.
    #[default]
    Everywhere,
}

/// Search query with optional facets for filtering.
#[derive(Debug, Clone, Deserialize)]
pub struct SearchQuery {
    /// The search query string (supports `key:value` facet syntax).
    pub q: String,

    /// Maximum number of results to return.
    #[serde(default = "default_limit")]
    pub limit: usize,

    /// Search scope: "metadata", "content", or "all" (default: "all").
    #[serde(default = "default_scope")]
    pub scope: SearchScope,

    /// File type filter: "markdown" or "all" (includes PDFs and text files).
    #[serde(default)]
    pub filetype: Option<String>,

    /// Folder path prefix for filtering (e.g., "/docs/").
    #[serde(default)]
    pub folder: Option<String>,

    /// Folder scope: "current" (within folder) or "everywhere" (whole repo).
    #[serde(default)]
    pub folder_scope: FolderScope,
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

    /// True when the background scan is still in progress (results may be incomplete).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub scan_in_progress: bool,
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

        // Parse the query to extract facets
        let parsed = parse_query(&query.q);

        // Early return for empty queries
        if parsed.is_empty() {
            return Ok(SearchResponse {
                query: query.q.clone(),
                total_matches: 0,
                results: Vec::new(),
                duration_ms: 0,
                scan_in_progress: false,
            });
        }

        let mut all_results = Vec::new();

        // Metadata search (always fast, data in memory)
        if query.scope != SearchScope::Content {
            all_results.extend(self.search_metadata(query, &parsed)?);
        }

        // Content search (requires file I/O) - only if we have search terms
        if query.scope != SearchScope::Metadata && !parsed.terms.is_empty() {
            all_results.extend(self.search_content(query, &parsed)?);
        }

        // Search other files (PDFs, text files) when filetype is "all"
        if self.should_search_other_files(&query.filetype) {
            all_results.extend(self.search_other_files_metadata(query, &parsed)?);
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
            scan_in_progress: false,
        })
    }

    /// Search metadata (titles, paths, tags, frontmatter) using nucleo-matcher.
    fn search_metadata(
        &self,
        query: &SearchQuery,
        parsed: &ParsedQuery,
    ) -> Result<Vec<SearchResult>, SearchError> {
        // Build pattern from terms only (facets are handled separately)
        let pattern = if parsed.terms.is_empty() {
            None
        } else {
            Some(Pattern::parse(
                &parsed.terms.join(" "),
                CaseMatching::Ignore,
                Normalization::Smart,
            ))
        };

        // Collect files to search, applying folder filter and facet pre-filtering
        let files: Vec<_> =
            self.repo
                .markdown_files
                .pin()
                .iter()
                .filter(|(_, info)| {
                    self.matches_folder_filter(info, &query.folder, &query.folder_scope)
                })
                .filter(|(_, info)| self.matches_filetype_filter(info, &query.filetype, true))
                // Apply facet filters - all facets must match
                .filter(|(_, info)| {
                    parsed.facets.iter().all(|(field, value)| {
                        facet_matches(info.frontmatter.as_ref(), field, value)
                    })
                })
                .map(|(_, info)| info.clone())
                .collect();

        // Intentionally sequential: par_iter causes 30s stalls from rayon thread pool contention
        // when multiple per-keystroke searches run concurrently.
        let mut matcher = Matcher::new(Config::DEFAULT.match_paths());
        let results: Vec<SearchResult> = files
            .into_iter()
            .filter_map(|info| self.match_metadata(pattern.as_ref(), &info, &mut matcher))
            .collect();

        Ok(results)
    }

    /// Match a single file's metadata against the pattern.
    ///
    /// If pattern is None (facet-only query), returns a base score for matching files.
    fn match_metadata(
        &self,
        pattern: Option<&Pattern>,
        info: &MarkdownInfo,
        matcher: &mut Matcher,
    ) -> Option<SearchResult> {
        // Helper to extract string from frontmatter value
        let extract_string = |fm: &std::collections::HashMap<String, serde_json::Value>,
                              key: &str|
         -> Option<String> {
            fm.get(key).and_then(|v| match v {
                serde_json::Value::String(s) => Some(s.clone()),
                serde_json::Value::Array(arr) => {
                    let strings: Vec<&str> = arr.iter().filter_map(|v| v.as_str()).collect();
                    if strings.is_empty() {
                        None
                    } else {
                        Some(strings.join(", "))
                    }
                }
                _ => None,
            })
        };

        // If no pattern (facet-only query), return base score for files that passed facet filter
        let Some(pattern) = pattern else {
            return Some(SearchResult {
                url_path: info.url_path.clone(),
                title: info
                    .frontmatter
                    .as_ref()
                    .and_then(|fm| extract_string(fm, "title")),
                description: info
                    .frontmatter
                    .as_ref()
                    .and_then(|fm| extract_string(fm, "description")),
                tags: info
                    .frontmatter
                    .as_ref()
                    .and_then(|fm| extract_string(fm, "tags")),
                score: FACET_MATCH_BASE_SCORE,
                snippet: None,
                is_content_match: false,
                filetype: "markdown".to_string(),
            });
        };

        let mut best_score: u32 = 0;

        // Match against URL path (high priority - 2x boost)
        if let Some(score) = self.fuzzy_match(pattern, &info.url_path, matcher) {
            best_score = best_score.max(score.saturating_mul(2));
        }

        // Match against filename (high priority - 2x boost)
        if let Some(filename) = info.raw_path.file_stem().and_then(|s| s.to_str())
            && let Some(score) = self.fuzzy_match(pattern, filename, matcher)
        {
            best_score = best_score.max(score.saturating_mul(2));
        }

        // Match against all frontmatter fields with dynamic weights
        if let Some(ref fm) = info.frontmatter {
            for (key, value) in fm.iter() {
                // Match against string values or each element of arrays
                match value {
                    serde_json::Value::String(s) => {
                        if let Some(score) = self.fuzzy_match(pattern, s, matcher) {
                            let weight = field_weight(key);
                            best_score = best_score.max(score.saturating_mul(weight));
                        }
                    }
                    serde_json::Value::Array(arr) => {
                        for item in arr {
                            if let Some(s) = item.as_str()
                                && let Some(score) = self.fuzzy_match(pattern, s, matcher)
                            {
                                let weight = field_weight(key);
                                best_score = best_score.max(score.saturating_mul(weight));
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        if best_score > 0 {
            Some(SearchResult {
                url_path: info.url_path.clone(),
                title: info
                    .frontmatter
                    .as_ref()
                    .and_then(|fm| extract_string(fm, "title")),
                description: info
                    .frontmatter
                    .as_ref()
                    .and_then(|fm| extract_string(fm, "description")),
                tags: info
                    .frontmatter
                    .as_ref()
                    .and_then(|fm| extract_string(fm, "tags")),
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
    fn search_content(
        &self,
        query: &SearchQuery,
        parsed: &ParsedQuery,
    ) -> Result<Vec<SearchResult>, SearchError> {
        // Build regex pattern from terms (escape special characters for literal search)
        let search_terms = parsed.terms.join(" ");
        let regex_pattern = regex::escape(&search_terms);
        let matcher = RegexMatcherBuilder::new()
            .case_insensitive(true)
            .build(&regex_pattern)
            .map_err(|e| SearchError::PatternInvalid {
                pattern: search_terms,
                reason: e.to_string(),
            })?;

        // Collect files to search, applying facet filters
        let files: Vec<_> =
            self.repo
                .markdown_files
                .pin()
                .iter()
                .filter(|(_, info)| {
                    self.matches_folder_filter(info, &query.folder, &query.folder_scope)
                })
                .filter(|(_, info)| self.matches_filetype_filter(info, &query.filetype, true))
                // Apply facet filters - all facets must match
                .filter(|(_, info)| {
                    parsed.facets.iter().all(|(field, value)| {
                        facet_matches(info.frontmatter.as_ref(), field, value)
                    })
                })
                .map(|(_, info)| info.clone())
                .collect();

        // Sequential content search.
        // Same rationale as metadata: concurrent par_iter calls from rapid
        // keystroke searches cause 30s rayon thread pool contention. Sequential
        // grep over 276 files takes ~13ms.
        let results: Vec<SearchResult> = files
            .into_iter()
            .filter_map(|info| self.search_file_content(&matcher, &info).ok().flatten())
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
            let snippet = matches.first().map(|(line_num, line)| {
                let mut s = format!("Line {}: ", line_num);
                if line.len() > MAX_SNIPPET_LENGTH {
                    // Use floor_char_boundary to avoid slicing in the middle of a UTF-8 character
                    let end = line.floor_char_boundary(MAX_SNIPPET_LENGTH);
                    s.push_str(&line[..end]);
                    s.push_str("...");
                } else {
                    s.push_str(line);
                }
                s
            });

            // Score based on match count (more matches = higher score)
            // Content matches are weighted lower than metadata matches
            let score = match_count.min(MAX_CONTENT_MATCH_SCORE);

            // Helper to extract string from frontmatter value
            let extract_string = |fm: &std::collections::HashMap<String, serde_json::Value>,
                                  key: &str|
             -> Option<String> {
                fm.get(key).and_then(|v| match v {
                    serde_json::Value::String(s) => Some(s.clone()),
                    serde_json::Value::Array(arr) => {
                        let strings: Vec<&str> = arr.iter().filter_map(|v| v.as_str()).collect();
                        if strings.is_empty() {
                            None
                        } else {
                            Some(strings.join(", "))
                        }
                    }
                    _ => None,
                })
            };

            Ok(Some(SearchResult {
                url_path: info.url_path.clone(),
                title: info
                    .frontmatter
                    .as_ref()
                    .and_then(|fm| extract_string(fm, "title")),
                description: info
                    .frontmatter
                    .as_ref()
                    .and_then(|fm| extract_string(fm, "description")),
                tags: info
                    .frontmatter
                    .as_ref()
                    .and_then(|fm| extract_string(fm, "tags")),
                score,
                snippet,
                is_content_match: true,
                filetype: "markdown".to_string(),
            }))
        } else {
            Ok(None)
        }
    }

    /// Check if a file matches the folder filter based on folder scope.
    fn matches_folder_filter(
        &self,
        info: &MarkdownInfo,
        folder: &Option<String>,
        folder_scope: &FolderScope,
    ) -> bool {
        // If scope is Everywhere, always match
        if *folder_scope == FolderScope::Everywhere {
            return true;
        }

        // For Current scope, check folder prefix
        match folder {
            Some(folder_path) => {
                let normalized = if folder_path.starts_with('/') {
                    folder_path.clone()
                } else {
                    format!("/{}", folder_path)
                };
                info.url_path.starts_with(&normalized)
            }
            None => true, // No folder specified = match all
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
                    _ => false,
                }
            }
            None => true,
        }
    }

    /// Check if we should search other files (PDFs, text files) based on filetype filter.
    fn should_search_other_files(&self, filetype: &Option<String>) -> bool {
        filetype
            .as_ref()
            .map(|ft| ft.to_lowercase() == "all")
            .unwrap_or(false)
    }

    /// Search other files (PDFs, text files) by path and extracted text.
    fn search_other_files_metadata(
        &self,
        query: &SearchQuery,
        parsed: &ParsedQuery,
    ) -> Result<Vec<SearchResult>, SearchError> {
        // Build pattern from terms (facets don't apply to other files - they have no frontmatter)
        let pattern = if parsed.terms.is_empty() {
            // No terms and other files have no frontmatter, so facet-only queries return nothing
            return Ok(Vec::new());
        } else {
            Pattern::parse(
                &parsed.terms.join(" "),
                CaseMatching::Ignore,
                Normalization::Smart,
            )
        };

        // Collect searchable files (PDFs and text files with extracted text)
        let files: Vec<_> = self
            .repo
            .other_files
            .pin()
            .iter()
            .filter(|(_, info)| info.is_searchable())
            .filter(|(_, info)| {
                self.matches_other_file_folder_filter(info, &query.folder, &query.folder_scope)
            })
            .map(|(_, info)| info.clone())
            .collect();

        // Sequential fuzzy matching (same rayon contention rationale as search_metadata)
        let mut matcher = Matcher::new(Config::DEFAULT.match_paths());
        let results: Vec<SearchResult> = files
            .into_iter()
            .filter_map(|info| self.match_other_file(&pattern, &info, &mut matcher))
            .collect();

        Ok(results)
    }

    /// Match a single other file against the pattern.
    fn match_other_file(
        &self,
        pattern: &Pattern,
        info: &OtherFileInfo,
        matcher: &mut Matcher,
    ) -> Option<SearchResult> {
        let mut best_score: u32 = 0;

        // Match against URL path
        if let Some(score) = self.fuzzy_match(pattern, &info.url_path, matcher) {
            best_score = best_score.max(score.saturating_mul(2));
        }

        // Match against filename
        if let Some(filename) = info.raw_path.file_stem().and_then(|s| s.to_str())
            && let Some(score) = self.fuzzy_match(pattern, filename, matcher)
        {
            best_score = best_score.max(score.saturating_mul(2));
        }

        // Match against extracted text (lower weight since it's body content)
        if let Some(ref text) = info.extracted_text {
            // Sample the text for matching (first ~5000 bytes for performance)
            // Use floor_char_boundary to avoid slicing in the middle of a UTF-8 character
            let sample = if text.len() > MAX_TEXT_SAMPLE_BYTES {
                &text[..text.floor_char_boundary(MAX_TEXT_SAMPLE_BYTES)]
            } else {
                text.as_str()
            };
            if let Some(score) = self.fuzzy_match(pattern, sample, matcher) {
                best_score = best_score.max(score);
            }
        }

        if best_score > 0 {
            // Build snippet from extracted text
            let snippet = info.extracted_text.as_ref().map(|text| {
                if text.len() > MAX_SNIPPET_LENGTH {
                    // Use floor_char_boundary to avoid slicing in the middle of a UTF-8 character
                    let end = text.floor_char_boundary(MAX_SNIPPET_LENGTH);
                    format!("{}...", &text[..end])
                } else {
                    text.clone()
                }
            });

            Some(SearchResult {
                url_path: info.url_path.clone(),
                title: info
                    .raw_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(|s| s.to_string()),
                description: None,
                tags: None,
                score: best_score,
                snippet,
                is_content_match: info.extracted_text.is_some(),
                filetype: info.filetype().to_string(),
            })
        } else {
            None
        }
    }

    /// Check if an other file matches the folder filter.
    fn matches_other_file_folder_filter(
        &self,
        info: &OtherFileInfo,
        folder: &Option<String>,
        folder_scope: &FolderScope,
    ) -> bool {
        if *folder_scope == FolderScope::Everywhere {
            return true;
        }

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
                        "image" => matches!(
                            ext.as_str(),
                            "jpg" | "jpeg" | "png" | "gif" | "webp" | "svg"
                        ),
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

    // Sequential fuzzy matching (same rayon contention rationale as search_metadata)
    let mut matcher = Matcher::new(Config::DEFAULT.match_paths());
    let mut results: Vec<SearchResult> = files
        .into_iter()
        .filter_map(|info| {
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

    // ==================== Query Parser Tests ====================

    #[test]
    fn test_parse_query_simple_terms() {
        let parsed = parse_query("rust async");
        assert_eq!(parsed.terms, vec!["rust", "async"]);
        assert!(parsed.facets.is_empty());
    }

    #[test]
    fn test_parse_query_single_facet() {
        let parsed = parse_query("category:rust");
        assert!(parsed.terms.is_empty());
        assert_eq!(
            parsed.facets,
            vec![("category".to_string(), "rust".to_string())]
        );
    }

    #[test]
    fn test_parse_query_mixed_terms_and_facets() {
        let parsed = parse_query("guide category:programming author:alice");
        assert_eq!(parsed.terms, vec!["guide"]);
        assert_eq!(
            parsed.facets,
            vec![
                ("category".to_string(), "programming".to_string()),
                ("author".to_string(), "alice".to_string()),
            ]
        );
    }

    #[test]
    fn test_parse_query_url_not_facet() {
        let parsed = parse_query("check https://example.com");
        assert_eq!(parsed.terms, vec!["check", "https://example.com"]);
        assert!(parsed.facets.is_empty());
    }

    #[test]
    fn test_parse_query_http_url_not_facet() {
        let parsed = parse_query("link http://example.com/path");
        assert_eq!(parsed.terms, vec!["link", "http://example.com/path"]);
        assert!(parsed.facets.is_empty());
    }

    #[test]
    fn test_parse_query_colon_at_start_not_facet() {
        let parsed = parse_query(":value");
        assert_eq!(parsed.terms, vec![":value"]);
        assert!(parsed.facets.is_empty());
    }

    #[test]
    fn test_parse_query_colon_at_end_not_facet() {
        let parsed = parse_query("key:");
        assert_eq!(parsed.terms, vec!["key:"]);
        assert!(parsed.facets.is_empty());
    }

    #[test]
    fn test_parse_query_empty() {
        let parsed = parse_query("");
        assert!(parsed.is_empty());
    }

    #[test]
    fn test_parse_query_whitespace_only() {
        let parsed = parse_query("   ");
        assert!(parsed.is_empty());
    }

    #[test]
    fn test_parsed_query_is_empty() {
        assert!(
            ParsedQuery {
                terms: vec![],
                facets: vec![]
            }
            .is_empty()
        );
        assert!(
            !ParsedQuery {
                terms: vec!["test".to_string()],
                facets: vec![]
            }
            .is_empty()
        );
        assert!(
            !ParsedQuery {
                terms: vec![],
                facets: vec![("k".to_string(), "v".to_string())]
            }
            .is_empty()
        );
    }

    // ==================== Facet Matching Tests ====================

    #[test]
    fn test_facet_matches_exact() {
        let mut fm = std::collections::HashMap::new();
        fm.insert(
            "category".to_string(),
            serde_json::Value::String("programming".to_string()),
        );

        assert!(facet_matches(Some(&fm), "category", "programming"));
    }

    #[test]
    fn test_facet_matches_contains() {
        let mut fm = std::collections::HashMap::new();
        fm.insert(
            "category".to_string(),
            serde_json::Value::String("systems programming".to_string()),
        );

        assert!(facet_matches(Some(&fm), "category", "programming"));
        assert!(facet_matches(Some(&fm), "category", "systems"));
    }

    #[test]
    fn test_facet_matches_case_insensitive() {
        let mut fm = std::collections::HashMap::new();
        fm.insert(
            "category".to_string(),
            serde_json::Value::String("Systems Programming".to_string()),
        );

        assert!(facet_matches(Some(&fm), "category", "PROGRAMMING"));
        assert!(facet_matches(Some(&fm), "category", "systems"));
    }

    #[test]
    fn test_facet_matches_missing_field() {
        let mut fm = std::collections::HashMap::new();
        fm.insert(
            "title".to_string(),
            serde_json::Value::String("test".to_string()),
        );

        assert!(!facet_matches(Some(&fm), "category", "anything"));
    }

    #[test]
    fn test_facet_matches_no_frontmatter() {
        assert!(!facet_matches(None, "category", "anything"));
    }

    #[test]
    fn test_facet_matches_no_match() {
        let mut fm = std::collections::HashMap::new();
        fm.insert(
            "category".to_string(),
            serde_json::Value::String("web development".to_string()),
        );

        assert!(!facet_matches(Some(&fm), "category", "systems"));
    }

    #[test]
    fn test_facet_matches_array_value() {
        let mut fm = std::collections::HashMap::new();
        fm.insert("tags".to_string(), serde_json::json!(["rust", "python"]));

        assert!(facet_matches(Some(&fm), "tags", "rust"));
        assert!(facet_matches(Some(&fm), "tags", "python"));
        assert!(!facet_matches(Some(&fm), "tags", "go"));
    }

    // ==================== Field Weight Tests ====================

    #[test]
    fn test_field_weight_priorities() {
        assert_eq!(field_weight("title"), 3);
        assert_eq!(field_weight("tags"), 2);
        assert_eq!(field_weight("keywords"), 2);
        assert_eq!(field_weight("categories"), 2);
        assert_eq!(field_weight("category"), 2);
        assert_eq!(field_weight("description"), 1);
        assert_eq!(field_weight("summary"), 1);
        assert_eq!(field_weight("custom_field"), 1);
    }

    // ==================== Folder Scope Tests ====================

    #[test]
    fn test_folder_scope_deserialization() {
        assert_eq!(
            serde_json::from_str::<FolderScope>(r#""current""#).unwrap(),
            FolderScope::Current
        );
        assert_eq!(
            serde_json::from_str::<FolderScope>(r#""everywhere""#).unwrap(),
            FolderScope::Everywhere
        );
    }

    #[test]
    fn test_search_query_with_folder_scope() {
        let json = r#"{"q": "test", "folder": "/docs/", "folder_scope": "current"}"#;
        let query: SearchQuery = serde_json::from_str(json).unwrap();

        assert_eq!(query.folder, Some("/docs/".to_string()));
        assert_eq!(query.folder_scope, FolderScope::Current);
    }

    #[test]
    fn test_search_query_folder_scope_default() {
        let json = r#"{"q": "test"}"#;
        let query: SearchQuery = serde_json::from_str(json).unwrap();

        assert_eq!(query.folder_scope, FolderScope::Everywhere);
    }

    // ==================== SearchResponse Serialization Tests ====================

    #[test]
    fn test_search_response_scan_in_progress_serialization() {
        // Verify scan_in_progress is skipped when false (default)
        let response = SearchResponse {
            query: "test".to_string(),
            total_matches: 0,
            results: vec![],
            duration_ms: 0,
            scan_in_progress: false,
        };
        let json = serde_json::to_string(&response).unwrap();
        assert!(!json.contains("scan_in_progress"));

        // Verify scan_in_progress is present when true
        let response2 = SearchResponse {
            scan_in_progress: true,
            ..response
        };
        let json2 = serde_json::to_string(&response2).unwrap();
        assert!(json2.contains("\"scan_in_progress\":true"));
    }
}
