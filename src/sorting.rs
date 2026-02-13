//! Configurable file sorting for mbr.
//!
//! Provides multi-level sorting by any field (title, filename, date, frontmatter),
//! with configurable order (ascending/descending) and comparison type (string/numeric).
//!
//! Uses a Schwartzian transform (decorate-sort-undecorate) to pre-extract sort keys,
//! avoiding repeated field lookups and string allocations during comparisons.

use crate::config::SortField;
use serde_json::Value;
use std::cmp::Ordering;

/// A pre-extracted sort key for a single field of a single file.
///
/// Keys are extracted once before sorting begins, eliminating per-comparison allocations.
#[derive(Debug, Clone, PartialEq)]
enum SortKey {
    /// Field value is missing — sorts AFTER all present values regardless of sort direction.
    Missing,
    /// Numeric value (parsed from string).
    Numeric(f64),
    /// String value (pre-lowercased for case-insensitive comparison).
    Text(String),
}

impl SortKey {
    /// Compares two sort keys for a single field.
    ///
    /// Missing values always sort after present values (regardless of sort order).
    /// The `reverse` flag only applies to present-vs-present comparisons.
    fn cmp_with_direction(&self, other: &SortKey, reverse: bool) -> Ordering {
        match (self, other) {
            (SortKey::Missing, SortKey::Missing) => Ordering::Equal,
            (SortKey::Missing, _) => Ordering::Greater,
            (_, SortKey::Missing) => Ordering::Less,
            (SortKey::Numeric(a), SortKey::Numeric(b)) => {
                let cmp = a.partial_cmp(b).unwrap_or(Ordering::Equal);
                if reverse { cmp.reverse() } else { cmp }
            }
            (SortKey::Text(a), SortKey::Text(b)) => {
                let cmp = a.cmp(b);
                if reverse { cmp.reverse() } else { cmp }
            }
            // Should not happen in practice (mixed types for same field),
            // but handle gracefully by treating as equal.
            _ => Ordering::Equal,
        }
    }
}

/// Parsed sort direction to avoid repeated string comparisons.
#[derive(Debug, Clone, Copy, PartialEq)]
enum SortOrder {
    Asc,
    Desc,
}

/// Parsed comparison type to avoid repeated string comparisons.
#[derive(Debug, Clone, Copy, PartialEq)]
enum SortCompare {
    String,
    Numeric,
}

/// Sorts files in place according to the given sort configuration.
///
/// Uses a Schwartzian transform to minimize allocations:
/// 1. **Decorate**: Extract all sort keys for all items (O(n * k) allocations, done once)
/// 2. **Sort**: Sort an index array by comparing pre-computed keys (zero allocations)
/// 3. **Undecorate**: Apply the permutation in-place
///
/// # Special field names
/// - `"title"` - Uses frontmatter title, falls back to filename without extension
/// - `"filename"` - Uses raw filename
/// - `"created"` - Uses created timestamp
/// - `"modified"` - Uses modified timestamp
/// - Any other string - Looks up frontmatter field
///
/// # Missing value behavior
/// Files missing a sort field sort AFTER files that have it.
/// This enables patterns like "pinned" (files with pinned:true first) or
/// "order" (files with explicit order first, then others).
pub fn sort_files(files: &mut [Value], sort_config: &[SortField]) {
    if sort_config.is_empty() || files.len() <= 1 {
        return;
    }

    // Parse sort configuration once
    let parsed_config: Vec<(SortOrder, SortCompare)> = sort_config
        .iter()
        .map(|sf| {
            let order = if sf.order == "desc" {
                SortOrder::Desc
            } else {
                SortOrder::Asc
            };
            let compare = if sf.compare == "numeric" {
                SortCompare::Numeric
            } else {
                SortCompare::String
            };
            (order, compare)
        })
        .collect();

    // Step 1: Decorate — extract all sort keys upfront
    let keys: Vec<Vec<SortKey>> = files
        .iter()
        .map(|file| {
            sort_config
                .iter()
                .zip(parsed_config.iter())
                .map(|(sf, &(_, compare))| extract_sort_key(file, &sf.field, compare))
                .collect()
        })
        .collect();

    // Step 2: Sort — build index array and sort by pre-computed keys
    let mut indices: Vec<usize> = (0..files.len()).collect();
    indices.sort_by(|&a, &b| {
        for (field_idx, &(order, _)) in parsed_config.iter().enumerate() {
            let reverse = order == SortOrder::Desc;
            let cmp = keys[a][field_idx].cmp_with_direction(&keys[b][field_idx], reverse);
            if cmp != Ordering::Equal {
                return cmp;
            }
        }
        Ordering::Equal
    });

    // Step 3: Undecorate — apply permutation in-place using cycle-chase
    apply_permutation(files, &mut indices);
}

/// Applies a permutation to a slice in-place using cycle-chase algorithm.
///
/// After this function, `data[i]` will contain the element that was originally
/// at position `perm[i]`. The `perm` array is consumed (modified) during the process.
fn apply_permutation(data: &mut [Value], perm: &mut [usize]) {
    for i in 0..data.len() {
        // Follow the cycle starting at position i
        if perm[i] == i {
            continue;
        }
        let mut current = i;
        loop {
            let target = perm[current];
            perm[current] = current; // Mark as placed
            if target == i {
                break;
            }
            data.swap(current, target);
            current = target;
        }
    }
}

/// Extracts a sort key from a file JSON object for a given field.
fn extract_sort_key(file: &Value, field: &str, compare: SortCompare) -> SortKey {
    match get_field_value(file, field) {
        None => SortKey::Missing,
        Some(s) => match compare {
            SortCompare::Numeric => {
                let num: f64 = s.parse().unwrap_or(0.0);
                SortKey::Numeric(num)
            }
            SortCompare::String => SortKey::Text(s.to_lowercase()),
        },
    }
}

/// Extracts a field value from a file JSON object.
///
/// # Special fields
/// - `"title"` - frontmatter.title, fallback to name (filename)
/// - `"filename"` - raw name field
/// - `"created"` - created timestamp as string
/// - `"modified"` - modified timestamp as string
/// - Any other field - frontmatter lookup
///
/// # Boolean handling
/// Boolean values are converted to "1" (true) or "0" (false) for sorting.
fn get_field_value(file: &Value, field: &str) -> Option<String> {
    match field {
        "title" => {
            // Try frontmatter.title first, then fall back to name (filename without extension)
            file.get("title")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .or_else(|| {
                    file.get("frontmatter")
                        .and_then(|fm| fm.get("title"))
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                })
                .or_else(|| get_filename_without_ext(file))
        }
        "filename" => file
            .get("name")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        "created" => file
            .get("created")
            .and_then(|v| v.as_u64())
            .map(|n| n.to_string()),
        "modified" => file
            .get("modified")
            .and_then(|v| v.as_u64())
            .map(|n| n.to_string()),
        // Look up any other field in frontmatter
        _ => get_frontmatter_field(file, field),
    }
}

/// Gets a field from the frontmatter object, handling various types.
fn get_frontmatter_field(file: &Value, field: &str) -> Option<String> {
    let fm = file.get("frontmatter")?;
    let value = fm.get(field)?;

    match value {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(if *b { "1".to_string() } else { "0".to_string() }),
        _ => None,
    }
}

/// Extracts filename without extension from the name field.
fn get_filename_without_ext(file: &Value) -> Option<String> {
    file.get("name").and_then(|v| v.as_str()).map(|name| {
        // Remove .md extension if present
        name.strip_suffix(".md").unwrap_or(name).to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_file(name: &str, title: Option<&str>, order: Option<i64>) -> Value {
        let mut frontmatter = serde_json::Map::new();
        if let Some(t) = title {
            frontmatter.insert("title".to_string(), json!(t));
        }
        if let Some(o) = order {
            frontmatter.insert("order".to_string(), json!(o));
        }

        json!({
            "name": name,
            "title": title,
            "created": 1000,
            "modified": 2000,
            "frontmatter": frontmatter
        })
    }

    fn make_file_with_pinned(name: &str, title: &str, pinned: Option<bool>) -> Value {
        let mut frontmatter = serde_json::Map::new();
        frontmatter.insert("title".to_string(), json!(title));
        if let Some(p) = pinned {
            frontmatter.insert("pinned".to_string(), json!(p));
        }

        json!({
            "name": name,
            "title": title,
            "created": 1000,
            "modified": 2000,
            "frontmatter": frontmatter
        })
    }

    #[test]
    fn test_default_sort_by_title_ascending() {
        let mut files = vec![
            make_file("zebra.md", Some("Zebra"), None),
            make_file("apple.md", Some("Apple"), None),
            make_file("mango.md", Some("Mango"), None),
        ];

        let config = vec![SortField {
            field: "title".to_string(),
            order: "asc".to_string(),
            compare: "string".to_string(),
        }];

        sort_files(&mut files, &config);

        assert_eq!(files[0]["title"], "Apple");
        assert_eq!(files[1]["title"], "Mango");
        assert_eq!(files[2]["title"], "Zebra");
    }

    #[test]
    fn test_sort_by_title_descending() {
        let mut files = vec![
            make_file("apple.md", Some("Apple"), None),
            make_file("zebra.md", Some("Zebra"), None),
            make_file("mango.md", Some("Mango"), None),
        ];

        let config = vec![SortField {
            field: "title".to_string(),
            order: "desc".to_string(),
            compare: "string".to_string(),
        }];

        sort_files(&mut files, &config);

        assert_eq!(files[0]["title"], "Zebra");
        assert_eq!(files[1]["title"], "Mango");
        assert_eq!(files[2]["title"], "Apple");
    }

    #[test]
    fn test_sort_by_numeric_order() {
        let mut files = vec![
            make_file("third.md", Some("Third"), Some(3)),
            make_file("first.md", Some("First"), Some(1)),
            make_file("second.md", Some("Second"), Some(2)),
        ];

        let config = vec![SortField {
            field: "order".to_string(),
            order: "asc".to_string(),
            compare: "numeric".to_string(),
        }];

        sort_files(&mut files, &config);

        assert_eq!(files[0]["title"], "First");
        assert_eq!(files[1]["title"], "Second");
        assert_eq!(files[2]["title"], "Third");
    }

    #[test]
    fn test_missing_field_sorts_after() {
        let mut files = vec![
            make_file("no_order.md", Some("No Order"), None), // No order field
            make_file("first.md", Some("First"), Some(1)),
            make_file("second.md", Some("Second"), Some(2)),
        ];

        let config = vec![SortField {
            field: "order".to_string(),
            order: "asc".to_string(),
            compare: "numeric".to_string(),
        }];

        sort_files(&mut files, &config);

        // Files with order come first (sorted), then files without
        assert_eq!(files[0]["title"], "First");
        assert_eq!(files[1]["title"], "Second");
        assert_eq!(files[2]["title"], "No Order");
    }

    #[test]
    fn test_multi_level_sort() {
        let mut files = vec![
            make_file("c.md", Some("C"), Some(1)),
            make_file("a.md", Some("A"), Some(2)),
            make_file("b.md", Some("B"), Some(1)),
            make_file("d.md", Some("D"), Some(2)),
        ];

        // Sort by order first, then by title
        let config = vec![
            SortField {
                field: "order".to_string(),
                order: "asc".to_string(),
                compare: "numeric".to_string(),
            },
            SortField {
                field: "title".to_string(),
                order: "asc".to_string(),
                compare: "string".to_string(),
            },
        ];

        sort_files(&mut files, &config);

        // Order 1: B, C (sorted by title)
        // Order 2: A, D (sorted by title)
        assert_eq!(files[0]["title"], "B");
        assert_eq!(files[1]["title"], "C");
        assert_eq!(files[2]["title"], "A");
        assert_eq!(files[3]["title"], "D");
    }

    #[test]
    fn test_pinned_pattern() {
        let mut files = vec![
            make_file_with_pinned("normal1.md", "Normal 1", None),
            make_file_with_pinned("pinned1.md", "Pinned 1", Some(true)),
            make_file_with_pinned("normal2.md", "Normal 2", None),
            make_file_with_pinned("unpinned.md", "Unpinned", Some(false)),
        ];

        // Sort by pinned descending (true=1 first), then by title
        let config = vec![
            SortField {
                field: "pinned".to_string(),
                order: "desc".to_string(),
                compare: "numeric".to_string(),
            },
            SortField {
                field: "title".to_string(),
                order: "asc".to_string(),
                compare: "string".to_string(),
            },
        ];

        sort_files(&mut files, &config);

        // Pinned:true first, then pinned:false, then files without pinned field
        assert_eq!(files[0]["title"], "Pinned 1");
        assert_eq!(files[1]["title"], "Unpinned"); // pinned: false = "0"
        // Files without pinned come last, sorted by title
        assert_eq!(files[2]["title"], "Normal 1");
        assert_eq!(files[3]["title"], "Normal 2");
    }

    #[test]
    fn test_title_falls_back_to_filename() {
        let mut files = vec![
            make_file("zebra.md", None, None), // No title, use filename
            make_file("apple.md", Some("Apple"), None),
        ];

        let config = vec![SortField {
            field: "title".to_string(),
            order: "asc".to_string(),
            compare: "string".to_string(),
        }];

        sort_files(&mut files, &config);

        assert_eq!(files[0]["title"], "Apple");
        // The second file has no title, fell back to "zebra"
        assert!(files[1]["title"].is_null());
        assert_eq!(files[1]["name"], "zebra.md");
    }

    #[test]
    fn test_empty_config_preserves_order() {
        let mut files = vec![
            make_file("c.md", Some("C"), None),
            make_file("a.md", Some("A"), None),
            make_file("b.md", Some("B"), None),
        ];

        let config: Vec<SortField> = vec![];

        sort_files(&mut files, &config);

        // Order preserved
        assert_eq!(files[0]["title"], "C");
        assert_eq!(files[1]["title"], "A");
        assert_eq!(files[2]["title"], "B");
    }

    #[test]
    fn test_case_insensitive_string_sort() {
        let mut files = vec![
            make_file("b.md", Some("Banana"), None),
            make_file("a.md", Some("apple"), None), // lowercase
            make_file("c.md", Some("Cherry"), None),
        ];

        let config = vec![SortField {
            field: "title".to_string(),
            order: "asc".to_string(),
            compare: "string".to_string(),
        }];

        sort_files(&mut files, &config);

        assert_eq!(files[0]["title"], "apple");
        assert_eq!(files[1]["title"], "Banana");
        assert_eq!(files[2]["title"], "Cherry");
    }

    #[test]
    fn test_sort_by_modified_descending() {
        let mut files = vec![
            json!({
                "name": "old.md",
                "title": "Old",
                "modified": 1000,
                "frontmatter": {}
            }),
            json!({
                "name": "new.md",
                "title": "New",
                "modified": 3000,
                "frontmatter": {}
            }),
            json!({
                "name": "middle.md",
                "title": "Middle",
                "modified": 2000,
                "frontmatter": {}
            }),
        ];

        let config = vec![SortField {
            field: "modified".to_string(),
            order: "desc".to_string(),
            compare: "numeric".to_string(),
        }];

        sort_files(&mut files, &config);

        assert_eq!(files[0]["title"], "New");
        assert_eq!(files[1]["title"], "Middle");
        assert_eq!(files[2]["title"], "Old");
    }

    #[test]
    fn test_single_element_preserves() {
        let mut files = vec![make_file("a.md", Some("A"), None)];
        let config = vec![SortField::default()];
        sort_files(&mut files, &config);
        assert_eq!(files[0]["title"], "A");
    }

    #[test]
    fn test_apply_permutation_identity() {
        let mut data = vec![json!("a"), json!("b"), json!("c")];
        let mut perm = vec![0, 1, 2];
        apply_permutation(&mut data, &mut perm);
        assert_eq!(data, vec![json!("a"), json!("b"), json!("c")]);
    }

    #[test]
    fn test_apply_permutation_reverse() {
        let mut data = vec![json!("a"), json!("b"), json!("c")];
        let mut perm = vec![2, 1, 0];
        apply_permutation(&mut data, &mut perm);
        assert_eq!(data, vec![json!("c"), json!("b"), json!("a")]);
    }

    #[test]
    fn test_apply_permutation_cycle() {
        let mut data = vec![json!("a"), json!("b"), json!("c"), json!("d")];
        let mut perm = vec![1, 2, 3, 0]; // rotate left
        apply_permutation(&mut data, &mut perm);
        assert_eq!(data, vec![json!("b"), json!("c"), json!("d"), json!("a")]);
    }
}
