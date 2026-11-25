//! `Content-Security-Policy` headers per route class, and what a strict policy
//! needs to allow for mbr's own pages.
//!
//! mbr sends no policy by default. An app that serves a repository on its own
//! origin (or an operator serving a shared repository) chooses one per
//! [`ContentClass`]; `mbr_server` adds the header to every response of that
//! class.
//!
//! # Strict `script-src`
//!
//! mbr's page templates run **no inline script**: page data
//! (`window.__MBR_CONFIG__`, `window.frontmatter`, `window.headings`,
//! `window.extendedMeta`, `window.__MBR_MEDIA_VIEWER__`) is written as
//! `<script type="application/json" data-mbr-globals>` blocks, which
//! `script-src` does not govern, and defined by the external
//! [`BOOTSTRAP_SCRIPT`]; the templates carry no `on…=` handler attributes and no
//! `javascript:` URLs. So `script-src 'self'` ([`STRICT_SCRIPT_SRC`]) is enough
//! for every page mbr renders. [`TEMPLATE_INLINE_SCRIPT_HASHES`] lists the
//! hashes of any inline script a template still carries, for a policy to name;
//! it is empty, and a test keeps it honest.
//!
//! Not covered by that: a repository's own `.mbr/` template overrides and
//! per-note page templates (they may carry inline script of their own),
//! `ServerConfig::extra_head_html`, a bare GitHub gist URL's `<script>` embed
//! (from `gist.github.com`), and raw HTML in notes under
//! [`crate::raw_html::RawHtmlPolicy::Passthrough`].

use serde::{Deserialize, Serialize};

/// The convenience policy for content that should never run script:
/// a CSP sandbox that keeps the document's origin (so its images, styles and
/// same-origin links still load) but allows no script, form, popup or
/// top-level navigation.
///
/// On a rendered note page it also stops mbr's own components, so the page is
/// read-only markup: no sidebar, search, editor or live reload.
pub const SANDBOX: &str = "sandbox allow-same-origin";

/// The `script-src` directive that is enough for mbr's own pages
/// (see the module docs).
pub const STRICT_SCRIPT_SRC: &str = "script-src 'self'";

/// URL path, under `/.mbr/`, of the classic script that turns the page's
/// `data-mbr-globals` JSON blocks into `window` globals.
pub const BOOTSTRAP_SCRIPT: &str = "/mbr-bootstrap.js";

/// `'sha256-…'` sources of the inline scripts mbr's default templates still
/// contain. Empty: every inline script was moved to [`BOOTSTRAP_SCRIPT`]. A
/// test renders the templates and fails if an executable inline script appears
/// whose hash is not listed here.
pub const TEMPLATE_INLINE_SCRIPT_HASHES: &[&str] = &[];

/// Which kind of response a policy applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ContentClass {
    /// HTML mbr renders from repository content: notes, the home page, folder
    /// listings, tag pages and error pages.
    Pages,
    /// Repository files served as they are: static files (an `.html` file
    /// included), media, and files a repository's `.mbr/` folder overrides.
    UserFiles,
    /// mbr's own UI under `/.mbr/`: compiled-in assets, JSON endpoints, the
    /// media viewers and the outside-link page.
    MbrAssets,
}

impl ContentClass {
    /// The class a response gets from its request path alone, when the
    /// handler did not name one: `/.mbr/…` is mbr's UI, anything else a page.
    #[must_use]
    pub fn for_path(path: &str) -> Self {
        if path == "/.mbr" || path.starts_with("/.mbr/") {
            Self::MbrAssets
        } else {
            Self::Pages
        }
    }
}

/// A `Content-Security-Policy` header value per [`ContentClass`]; `None` sends
/// no header for that class. The `[content_security_policy]` table of
/// `config.toml` (`MBR_CONTENT_SECURITY_POLICY__PAGES`, …) and
/// `mbr_server::server::ServerConfig::content_security_policy`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ContentSecurityPolicy {
    pub pages: Option<String>,
    pub user_files: Option<String>,
    pub mbr_assets: Option<String>,
}

impl ContentSecurityPolicy {
    /// [`SANDBOX`] for pages and user files, nothing for mbr's assets: no
    /// script from the repository runs at all (mbr's components on note pages
    /// included).
    #[must_use]
    pub fn sandbox_user_content() -> Self {
        Self {
            pages: Some(SANDBOX.to_string()),
            user_files: Some(SANDBOX.to_string()),
            mbr_assets: None,
        }
    }

    /// The policy for `class`, if any.
    #[must_use]
    pub fn for_class(&self, class: ContentClass) -> Option<&str> {
        match class {
            ContentClass::Pages => self.pages.as_deref(),
            ContentClass::UserFiles => self.user_files.as_deref(),
            ContentClass::MbrAssets => self.mbr_assets.as_deref(),
        }
    }

    /// True when no class sends a header (the default).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pages.is_none() && self.user_files.is_none() && self.mbr_assets.is_none()
    }
}

/// The `'sha256-…'` source that lets a policy allow one inline script whose
/// text is `script`.
#[must_use]
pub fn script_hash_source(script: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(script.as_bytes());
    format!(
        "'sha256-{}'",
        crate::host_head::base64_standard(digest.as_slice())
    )
}

/// True for a `<script type>` a browser runs: none, empty, `module`, or a
/// JavaScript MIME type. Everything else (`application/json`, …) is a data
/// block, which `script-src` does not govern.
fn is_executable_script_type(script_type: Option<&str>) -> bool {
    let Some(script_type) = script_type.map(|t| t.trim().to_ascii_lowercase()) else {
        return true;
    };
    script_type.is_empty()
        || script_type == "module"
        || script_type.contains("javascript")
        || script_type.contains("ecmascript")
        || script_type.contains("jscript")
        || script_type.contains("livescript")
}

/// What in a page a `script-src` without `'unsafe-inline'` would block: each
/// executable inline `<script>` whose hash is not in `allowed_hashes`, each
/// `on…=` event-handler attribute and each `javascript:` URL in an `href`,
/// `src`, `action` or `formaction`. Empty means the page works under
/// [`STRICT_SCRIPT_SRC`] plus those hashes.
///
/// For checking pages and templates (mbr's tests use it on every kind of page
/// it renders); it does not judge `style-src`.
#[must_use]
pub fn strict_script_violations(html: &str, allowed_hashes: &[&str]) -> Vec<String> {
    let document = scraper::Html::parse_document(html);
    let mut violations = Vec::new();
    let Ok(all) = scraper::Selector::parse("*") else {
        return violations;
    };
    for element in document.select(&all) {
        let value = element.value();
        if value.name() == "script"
            && value.attr("src").is_none()
            && is_executable_script_type(value.attr("type"))
        {
            let text: String = element.text().collect();
            let hash = script_hash_source(&text);
            if !allowed_hashes.contains(&hash.as_str()) {
                violations.push(format!("inline <script> {hash}: {}", text.trim()));
            }
        }
        for (name, attr) in value.attrs() {
            let name = name.to_ascii_lowercase();
            let handler = name.starts_with("on");
            let script_url = matches!(name.as_str(), "href" | "src" | "action" | "formaction")
                && attr
                    .trim_start()
                    .to_ascii_lowercase()
                    .starts_with("javascript:");
            if handler || script_url {
                violations.push(format!("<{} {name}=\"{attr}\">", value.name()));
            }
        }
    }
    violations
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_violations_find_inline_code_only() {
        let html = r#"<html><head>
            <script type="application/json" data-mbr-globals>{"a":1}</script>
            <script src="/.mbr/mbr-bootstrap.js"></script>
            <script>alert(1)</script>
            <script type="module">import "x"</script>
            </head><body>
            <button onclick="go()">x</button>
            <a href=" javascript:alert(1)">y</a>
            <a href="/ok/">z</a>
            </body></html>"#;
        let violations = strict_script_violations(html, &[]);
        assert_eq!(violations.len(), 4, "{violations:#?}");
        let alert = script_hash_source("alert(1)");
        let module = script_hash_source("import \"x\"");
        let allowed = [alert.as_str(), module.as_str()];
        assert_eq!(strict_script_violations(html, &allowed).len(), 2);
    }

    /// Every default template, as source (Tera comments removed): no inline
    /// script beyond [`TEMPLATE_INLINE_SCRIPT_HASHES`], no handler attribute,
    /// no `javascript:` URL. A new inline script fails here first; the
    /// rendered pages are checked by the server and build integration tests.
    #[test]
    fn default_templates_need_no_unsafe_inline() {
        let comments = regex::Regex::new(r"(?s)\{#.*?#\}").unwrap();
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("templates");
        let mut checked = 0;
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|e| e.to_str()) != Some("html") {
                continue;
            }
            let source = std::fs::read_to_string(&path).unwrap();
            let source = comments.replace_all(&source, "");
            let violations = strict_script_violations(&source, TEMPLATE_INLINE_SCRIPT_HASHES);
            assert!(violations.is_empty(), "{}: {violations:#?}", path.display());
            checked += 1;
        }
        assert!(checked > 20, "only {checked} templates found");
    }

    #[test]
    fn bootstrap_is_served_where_the_templates_load_it() {
        let (bytes, _) = crate::assets::default_file(BOOTSTRAP_SCRIPT).expect("in DEFAULT_FILES");
        let text = std::str::from_utf8(bytes).unwrap();
        for global in ["__MBR_CONFIG__", "frontmatter", "headings", "extendedMeta"] {
            assert!(text.contains(&format!("\"{global}\"")), "{global}");
        }
        assert!(text.contains("data-mbr-companion-prepaint"));
        assert!(text.contains("data-mbr-history-back"));
        for template in [
            include_str!("../templates/_head.html"),
            include_str!("../templates/_head_markdown.html"),
            include_str!("../templates/media_viewer.html"),
        ] {
            assert!(template.contains("{{ asset_base }}mbr-bootstrap.js"));
        }
    }

    #[test]
    fn script_hash_matches_a_known_value() {
        // printf 'alert(1)' | openssl dgst -sha256 -binary | base64
        assert_eq!(
            script_hash_source("alert(1)"),
            "'sha256-bhHHL3z2vDgxUt0W3dWQOrprscmda2Y5pLsLg4GF+pI='"
        );
    }

    #[test]
    fn class_from_path() {
        assert_eq!(
            ContentClass::for_path("/.mbr/theme.css"),
            ContentClass::MbrAssets
        );
        assert_eq!(ContentClass::for_path("/.mbr"), ContentClass::MbrAssets);
        assert_eq!(ContentClass::for_path("/.mbrx/a"), ContentClass::Pages);
        assert_eq!(ContentClass::for_path("/notes/a/"), ContentClass::Pages);
        assert_eq!(ContentClass::for_path("/"), ContentClass::Pages);
    }

    #[test]
    fn sandbox_preset() {
        let policy = ContentSecurityPolicy::sandbox_user_content();
        assert_eq!(policy.for_class(ContentClass::Pages), Some(SANDBOX));
        assert_eq!(policy.for_class(ContentClass::UserFiles), Some(SANDBOX));
        assert_eq!(policy.for_class(ContentClass::MbrAssets), None);
        assert!(!SANDBOX.contains("allow-scripts"));
        assert!(ContentSecurityPolicy::default().is_empty());
    }
}
