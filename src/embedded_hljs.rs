//! Embedded highlight.js files for syntax highlighting.
//!
//! This module embeds all highlight.js core and language files, providing
//! centralized access to syntax highlighting assets.

/// highlight.js dark theme CSS
pub const HLJS_DARK_CSS: &[u8] = include_bytes!("../templates/hljs.dark.11.11.1.css");

/// highlight.js core library
pub const HLJS_JS: &[u8] = include_bytes!("../templates/hljs.11.11.1.js");

// Language modules
pub const HLJS_LANG_BASH: &[u8] = include_bytes!("../templates/hljs.lang.bash.11.11.1.js");
pub const HLJS_LANG_CSS: &[u8] = include_bytes!("../templates/hljs.lang.css.11.11.1.js");
pub const HLJS_LANG_DOCKERFILE: &[u8] =
    include_bytes!("../templates/hljs.lang.dockerfile.11.11.1.js");
pub const HLJS_LANG_GO: &[u8] = include_bytes!("../templates/hljs.lang.go.11.11.1.js");
pub const HLJS_LANG_JAVA: &[u8] = include_bytes!("../templates/hljs.lang.java.11.11.1.js");
pub const HLJS_LANG_JAVASCRIPT: &[u8] =
    include_bytes!("../templates/hljs.lang.javascript.11.11.1.js");
pub const HLJS_LANG_JSON: &[u8] = include_bytes!("../templates/hljs.lang.json.11.11.1.js");
pub const HLJS_LANG_MARKDOWN: &[u8] = include_bytes!("../templates/hljs.lang.markdown.11.11.1.js");
pub const HLJS_LANG_NIX: &[u8] = include_bytes!("../templates/hljs.lang.nix.11.11.1.js");
pub const HLJS_LANG_PYTHON: &[u8] = include_bytes!("../templates/hljs.lang.python.11.11.1.js");
pub const HLJS_LANG_RUBY: &[u8] = include_bytes!("../templates/hljs.lang.ruby.11.11.1.js");
pub const HLJS_LANG_RUST: &[u8] = include_bytes!("../templates/hljs.lang.rust.11.11.1.js");
pub const HLJS_LANG_SCALA: &[u8] = include_bytes!("../templates/hljs.lang.scala.11.11.1.js");
pub const HLJS_LANG_SQL: &[u8] = include_bytes!("../templates/hljs.lang.sql.11.11.1.js");
pub const HLJS_LANG_TYPESCRIPT: &[u8] =
    include_bytes!("../templates/hljs.lang.typescript.11.11.1.js");
pub const HLJS_LANG_XML: &[u8] = include_bytes!("../templates/hljs.lang.xml.11.11.1.js");
pub const HLJS_LANG_YAML: &[u8] = include_bytes!("../templates/hljs.lang.yaml.11.11.1.js");

/// All highlight.js files as (url_path, bytes, mime_type) tuples.
///
/// The url_path is the path without version numbers for cleaner URLs.
pub const HLJS_FILES: &[(&str, &[u8], &str)] = &[
    ("/hljs.dark.css", HLJS_DARK_CSS, "text/css"),
    ("/hljs.js", HLJS_JS, "application/javascript"),
    (
        "/hljs.lang.bash.js",
        HLJS_LANG_BASH,
        "application/javascript",
    ),
    ("/hljs.lang.css.js", HLJS_LANG_CSS, "application/javascript"),
    (
        "/hljs.lang.dockerfile.js",
        HLJS_LANG_DOCKERFILE,
        "application/javascript",
    ),
    ("/hljs.lang.go.js", HLJS_LANG_GO, "application/javascript"),
    (
        "/hljs.lang.java.js",
        HLJS_LANG_JAVA,
        "application/javascript",
    ),
    (
        "/hljs.lang.javascript.js",
        HLJS_LANG_JAVASCRIPT,
        "application/javascript",
    ),
    (
        "/hljs.lang.json.js",
        HLJS_LANG_JSON,
        "application/javascript",
    ),
    (
        "/hljs.lang.markdown.js",
        HLJS_LANG_MARKDOWN,
        "application/javascript",
    ),
    ("/hljs.lang.nix.js", HLJS_LANG_NIX, "application/javascript"),
    (
        "/hljs.lang.python.js",
        HLJS_LANG_PYTHON,
        "application/javascript",
    ),
    (
        "/hljs.lang.ruby.js",
        HLJS_LANG_RUBY,
        "application/javascript",
    ),
    (
        "/hljs.lang.rust.js",
        HLJS_LANG_RUST,
        "application/javascript",
    ),
    (
        "/hljs.lang.scala.js",
        HLJS_LANG_SCALA,
        "application/javascript",
    ),
    ("/hljs.lang.sql.js", HLJS_LANG_SQL, "application/javascript"),
    (
        "/hljs.lang.typescript.js",
        HLJS_LANG_TYPESCRIPT,
        "application/javascript",
    ),
    ("/hljs.lang.xml.js", HLJS_LANG_XML, "application/javascript"),
    (
        "/hljs.lang.yaml.js",
        HLJS_LANG_YAML,
        "application/javascript",
    ),
];

/// List of supported highlight.js languages.
pub const HLJS_LANGUAGES: &[&str] = &[
    "bash",
    "css",
    "dockerfile",
    "go",
    "java",
    "javascript",
    "json",
    "markdown",
    "nix",
    "python",
    "ruby",
    "rust",
    "scala",
    "sql",
    "typescript",
    "xml",
    "yaml",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hljs_files_not_empty() {
        for (path, content, _mime) in HLJS_FILES.iter() {
            assert!(
                !content.is_empty(),
                "HLJS file {} should not be empty",
                path
            );
        }
    }

    #[test]
    fn test_hljs_file_count() {
        // 1 CSS + 1 core JS + 17 language modules = 19 total
        assert_eq!(HLJS_FILES.len(), 19);
    }

    #[test]
    fn test_all_languages_have_files() {
        for lang in HLJS_LANGUAGES {
            let expected_path = format!("/hljs.lang.{}.js", lang);
            assert!(
                HLJS_FILES.iter().any(|(path, _, _)| *path == expected_path),
                "Missing HLJS file for language: {}",
                lang
            );
        }
    }

    #[test]
    fn test_core_files_present() {
        assert!(
            HLJS_FILES.iter().any(|(path, _, _)| *path == "/hljs.js"),
            "Core hljs.js should be present"
        );
        assert!(
            HLJS_FILES
                .iter()
                .any(|(path, _, _)| *path == "/hljs.dark.css"),
            "Dark theme CSS should be present"
        );
    }

    #[test]
    fn test_mime_types_correct() {
        for (path, _, mime) in HLJS_FILES.iter() {
            if path.ends_with(".css") {
                assert_eq!(
                    *mime, "text/css",
                    "CSS files should have text/css mime type"
                );
            } else if path.ends_with(".js") {
                assert_eq!(
                    *mime, "application/javascript",
                    "JS files should have application/javascript mime type"
                );
            }
        }
    }
}
