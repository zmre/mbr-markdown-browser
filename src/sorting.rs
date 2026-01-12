//! Configurable file sorting for mbr.
//!
//! Provides multi-level sorting by any field (title, filename, date, frontmatter),
//! with configurable order (ascending/descending) and comparison type (string/numeric).

use crate::config::SortField;
use serde_json::Value;
use std::cmp::Ordering;

/// Sorts files in place according to the given sort configuration.
///
/// Files are sorted by each field in order. When two files are equal
/// on one sort field, the next field is used to break the tie.
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
    if sort_config.is_empty() {
        return;
    }

    files.sort_by(|a, b| {
        for sort_field in sort_config {
            let cmp = compare_by_field(a, b, sort_field);
            if cmp != Ordering::Equal {
                return cmp;
            }
        }
        Ordering::Equal
    });
}

/// Compares two file objects by a single sort field configuration.
fn compare_by_field(a: &Value, b: &Value, config: &SortField) -> Ordering {
    let val_a = get_field_value(a, &config.field);
    let val_b = get_field_value(b, &config.field);

    // Handle missing values: files without field sort AFTER files with it
    // Note: Missing value handling is NOT affected by sort direction
    match (&val_a, &val_b) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater, // a missing → a comes after
        (Some(_), None) => Ordering::Less,    // b missing → a comes before
        (Some(a_str), Some(b_str)) => {
            // Only the value comparison is affected by sort direction
            let cmp = if config.compare == "numeric" {
                compare_numeric(a_str, b_str)
            } else {
                a_str.to_lowercase().cmp(&b_str.to_lowercase())
            };

            if config.order == "desc" {
                cmp.reverse()
            } else {
                cmp
            }
        }
    }
}

/// Compares two strings as numbers.
/// Non-numeric strings are treated as 0.
fn compare_numeric(a: &str, b: &str) -> Ordering {
    let num_a: f64 = a.parse().unwrap_or(0.0);
    let num_b: f64 = b.parse().unwrap_or(0.0);
    num_a.partial_cmp(&num_b).unwrap_or(Ordering::Equal)
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
}
