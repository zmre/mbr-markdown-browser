//! Tag index module for tracking tagged pages.
//!
//! This module provides a thread-safe index of pages organized by tag source and value.
//! Tags are normalized (lowercase, spaces as underscores) for consistent lookup,
//! while preserving the original display form.

use papaya::HashMap;
use serde::Serialize;
use std::collections::HashSet;

use crate::wikilink::normalize_tag_value;

/// Information about a page tagged with a specific tag.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TaggedPage {
    /// URL path to the page (e.g., "/docs/rust-guide/")
    pub url_path: String,
    /// Page title (from frontmatter or filename)
    pub title: String,
    /// Optional page description (from frontmatter)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The original tag value as it appears on this page (preserves case/spacing)
    pub original_tag_value: String,
}

impl TaggedPage {
    /// Creates a new TaggedPage.
    pub fn new(
        url_path: impl Into<String>,
        title: impl Into<String>,
        original_tag_value: impl Into<String>,
    ) -> Self {
        Self {
            url_path: url_path.into(),
            title: title.into(),
            description: None,
            original_tag_value: original_tag_value.into(),
        }
    }

    /// Creates a new TaggedPage with a description.
    pub fn with_description(
        url_path: impl Into<String>,
        title: impl Into<String>,
        description: impl Into<String>,
        original_tag_value: impl Into<String>,
    ) -> Self {
        Self {
            url_path: url_path.into(),
            title: title.into(),
            description: Some(description.into()),
            original_tag_value: original_tag_value.into(),
        }
    }
}

/// A single tag with its normalized key and display value.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TagInfo {
    /// Normalized tag value (lowercase, underscores for spaces) - used in URLs
    pub normalized: String,
    /// Display value (first occurrence's original form, e.g., "Joshua Jay")
    pub display: String,
    /// Number of pages with this tag
    pub count: usize,
}

/// Thread-safe index of tagged pages.
///
/// Uses papaya concurrent HashMap for lock-free reads and writes.
/// Keys are normalized (lowercase source, lowercase+underscore value),
/// while display forms are preserved from the first occurrence.
pub struct TagIndex {
    /// Map of (source, tag_value) -> Vec<TaggedPage>
    /// Key format: "{normalized_source}:{normalized_value}"
    index: HashMap<String, Vec<TaggedPage>>,
    /// Map of normalized tag key -> display value (first occurrence wins)
    display_values: HashMap<String, String>,
    /// Set of all sources that have at least one tag
    sources: HashMap<String, String>, // normalized -> display
}

impl Default for TagIndex {
    fn default() -> Self {
        Self::new()
    }
}

impl TagIndex {
    /// Creates a new empty TagIndex.
    pub fn new() -> Self {
        Self {
            index: HashMap::new(),
            display_values: HashMap::new(),
            sources: HashMap::new(),
        }
    }

    /// Normalizes a source name for use as a key.
    pub fn normalize_source(source: &str) -> String {
        source.to_lowercase()
    }

    /// Normalizes a tag value for use as a key.
    pub fn normalize_value(value: &str) -> String {
        normalize_tag_value(value)
    }

    /// Builds a cache key from source and value.
    fn make_key(normalized_source: &str, normalized_value: &str) -> String {
        format!("{}:{}", normalized_source, normalized_value)
    }

    /// Adds a page to the index under the given source and tag value.
    ///
    /// # Arguments
    ///
    /// * `source` - The tag source (e.g., "tags", "performers")
    /// * `value` - The tag value (e.g., "rust", "Joshua Jay")
    /// * `page` - The tagged page information
    pub fn add_page(&self, source: &str, value: &str, page: TaggedPage) {
        let norm_source = Self::normalize_source(source);
        let norm_value = Self::normalize_value(value);
        let key = Self::make_key(&norm_source, &norm_value);

        // Track the source (first occurrence wins for display)
        let sources_guard = self.sources.pin();
        if sources_guard.get(&norm_source).is_none() {
            sources_guard.insert(norm_source.clone(), source.to_string());
        }

        // Track display value (first occurrence wins)
        let display_guard = self.display_values.pin();
        if display_guard.get(&key).is_none() {
            display_guard.insert(key.clone(), value.to_string());
        }

        // Add page to the index
        let guard = self.index.pin();
        let mut pages = guard.get(&key).cloned().unwrap_or_default();

        // Avoid duplicate pages (same url_path)
        if !pages.iter().any(|p| p.url_path == page.url_path) {
            pages.push(page);
            guard.insert(key, pages);
        }
    }

    /// Gets all pages tagged with the given source and value.
    ///
    /// Returns an empty vector if no pages have this tag.
    pub fn get_pages(&self, source: &str, value: &str) -> Vec<TaggedPage> {
        let norm_source = Self::normalize_source(source);
        let norm_value = Self::normalize_value(value);
        let key = Self::make_key(&norm_source, &norm_value);

        self.index.pin().get(&key).cloned().unwrap_or_default()
    }

    /// Gets all unique tags for a given source.
    ///
    /// Returns a vector of TagInfo with normalized key, display value, and count.
    pub fn get_all_tags(&self, source: &str) -> Vec<TagInfo> {
        let norm_source = Self::normalize_source(source);
        let prefix = format!("{}:", norm_source);

        let guard = self.index.pin();
        let display_guard = self.display_values.pin();

        let mut tags: Vec<TagInfo> = guard
            .iter()
            .filter(|(k, _)| k.starts_with(&prefix))
            .map(|(key, pages)| {
                let norm_value = key.strip_prefix(&prefix).unwrap_or(key).to_string();
                let display = display_guard
                    .get(key)
                    .cloned()
                    .unwrap_or_else(|| norm_value.clone());
                TagInfo {
                    normalized: norm_value,
                    display,
                    count: pages.len(),
                }
            })
            .collect();

        // Sort by display name (case-insensitive)
        tags.sort_by(|a, b| a.display.to_lowercase().cmp(&b.display.to_lowercase()));

        tags
    }

    /// Gets all sources that have at least one tag.
    ///
    /// Returns a set of normalized source names.
    pub fn get_all_sources(&self) -> HashSet<String> {
        self.sources.pin().iter().map(|(k, _)| k.clone()).collect()
    }

    /// Gets the display name for a source.
    pub fn get_source_display(&self, normalized_source: &str) -> Option<String> {
        self.sources.pin().get(normalized_source).cloned()
    }

    /// Gets the display name for a tag value.
    pub fn get_tag_display(&self, source: &str, value: &str) -> Option<String> {
        let norm_source = Self::normalize_source(source);
        let norm_value = Self::normalize_value(value);
        let key = Self::make_key(&norm_source, &norm_value);

        self.display_values.pin().get(&key).cloned()
    }

    /// Checks if a tag exists in the index.
    pub fn has_tag(&self, source: &str, value: &str) -> bool {
        let norm_source = Self::normalize_source(source);
        let norm_value = Self::normalize_value(value);
        let key = Self::make_key(&norm_source, &norm_value);

        self.index.pin().get(&key).is_some()
    }

    /// Checks if a source has any tags.
    pub fn has_source(&self, source: &str) -> bool {
        let norm_source = Self::normalize_source(source);
        self.sources.pin().get(&norm_source).is_some()
    }

    /// Returns the total number of unique tags across all sources.
    pub fn total_tags(&self) -> usize {
        self.index.pin().len()
    }

    /// Returns the total number of sources.
    pub fn total_sources(&self) -> usize {
        self.sources.pin().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_source() {
        assert_eq!(TagIndex::normalize_source("Tags"), "tags");
        assert_eq!(TagIndex::normalize_source("PERFORMERS"), "performers");
        assert_eq!(TagIndex::normalize_source("taxonomy.tags"), "taxonomy.tags");
    }

    #[test]
    fn test_normalize_value() {
        assert_eq!(TagIndex::normalize_value("rust"), "rust");
        assert_eq!(TagIndex::normalize_value("Rust"), "rust");
        assert_eq!(TagIndex::normalize_value("Joshua Jay"), "joshua_jay");
    }

    #[test]
    fn test_add_and_get_page() {
        let index = TagIndex::new();

        let page = TaggedPage::new("/docs/rust-guide/", "Rust Guide", "rust");
        index.add_page("tags", "rust", page.clone());

        let pages = index.get_pages("tags", "rust");
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].url_path, "/docs/rust-guide/");
    }

    #[test]
    fn test_case_insensitive_source() {
        let index = TagIndex::new();

        let page = TaggedPage::new("/page/", "Page", "rust");
        index.add_page("Tags", "rust", page);

        // All these should find the same tag
        assert_eq!(index.get_pages("tags", "rust").len(), 1);
        assert_eq!(index.get_pages("Tags", "rust").len(), 1);
        assert_eq!(index.get_pages("TAGS", "rust").len(), 1);
    }

    #[test]
    fn test_case_insensitive_value() {
        let index = TagIndex::new();

        let page = TaggedPage::new("/page/", "Page", "Rust");
        index.add_page("tags", "Rust", page);

        // All these should find the same tag
        assert_eq!(index.get_pages("tags", "rust").len(), 1);
        assert_eq!(index.get_pages("tags", "Rust").len(), 1);
        assert_eq!(index.get_pages("tags", "RUST").len(), 1);
    }

    #[test]
    fn test_value_with_spaces() {
        let index = TagIndex::new();

        let page = TaggedPage::new("/performer/", "Page", "Joshua Jay");
        index.add_page("performers", "Joshua Jay", page);

        // Can be found by normalized or original form
        assert_eq!(index.get_pages("performers", "Joshua Jay").len(), 1);
        assert_eq!(index.get_pages("performers", "joshua_jay").len(), 1);
        assert_eq!(index.get_pages("performers", "joshua jay").len(), 1);
    }

    #[test]
    fn test_multiple_pages_same_tag() {
        let index = TagIndex::new();

        let page1 = TaggedPage::new("/page1/", "Page 1", "rust");
        let page2 = TaggedPage::new("/page2/", "Page 2", "Rust"); // Different case

        index.add_page("tags", "rust", page1);
        index.add_page("tags", "Rust", page2); // Should go to same tag

        let pages = index.get_pages("tags", "rust");
        assert_eq!(pages.len(), 2);
    }

    #[test]
    fn test_no_duplicate_pages() {
        let index = TagIndex::new();

        let page = TaggedPage::new("/page/", "Page", "rust");

        index.add_page("tags", "rust", page.clone());
        index.add_page("tags", "rust", page.clone()); // Same page, should not duplicate

        let pages = index.get_pages("tags", "rust");
        assert_eq!(pages.len(), 1);
    }

    #[test]
    fn test_get_all_tags() {
        let index = TagIndex::new();

        index.add_page("tags", "rust", TaggedPage::new("/p1/", "P1", "rust"));
        index.add_page("tags", "Python", TaggedPage::new("/p2/", "P2", "Python"));
        index.add_page("tags", "go", TaggedPage::new("/p3/", "P3", "go"));
        index.add_page("tags", "rust", TaggedPage::new("/p4/", "P4", "Rust")); // Another rust page

        let tags = index.get_all_tags("tags");
        assert_eq!(tags.len(), 3);

        // Find the rust tag
        let rust_tag = tags.iter().find(|t| t.normalized == "rust").unwrap();
        assert_eq!(rust_tag.count, 2);
        assert_eq!(rust_tag.display, "rust"); // First occurrence
    }

    #[test]
    fn test_get_all_sources() {
        let index = TagIndex::new();

        index.add_page("tags", "rust", TaggedPage::new("/p1/", "P1", "rust"));
        index.add_page(
            "performers",
            "Joshua Jay",
            TaggedPage::new("/p2/", "P2", "Joshua Jay"),
        );

        let sources = index.get_all_sources();
        assert_eq!(sources.len(), 2);
        assert!(sources.contains("tags"));
        assert!(sources.contains("performers"));
    }

    #[test]
    fn test_source_display() {
        let index = TagIndex::new();

        index.add_page("Tags", "rust", TaggedPage::new("/p/", "P", "rust"));

        let display = index.get_source_display("tags");
        assert_eq!(display, Some("Tags".to_string()));
    }

    #[test]
    fn test_tag_display() {
        let index = TagIndex::new();

        index.add_page(
            "performers",
            "Joshua Jay",
            TaggedPage::new("/p/", "P", "Joshua Jay"),
        );

        let display = index.get_tag_display("performers", "joshua_jay");
        assert_eq!(display, Some("Joshua Jay".to_string()));
    }

    #[test]
    fn test_has_tag() {
        let index = TagIndex::new();

        index.add_page("tags", "rust", TaggedPage::new("/p/", "P", "rust"));

        assert!(index.has_tag("tags", "rust"));
        assert!(index.has_tag("Tags", "Rust")); // Case insensitive
        assert!(!index.has_tag("tags", "python"));
        assert!(!index.has_tag("category", "rust"));
    }

    #[test]
    fn test_has_source() {
        let index = TagIndex::new();

        index.add_page("tags", "rust", TaggedPage::new("/p/", "P", "rust"));

        assert!(index.has_source("tags"));
        assert!(index.has_source("Tags")); // Case insensitive
        assert!(!index.has_source("performers"));
    }

    #[test]
    fn test_totals() {
        let index = TagIndex::new();

        assert_eq!(index.total_tags(), 0);
        assert_eq!(index.total_sources(), 0);

        index.add_page("tags", "rust", TaggedPage::new("/p1/", "P1", "rust"));
        index.add_page("tags", "python", TaggedPage::new("/p2/", "P2", "python"));
        index.add_page(
            "performers",
            "Joshua Jay",
            TaggedPage::new("/p3/", "P3", "Joshua Jay"),
        );

        assert_eq!(index.total_tags(), 3);
        assert_eq!(index.total_sources(), 2);
    }

    #[test]
    fn test_empty_results() {
        let index = TagIndex::new();

        let pages = index.get_pages("tags", "nonexistent");
        assert!(pages.is_empty());

        let tags = index.get_all_tags("nonexistent");
        assert!(tags.is_empty());
    }

    #[test]
    fn test_tagged_page_with_description() {
        let page =
            TaggedPage::with_description("/page/", "Page Title", "This is a description", "rust");

        assert_eq!(page.title, "Page Title");
        assert_eq!(page.description, Some("This is a description".to_string()));
    }
}
