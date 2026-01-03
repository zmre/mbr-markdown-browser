//! Common test utilities for integration tests.

use std::collections::HashMap;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// A test fixture that creates a temporary markdown repository.
pub struct TestRepo {
    #[allow(dead_code)] // Kept to prevent TempDir from being dropped
    dir: TempDir,
    pub root: PathBuf,
}

impl TestRepo {
    /// Creates a new empty test repository with .mbr directory.
    pub fn new() -> Self {
        let dir = TempDir::new().expect("Failed to create temp directory");
        // Canonicalize the path to resolve symlinks (e.g., /var -> /private/var on macOS)
        // This prevents path diffing issues when computing relative paths
        let root = dir
            .path()
            .canonicalize()
            .expect("Failed to canonicalize temp directory");

        // Create .mbr directory (required for config detection)
        std::fs::create_dir(root.join(".mbr")).expect("Failed to create .mbr directory");

        Self { dir, root }
    }

    /// Creates a markdown file with the given content.
    pub fn create_markdown(&self, path: &str, content: &str) -> PathBuf {
        let file_path = self.root.join(path);
        if let Some(parent) = file_path.parent() {
            std::fs::create_dir_all(parent).expect("Failed to create parent directories");
        }
        std::fs::write(&file_path, content).expect("Failed to write markdown file");
        file_path
    }

    /// Creates a markdown file with frontmatter.
    pub fn create_markdown_with_frontmatter(
        &self,
        path: &str,
        frontmatter: &HashMap<&str, &str>,
        body: &str,
    ) -> PathBuf {
        let mut content = String::from("---\n");
        for (key, value) in frontmatter {
            content.push_str(&format!("{}: {}\n", key, value));
        }
        content.push_str("---\n\n");
        content.push_str(body);
        self.create_markdown(path, &content)
    }

    /// Creates a subdirectory.
    pub fn create_dir(&self, path: &str) -> PathBuf {
        let dir_path = self.root.join(path);
        std::fs::create_dir_all(&dir_path).expect("Failed to create directory");
        dir_path
    }

    /// Creates a static file.
    #[allow(dead_code)]
    pub fn create_static_file(&self, path: &str, content: &[u8]) -> PathBuf {
        let file_path = self.root.join(path);
        if let Some(parent) = file_path.parent() {
            std::fs::create_dir_all(parent).expect("Failed to create parent directories");
        }
        std::fs::write(&file_path, content).expect("Failed to write static file");
        file_path
    }

    /// Returns the path to the root directory.
    pub fn path(&self) -> &Path {
        &self.root
    }
}

impl Default for TestRepo {
    fn default() -> Self {
        Self::new()
    }
}

/// Finds an available port for testing.
pub fn find_available_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("Failed to bind to port");
    listener
        .local_addr()
        .expect("Failed to get local address")
        .port()
}

/// Asserts that HTML content contains the expected substring.
pub fn assert_html_contains(html: &str, expected: &str) {
    assert!(
        html.contains(expected),
        "Expected HTML to contain '{}', but it wasn't found.\nHTML content:\n{}",
        expected,
        html
    );
}

/// Asserts that HTML content does not contain the unexpected substring.
pub fn assert_html_not_contains(html: &str, unexpected: &str) {
    assert!(
        !html.contains(unexpected),
        "Expected HTML to NOT contain '{}', but it was found.\nHTML content:\n{}",
        unexpected,
        html
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_repo() {
        let repo = TestRepo::new();
        assert!(repo.path().exists());
        assert!(repo.path().join(".mbr").exists());
    }

    #[test]
    fn test_create_markdown() {
        let repo = TestRepo::new();
        let path = repo.create_markdown("test.md", "# Hello");
        assert!(path.exists());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "# Hello");
    }

    #[test]
    fn test_create_markdown_in_subdir() {
        let repo = TestRepo::new();
        let path = repo.create_markdown("docs/guide.md", "# Guide");
        assert!(path.exists());
        assert!(repo.path().join("docs").is_dir());
    }

    #[test]
    fn test_create_markdown_with_frontmatter() {
        let repo = TestRepo::new();
        let mut fm = HashMap::new();
        fm.insert("title", "My Title");
        fm.insert("author", "Test Author");

        let path = repo.create_markdown_with_frontmatter("post.md", &fm, "Content here");
        let content = std::fs::read_to_string(&path).unwrap();

        assert!(content.contains("---"));
        assert!(content.contains("title: My Title"));
        assert!(content.contains("author: Test Author"));
        assert!(content.contains("Content here"));
    }

    #[test]
    fn test_find_available_port() {
        let port = find_available_port();
        assert!(port > 0);

        // Verify the port is actually available
        let listener = TcpListener::bind(format!("127.0.0.1:{}", port));
        assert!(listener.is_ok());
    }

    #[test]
    fn test_assert_html_contains() {
        let html = "<html><body><h1>Title</h1></body></html>";
        assert_html_contains(html, "<h1>Title</h1>");
        assert_html_contains(html, "body");
    }

    #[test]
    #[should_panic(expected = "Expected HTML to contain")]
    fn test_assert_html_contains_fails() {
        let html = "<html><body></body></html>";
        assert_html_contains(html, "missing");
    }

    #[test]
    fn test_assert_html_not_contains() {
        let html = "<html><body></body></html>";
        assert_html_not_contains(html, "script");
    }
}
