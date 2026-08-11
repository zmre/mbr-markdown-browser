//! Embedded highlight.js files for syntax highlighting.
//!
//! This module embeds all highlight.js core and language files, providing
//! centralized access to syntax highlighting assets.

/// highlight.js dark theme CSS
pub const HLJS_DARK_CSS: &[u8] = include_bytes!("../templates/hljs.dark.11.11.2.css");
/// highlight.js atom-one-dark theme CSS
pub const HLJS_ATOM_ONE_DARK_CSS: &[u8] =
    include_bytes!("../templates/hljs.atom-one-dark.11.11.2.css");

/// highlight.js core library
pub const HLJS_JS: &[u8] = include_bytes!("../templates/hljs.11.11.2.js");

// Language modules
pub const HLJS_LANG_BASH: &[u8] = include_bytes!("../templates/hljs.lang.bash.11.11.2.js");
pub const HLJS_LANG_CSS: &[u8] = include_bytes!("../templates/hljs.lang.css.11.11.2.js");
pub const HLJS_LANG_DOCKERFILE: &[u8] =
    include_bytes!("../templates/hljs.lang.dockerfile.11.11.2.js");
pub const HLJS_LANG_GO: &[u8] = include_bytes!("../templates/hljs.lang.go.11.11.2.js");
pub const HLJS_LANG_JAVA: &[u8] = include_bytes!("../templates/hljs.lang.java.11.11.2.js");
pub const HLJS_LANG_JAVASCRIPT: &[u8] =
    include_bytes!("../templates/hljs.lang.javascript.11.11.2.js");
pub const HLJS_LANG_JSON: &[u8] = include_bytes!("../templates/hljs.lang.json.11.11.2.js");
pub const HLJS_LANG_MARKDOWN: &[u8] = include_bytes!("../templates/hljs.lang.markdown.11.11.2.js");
pub const HLJS_LANG_NIX: &[u8] = include_bytes!("../templates/hljs.lang.nix.11.11.2.js");
pub const HLJS_LANG_PYTHON: &[u8] = include_bytes!("../templates/hljs.lang.python.11.11.2.js");
pub const HLJS_LANG_RUBY: &[u8] = include_bytes!("../templates/hljs.lang.ruby.11.11.2.js");
pub const HLJS_LANG_RUST: &[u8] = include_bytes!("../templates/hljs.lang.rust.11.11.2.js");
pub const HLJS_LANG_SCALA: &[u8] = include_bytes!("../templates/hljs.lang.scala.11.11.2.js");
pub const HLJS_LANG_SQL: &[u8] = include_bytes!("../templates/hljs.lang.sql.11.11.2.js");
pub const HLJS_LANG_TYPESCRIPT: &[u8] =
    include_bytes!("../templates/hljs.lang.typescript.11.11.2.js");
pub const HLJS_LANG_XML: &[u8] = include_bytes!("../templates/hljs.lang.xml.11.11.2.js");
pub const HLJS_LANG_YAML: &[u8] = include_bytes!("../templates/hljs.lang.yaml.11.11.2.js");

/// All highlight.js files as (url_path, bytes, mime_type) tuples.
///
/// The url_path is the path without version numbers for cleaner URLs.
pub const HLJS_FILES: &[(&str, &[u8], &str)] = &[
    (
        "/hljs.atom-one-dark.css",
        HLJS_ATOM_ONE_DARK_CSS,
        "text/css",
    ),
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

/// Maps a filename extension to the highlight.js language that should render
/// it, or `None` when no *embedded* language module covers that extension.
///
/// Deliberately closed over [`HLJS_LANGUAGES`]: the binary only ships those 17
/// grammars, so returning any other name would emit a `language-*` class that
/// highlight.js silently ignores. `test_every_mapped_language_is_embedded`
/// keeps the two in sync.
///
/// `extension` is matched case-insensitively and must not include the dot.
/// Returning `None` is not an error - callers render those files verbatim.
pub fn language_for_extension(extension: &str) -> Option<&'static str> {
    // Allocation-free lowercasing for the common (already-lowercase) case.
    let ext = extension.to_ascii_lowercase();
    Some(match ext.as_str() {
        "bash" | "sh" | "zsh" | "ksh" | "bashrc" | "zshrc" => "bash",
        "css" => "css",
        "dockerfile" => "dockerfile",
        "go" => "go",
        "java" => "java",
        "js" | "mjs" | "cjs" | "jsx" => "javascript",
        "json" | "jsonc" | "json5" | "webmanifest" => "json",
        "md" | "markdown" | "mdown" | "mkd" | "mkdn" => "markdown",
        "nix" => "nix",
        "py" | "pyi" | "pyw" => "python",
        "rb" | "rake" | "gemspec" => "ruby",
        "rs" => "rust",
        "scala" | "sbt" | "sc" => "scala",
        "sql" => "sql",
        "ts" | "tsx" | "mts" | "cts" => "typescript",
        "xml" | "html" | "htm" | "xhtml" | "svg" | "plist" | "xsl" | "xslt" => "xml",
        "yaml" | "yml" => "yaml",
        _ => return None,
    })
}

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
                .any(|(path, _, _)| *path == "/hljs.atom-one-dark.css"),
            "Dark theme CSS should be present"
        );
    }

    #[test]
    fn test_every_mapped_language_is_embedded() {
        // A `language-foo` class for a grammar we do not ship is a silent
        // no-op in the browser, so every mapping must name an embedded module.
        let mapped = [
            "sh",
            "css",
            "dockerfile",
            "go",
            "java",
            "jsx",
            "json",
            "md",
            "nix",
            "py",
            "rb",
            "rs",
            "scala",
            "sql",
            "tsx",
            "svg",
            "yml",
        ];
        for ext in mapped {
            let lang = language_for_extension(ext)
                .unwrap_or_else(|| panic!("extension {ext} should map to a language"));
            assert!(
                HLJS_LANGUAGES.contains(&lang),
                "{ext} maps to {lang}, which has no embedded hljs module"
            );
        }
    }

    #[test]
    fn test_language_for_extension_is_case_insensitive() {
        assert_eq!(language_for_extension("RS"), Some("rust"));
        assert_eq!(language_for_extension("Json"), Some("json"));
    }

    #[test]
    fn test_language_for_extension_unknown_is_none() {
        // Plain text and types we ship no grammar for must fall through so the
        // caller renders them verbatim rather than mislabelling them.
        for ext in ["txt", "log", "toml", "csv", "", "exe"] {
            assert_eq!(language_for_extension(ext), None, "unexpected match: {ext}");
        }
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
