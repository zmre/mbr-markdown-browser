//! Pure, lexical URL helpers for page chrome: breadcrumbs, the "up" link, and
//! the relative prefixes a static page uses to reach `.mbr/` and the site root.
//!
//! Shared by the server (live pages), the static build and
//! [`crate::page_context`]. Nothing here touches the filesystem: inputs are
//! repository-relative paths or page depths, and every URL produced is
//! `/`-separated whatever the host platform.

use std::path::Path;

/// A breadcrumb entry for navigation.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Breadcrumb {
    pub name: String,
    pub url: String,
}

impl Breadcrumb {
    pub fn new(name: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            url: url.into(),
        }
    }
}

/// Generates breadcrumb navigation from a relative path.
///
/// Always starts with "Home" → "/" and includes all path components.
/// The last component is not included in the returned breadcrumbs (it's the current page).
pub fn generate_breadcrumbs(relative_path: &Path) -> Vec<Breadcrumb> {
    let path_components: Vec<_> = relative_path
        .components()
        .filter_map(|c| {
            if let std::path::Component::Normal(s) = c {
                s.to_str()
            } else {
                None
            }
        })
        .collect();

    // For root (no path components), return empty breadcrumbs
    // The current page name will be shown separately, avoiding "Home > Home"
    if path_components.is_empty() {
        return vec![];
    }

    // Start with Home
    let mut breadcrumbs = vec![Breadcrumb::new("Home", "/")];

    // Add all but the last component (last is current page/directory)
    for (idx, _) in path_components
        .iter()
        .enumerate()
        .take(path_components.len().saturating_sub(1))
    {
        // Join the already-extracted `&str` components directly: routing them
        // back through a `PathBuf` would reintroduce the platform separator.
        let url = format!("/{}/", path_components[..=idx].join("/"));
        let name = path_components[idx].to_string();
        breadcrumbs.push(Breadcrumb::new(name, url));
    }

    breadcrumbs
}

/// Gets the current directory name from a relative path.
pub fn get_current_dir_name(relative_path: &Path) -> String {
    relative_path
        .file_name()
        .and_then(|s| s.to_str())
        .map(String::from)
        .unwrap_or_else(|| "Home".to_string())
}

/// Gets the parent path URL for "up" navigation.
pub fn get_parent_path(relative_path: &Path) -> Option<String> {
    let path_components: Vec<_> = relative_path
        .components()
        .filter_map(|c| {
            if let std::path::Component::Normal(s) = c {
                s.to_str()
            } else {
                None
            }
        })
        .collect();

    if path_components.len() > 1 {
        // Join the `&str` components directly rather than via `PathBuf`, which
        // would use `\` on Windows.
        let parent = path_components[..path_components.len() - 1].join("/");
        Some(format!("/{}/", parent))
    } else if !path_components.is_empty() {
        Some("/".to_string())
    } else {
        None
    }
}

/// Build the relative path prefix for .mbr assets based on page depth.
///
/// Examples:
/// - depth 0 → ".mbr/"
/// - depth 1 → "../.mbr/"
/// - depth 2 → "../../.mbr/"
pub fn relative_base(depth: usize) -> String {
    if depth == 0 {
        ".mbr/".to_string()
    } else {
        format!("{}.mbr/", "../".repeat(depth))
    }
}

/// Build the relative path prefix to root based on page depth.
///
/// Examples:
/// - depth 0 → "" (empty string, already at root)
/// - depth 1 → "../"
/// - depth 2 → "../../"
pub fn relative_root(depth: usize) -> String {
    if depth == 0 {
        String::new()
    } else {
        "../".repeat(depth)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_breadcrumbs_root() {
        let path = Path::new("");
        let breadcrumbs = generate_breadcrumbs(path);

        // Root returns empty breadcrumbs to avoid "Home > Home" duplication
        // The template handles showing just "Home" as the current page
        assert_eq!(breadcrumbs.len(), 0);
    }

    #[test]
    fn test_generate_breadcrumbs_single_level() {
        let path = Path::new("docs");
        let breadcrumbs = generate_breadcrumbs(path);

        // Home only - "docs" is the current directory, not shown in breadcrumbs
        assert_eq!(breadcrumbs.len(), 1);
        assert_eq!(breadcrumbs[0], Breadcrumb::new("Home", "/"));
    }

    #[test]
    fn test_generate_breadcrumbs_two_levels() {
        let path = Path::new("docs/api");
        let breadcrumbs = generate_breadcrumbs(path);

        assert_eq!(breadcrumbs.len(), 2);
        assert_eq!(breadcrumbs[0], Breadcrumb::new("Home", "/"));
        assert_eq!(breadcrumbs[1], Breadcrumb::new("docs", "/docs/"));
    }

    #[test]
    fn test_generate_breadcrumbs_deep_nesting() {
        let path = Path::new("/a/b/c/d");
        let breadcrumbs = generate_breadcrumbs(path);

        assert_eq!(breadcrumbs.len(), 4);
        assert_eq!(breadcrumbs[0], Breadcrumb::new("Home", "/"));
        assert_eq!(breadcrumbs[1], Breadcrumb::new("a", "/a/"));
        assert_eq!(breadcrumbs[2], Breadcrumb::new("b", "/a/b/"));
        assert_eq!(breadcrumbs[3], Breadcrumb::new("c", "/a/b/c/"));
    }

    #[test]
    fn test_get_current_dir_name_root() {
        let path = Path::new("");
        assert_eq!(get_current_dir_name(path), "Home");
    }

    #[test]
    fn test_get_current_dir_name_single_level() {
        let path = Path::new("docs");
        assert_eq!(get_current_dir_name(path), "docs");
    }

    #[test]
    fn test_get_current_dir_name_nested() {
        let path = Path::new("a/b/c");
        assert_eq!(get_current_dir_name(path), "c");
    }

    #[test]
    fn test_get_parent_path_root() {
        let path = Path::new("");
        assert_eq!(get_parent_path(path), None);
    }

    #[test]
    fn test_get_parent_path_single_level() {
        let path = Path::new("docs");
        assert_eq!(get_parent_path(path), Some("/".to_string()));
    }

    #[test]
    fn test_get_parent_path_two_levels() {
        let path = Path::new("docs/api");
        assert_eq!(get_parent_path(path), Some("/docs/".to_string()));
    }

    #[test]
    fn test_get_parent_path_deep() {
        let path = Path::new("a/b/c/d");
        assert_eq!(get_parent_path(path), Some("/a/b/c/".to_string()));
    }

    #[test]
    fn test_breadcrumb_equality() {
        let b1 = Breadcrumb::new("Home", "/");
        let b2 = Breadcrumb::new("Home", "/");
        let b3 = Breadcrumb::new("Docs", "/docs/");

        assert_eq!(b1, b2);
        assert_ne!(b1, b3);
    }

    #[test]
    fn test_relative_base_at_root() {
        assert_eq!(relative_base(0), ".mbr/");
    }

    #[test]
    fn test_relative_base_one_level() {
        assert_eq!(relative_base(1), "../.mbr/");
    }

    #[test]
    fn test_relative_base_multiple_levels() {
        assert_eq!(relative_base(2), "../../.mbr/");
        assert_eq!(relative_base(3), "../../../.mbr/");
    }

    #[test]
    fn test_relative_root_at_root() {
        assert_eq!(relative_root(0), "");
    }

    #[test]
    fn test_relative_root_one_level() {
        assert_eq!(relative_root(1), "../");
    }

    #[test]
    fn test_relative_root_multiple_levels() {
        assert_eq!(relative_root(2), "../../");
        assert_eq!(relative_root(3), "../../../");
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    // Strategy for valid path component names
    fn path_component_strategy() -> impl Strategy<Value = String> {
        "[a-zA-Z0-9_-]{1,15}"
    }

    proptest! {
        /// Breadcrumb count: Home + all components except the last (current dir)
        /// For 0 components: [] = 0 (root page, no breadcrumbs to avoid "Home > Home")
        /// For 1 component: [Home] = 1 (last component is current dir, not a link)
        /// For 2+ components: [Home, c1, c2, ...] = components.len()
        #[test]
        fn prop_breadcrumb_count_matches_path_depth(
            components in proptest::collection::vec(path_component_strategy(), 0..5)
        ) {
            let path_str = components.join("/");
            let path = Path::new(&path_str);
            let breadcrumbs = generate_breadcrumbs(path);

            // Breadcrumbs = "Home" + all components except the last (which is current dir)
            // For empty path (root), return empty to avoid "Home > Home"
            let expected_count = if components.is_empty() {
                0  // Empty for root page
            } else {
                components.len()  // Home + all but last = components.len()
            };
            prop_assert_eq!(
                breadcrumbs.len(),
                expected_count,
                "Path {:?} should have {} breadcrumbs, got {}",
                path,
                expected_count,
                breadcrumbs.len()
            );
        }

        /// For non-empty paths, first breadcrumb is always "Home" with url "/"
        #[test]
        fn prop_first_breadcrumb_is_home(
            components in proptest::collection::vec(path_component_strategy(), 1..5)
        ) {
            let path_str = components.join("/");
            let path = Path::new(&path_str);
            let breadcrumbs = generate_breadcrumbs(path);

            prop_assert!(!breadcrumbs.is_empty(), "Non-root paths should have at least Home breadcrumb");
            prop_assert_eq!(&breadcrumbs[0].name, "Home");
            prop_assert_eq!(&breadcrumbs[0].url, "/");
        }

        /// For 2+ components, last breadcrumb is second-to-last path component
        #[test]
        fn prop_last_breadcrumb_matches_parent_component(
            components in proptest::collection::vec(path_component_strategy(), 2..5)
        ) {
            let path_str = components.join("/");
            let path = Path::new(&path_str);
            let breadcrumbs = generate_breadcrumbs(path);

            let last_breadcrumb = breadcrumbs.last().unwrap();
            // The second-to-last component is the parent dir
            let parent_component = &components[components.len() - 2];
            prop_assert_eq!(
                &last_breadcrumb.name,
                parent_component,
                "Last breadcrumb should be {:?}, got {:?}",
                parent_component,
                last_breadcrumb.name
            );
        }

        /// All breadcrumb URLs end with /
        #[test]
        fn prop_breadcrumb_urls_end_with_slash(
            components in proptest::collection::vec(path_component_strategy(), 0..5)
        ) {
            let path_str = components.join("/");
            let path = Path::new(&path_str);
            let breadcrumbs = generate_breadcrumbs(path);

            for bc in &breadcrumbs {
                prop_assert!(
                    bc.url.ends_with('/'),
                    "Breadcrumb URL {:?} should end with /",
                    bc.url
                );
            }
        }

        /// get_current_dir_name returns the last path component
        #[test]
        fn prop_current_dir_name_is_last_component(
            components in proptest::collection::vec(path_component_strategy(), 1..5)
        ) {
            let path_str = components.join("/");
            let path = Path::new(&path_str);
            let name = get_current_dir_name(path);

            let expected = components.last().unwrap();
            prop_assert_eq!(
                &name,
                expected,
                "Current dir name should be {:?}, got {:?}",
                expected,
                name
            );
        }

        /// get_parent_path returns None for root, Some for others
        #[test]
        fn prop_parent_path_behavior(
            components in proptest::collection::vec(path_component_strategy(), 0..5)
        ) {
            let path_str = components.join("/");
            let path = Path::new(&path_str);
            let parent = get_parent_path(path);

            if components.is_empty() {
                prop_assert!(parent.is_none(), "Root should have no parent");
            } else {
                prop_assert!(parent.is_some(), "Non-root should have parent");
                let parent_str = parent.unwrap();
                prop_assert!(
                    parent_str.ends_with('/'),
                    "Parent path should end with /: {:?}",
                    parent_str
                );
            }
        }

        /// Parent path is shorter than original path (fewer characters)
        #[test]
        fn prop_parent_path_shorter_than_original(
            components in proptest::collection::vec(path_component_strategy(), 2..5)
        ) {
            // Need at least 2 components - for single component, parent is "/"
            // which is hard to compare meaningfully
            let path_str = components.join("/");
            let path = Path::new(&path_str);

            if let Some(parent) = get_parent_path(path) {
                // Parent path should be shorter in character length
                // (excluding the trailing slash we add)
                let parent_trimmed = parent.trim_end_matches('/');
                prop_assert!(
                    parent_trimmed.len() < path_str.len(),
                    "Parent {:?} should be shorter than {:?}",
                    parent_trimmed,
                    path_str
                );
            }
        }
    }
}
