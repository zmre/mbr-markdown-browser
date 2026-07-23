use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use crate::errors::TemplateError;
use parking_lot::RwLock;
use tera::{Context, Tera};

#[derive(Clone)]
pub struct Templates {
    tera: Arc<RwLock<Tera>>,
    /// Path used for template loading (for hot reload)
    template_path: PathBuf,
}

impl Templates {
    /// Creates a new Templates instance.
    ///
    /// Template loading priority:
    /// 1. If `template_folder` is provided, load from `{template_folder}/**/*.html`
    /// 2. Otherwise, load from `{root_path}/.mbr/**/*.html`
    /// 3. Fall back to compiled defaults for any missing templates
    pub fn new(root_path: &Path, template_folder: Option<&Path>) -> Result<Self, TemplateError> {
        let template_path = if let Some(tf) = template_folder {
            tf.to_path_buf()
        } else {
            root_path.join(".mbr")
        };

        let tera = Self::load_tera(&template_path)?;

        Ok(Templates {
            tera: Arc::new(RwLock::new(tera)),
            template_path,
        })
    }

    /// Load Tera templates from the given path, with fallback to compiled defaults.
    fn load_tera(template_path: &Path) -> Result<Tera, TemplateError> {
        let globs = template_path.join("**/*.html");
        let source_desc = format!("{}", template_path.display());

        let globs_str = globs.to_str().ok_or(TemplateError::InvalidPathEncoding)?;
        let mut tera = Tera::new(globs_str).unwrap_or_else(|e| {
            tracing::warn!(
                "Failed to load user templates from {}: {}. Using built-in defaults.",
                source_desc,
                e
            );
            Tera::default()
        });

        // Custom filters. `humandate` humanizes ISO-ish date strings (see
        // `humanize_date`); registered here so it survives `reload()` and the
        // `Tera::default()` fallback above.
        tera.register_filter("humandate", humandate_filter);

        for (name, tpl) in DEFAULT_TEMPLATES.iter() {
            if tera.get_template(name).is_err() {
                tracing::debug!("Adding default template {}", name);
                tera.add_raw_template(name, tpl)
                    .map_err(|e| TemplateError::RenderFailed {
                        template_name: name.to_string(),
                        source: e,
                    })?;
            }
        }

        Ok(tera)
    }

    /// Reload all templates from disk. Call this when template files change.
    pub fn reload(&self) -> Result<(), TemplateError> {
        tracing::info!("Reloading templates from {:?}", self.template_path);
        let new_tera = Self::load_tera(&self.template_path)?;
        *self.tera.write() = new_tera;
        tracing::debug!("Templates reloaded successfully");
        Ok(())
    }

    /// Returns a clone of the Tera engine for lock-free rendering.
    ///
    /// Acquires the read lock once to clone the Tera instance (~KB of template AST).
    /// Use this before entering a rayon thread pool to avoid per-file lock contention.
    pub fn tera_clone(&self) -> Tera {
        self.tera.read().clone()
    }

    pub fn render_markdown(
        &self,
        html: &str,
        frontmatter: HashMap<String, serde_json::Value>,
        extra_context: HashMap<String, serde_json::Value>,
    ) -> Result<String, TemplateError> {
        let tera = self.tera.read();
        Self::render_markdown_with_tera(&tera, html, frontmatter, extra_context)
    }

    /// Lock-free variant of `render_markdown` that takes a `&Tera` directly.
    ///
    /// Use with `tera_clone()` to avoid `Arc<RwLock<Tera>>` contention when
    /// rendering many files in parallel (e.g., from a rayon thread pool).
    pub fn render_markdown_with_tera(
        tera: &Tera,
        html: &str,
        frontmatter: HashMap<String, serde_json::Value>,
        extra_context: HashMap<String, serde_json::Value>,
    ) -> Result<String, TemplateError> {
        tracing::debug!("frontmatter: {:?}", &frontmatter);

        // Create JSON from frontmatter BEFORE adding markdown to context
        // This avoids including the large markdown HTML in the frontmatter JSON
        let frontmatter_json =
            serde_json::to_string(&frontmatter).unwrap_or_else(|_| "{}".to_string());

        let mut context = Context::new();
        frontmatter.iter().for_each(|(k, v)| {
            // Normalize "style" frontmatter: if it's an array, join with spaces
            // This allows `style: ['slides', 'other']` to work as body classes
            if k == "style" {
                let normalized = normalize_style_value(v);
                context.insert(k, &normalized);
            } else {
                context.insert(k, v);
            }
        });
        // Add extra context (breadcrumbs, current_dir_name, etc.)
        extra_context.iter().for_each(|(k, v)| {
            context.insert(k, v);
        });
        context.insert("markdown", html);
        context.insert("frontmatter_json", &frontmatter_json);

        let html_output =
            tera.render("index.html", &context)
                .map_err(|e| TemplateError::RenderFailed {
                    template_name: "index.html".to_string(),
                    source: e,
                })?;
        Ok(html_output)
    }

    /// Lock-free generic template render that takes a `&Tera` directly.
    ///
    /// Use with `tera_clone()` to avoid `Arc<RwLock<Tera>>` contention when
    /// rendering many pages in parallel (e.g., from a rayon thread pool).
    pub fn render_template_with_tera(
        tera: &Tera,
        template_name: &str,
        context_data: HashMap<String, serde_json::Value>,
    ) -> Result<String, TemplateError> {
        let mut context = Context::new();
        context_data.iter().for_each(|(k, v)| {
            context.insert(k, v);
        });
        tera.render(template_name, &context)
            .map_err(|e| TemplateError::RenderFailed {
                template_name: template_name.to_string(),
                source: e,
            })
    }

    pub fn render_section(
        &self,
        context_data: HashMap<String, serde_json::Value>,
    ) -> Result<String, TemplateError> {
        let tera = self.tera.read();
        Self::render_template_with_tera(&tera, "section.html", context_data)
    }

    /// Renders the home page (root directory) using home.html template.
    /// This allows users to customize their home page differently from section pages.
    pub fn render_home(
        &self,
        context_data: HashMap<String, serde_json::Value>,
    ) -> Result<String, TemplateError> {
        let tera = self.tera.read();
        Self::render_template_with_tera(&tera, "home.html", context_data)
    }

    /// Renders an error page using error.html template.
    ///
    /// Context variables:
    /// - `error_code`: HTTP status code (e.g., 404, 500)
    /// - `error_title`: Short error title (e.g., "Not Found")
    /// - `error_message`: Optional detailed message
    /// - `requested_url`: The URL that was requested (useful in GUI mode without URL bar)
    /// - `server_mode`: Boolean indicating server vs static mode
    /// - `relative_base`: Path prefix to .mbr assets (e.g., ".mbr/", "../.mbr/")
    /// - `relative_root`: Path prefix to root (e.g., "", "../")
    pub fn render_error(
        &self,
        context_data: HashMap<String, serde_json::Value>,
    ) -> Result<String, TemplateError> {
        let tera = self.tera.read();
        Self::render_template_with_tera(&tera, "error.html", context_data)
    }

    /// Renders a tag page showing all pages with a specific tag.
    ///
    /// Context variables:
    /// - `tag_source`: URL identifier for the tag source (e.g., "tags", "performers")
    /// - `tag_display_value`: Original display value of the tag (e.g., "Rust", "Joshua Jay")
    /// - `tag_label`: Singular label for the tag source (e.g., "Tag", "Performer")
    /// - `tag_label_plural`: Plural label for the tag source (e.g., "Tags", "Performers")
    /// - `pages`: Array of page objects with url_path, title, description
    /// - `page_count`: Number of pages with this tag
    /// - `server_mode`: Boolean indicating server vs static mode
    /// - `relative_base`: Path prefix to .mbr assets
    pub fn render_tag(
        &self,
        context_data: HashMap<String, serde_json::Value>,
    ) -> Result<String, TemplateError> {
        let tera = self.tera.read();
        Self::render_template_with_tera(&tera, "tag.html", context_data)
    }

    /// Renders a tag source index showing all tags from a source.
    ///
    /// Context variables:
    /// - `tag_source`: URL identifier for the tag source (e.g., "tags", "performers")
    /// - `tag_label`: Singular label for the tag source (e.g., "Tag", "Performer")
    /// - `tag_label_plural`: Plural label for the tag source (e.g., "Tags", "Performers")
    /// - `tags`: Array of tag objects with url_value, display_value, page_count
    /// - `tag_count`: Total number of unique tags
    /// - `server_mode`: Boolean indicating server vs static mode
    /// - `relative_base`: Path prefix to .mbr assets
    pub fn render_tag_index(
        &self,
        context_data: HashMap<String, serde_json::Value>,
    ) -> Result<String, TemplateError> {
        let tera = self.tera.read();
        Self::render_template_with_tera(&tera, "tag_index.html", context_data)
    }

    /// Renders a media viewer page for video, PDF, or audio content.
    ///
    /// Context variables:
    /// - `media_type`: Type of media ("video", "pdf", "audio")
    /// - `title`: Page title (defaults to filename)
    /// - `media_path`: Path to the media file
    /// - `breadcrumbs`: Navigation breadcrumbs
    /// - `parent_path`: URL to parent directory for back navigation
    /// - `server_mode`: Boolean indicating server vs static mode
    /// - `relative_base`: Path prefix to .mbr assets
    /// - `sidebar_style`: Sidebar navigation style
    /// - `sidebar_max_items`: Maximum items per section in sidebar
    pub fn render_media_viewer(
        &self,
        context_data: HashMap<String, serde_json::Value>,
    ) -> Result<String, TemplateError> {
        let tera = self.tera.read();
        Self::render_template_with_tera(&tera, "media_viewer.html", context_data)
    }
}

/// Full English month names, indexed by `month - 1`.
const MONTH_NAMES: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// Tera `humandate` filter: humanize a date string, passing through any other
/// value unchanged.
///
/// Strings are run through [`humanize_date`]; non-string values (numbers, bools,
/// null, arrays, objects) are returned as-is so the filter never errors.
fn humandate_filter(
    value: &serde_json::Value,
    _args: &HashMap<String, serde_json::Value>,
) -> tera::Result<serde_json::Value> {
    match value {
        serde_json::Value::String(s) => Ok(serde_json::Value::String(humanize_date(s))),
        other => Ok(other.clone()),
    }
}

/// Returns `true` when `s` is a non-empty run of ASCII digits.
fn is_all_ascii_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

/// Parse an exactly-4-digit year component (e.g. `"1855"`).
fn parse_year4(s: &str) -> Option<u32> {
    if s.len() == 4 && is_all_ascii_digits(s) {
        s.parse().ok()
    } else {
        None
    }
}

/// Parse an exactly-2-digit month in `1..=12`.
fn parse_month2(s: &str) -> Option<u32> {
    if s.len() == 2 && is_all_ascii_digits(s) {
        let m: u32 = s.parse().ok()?;
        (1..=12).contains(&m).then_some(m)
    } else {
        None
    }
}

/// Parse an exactly-2-digit day in `1..=31`.
fn parse_day2(s: &str) -> Option<u32> {
    if s.len() == 2 && is_all_ascii_digits(s) {
        let d: u32 = s.parse().ok()?;
        (1..=31).contains(&d).then_some(d)
    } else {
        None
    }
}

/// Humanize an ISO-ish date string into a reader-friendly form.
///
/// - `YYYY-MM-DD` (valid month 1-12, day 1-31) → `"Month D, YYYY"` with the
///   day's leading zero stripped (e.g. `"1855-10-30"` → `"October 30, 1855"`).
/// - `YYYY-MM` (valid month) → `"Month YYYY"` (e.g. `"1855-10"` → `"October 1855"`).
/// - `YYYY` → unchanged.
/// - Anything else — partial, prefixed ("circa 1855"), already-formatted, or
///   out-of-range (e.g. `"2020-13-40"`) — is returned UNCHANGED.
fn humanize_date(input: &str) -> String {
    let parts: Vec<&str> = input.split('-').collect();
    match parts.as_slice() {
        [y, m, d] => {
            if let (Some(year), Some(month), Some(day)) =
                (parse_year4(y), parse_month2(m), parse_day2(d))
            {
                return format!("{} {}, {}", MONTH_NAMES[(month - 1) as usize], day, year);
            }
        }
        [y, m] => {
            if let (Some(year), Some(month)) = (parse_year4(y), parse_month2(m)) {
                return format!("{} {}", MONTH_NAMES[(month - 1) as usize], year);
            }
        }
        // Everything else — a bare `YYYY`, partials, prose, already-formatted,
        // or out-of-range dates — is returned unchanged by the fallthrough.
        _ => {}
    }
    input.to_string()
}

/// Normalize a style frontmatter value to a space-separated string.
///
/// Handles:
/// - String: returned as-is
/// - Array: elements joined with spaces
/// - Other: converted to string representation
fn normalize_style_value(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(arr) => arr
            .iter()
            .filter_map(|v| v.as_str())
            .collect::<Vec<_>>()
            .join(" "),
        other => other.to_string(),
    }
}

const DEFAULT_TEMPLATES: &[(&str, &str)] = &[
    // Partials (underscore prefix indicates internal-only templates)
    ("_head.html", include_str!("../templates/_head.html")),
    (
        "_head_custom.html",
        include_str!("../templates/_head_custom.html"),
    ),
    (
        "_head_markdown.html",
        include_str!("../templates/_head_markdown.html"),
    ),
    ("_nav.html", include_str!("../templates/_nav.html")),
    (
        "_breadcrumbs.html",
        include_str!("../templates/_breadcrumbs.html"),
    ),
    ("_footer.html", include_str!("../templates/_footer.html")),
    (
        "_footer_custom.html",
        include_str!("../templates/_footer_custom.html"),
    ),
    ("_scripts.html", include_str!("../templates/_scripts.html")),
    (
        "_display_enhancements.html",
        include_str!("../templates/_display_enhancements.html"),
    ),
    (
        "_person_infobox.html",
        include_str!("../templates/_person_infobox.html"),
    ),
    // Main templates
    ("index.html", include_str!("../templates/index.html")),
    ("section.html", include_str!("../templates/section.html")),
    ("home.html", include_str!("../templates/home.html")),
    ("error.html", include_str!("../templates/error.html")),
    // Tag templates
    ("tag.html", include_str!("../templates/tag.html")),
    (
        "tag_index.html",
        include_str!("../templates/tag_index.html"),
    ),
    // Media viewer template
    (
        "media_viewer.html",
        include_str!("../templates/media_viewer.html"),
    ),
];

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_normalize_style_string() {
        let value = json!("slides");
        assert_eq!(normalize_style_value(&value), "slides");
    }

    #[test]
    fn test_normalize_style_string_with_spaces() {
        let value = json!("slides other");
        assert_eq!(normalize_style_value(&value), "slides other");
    }

    #[test]
    fn test_normalize_style_array() {
        let value = json!(["slides", "other"]);
        assert_eq!(normalize_style_value(&value), "slides other");
    }

    #[test]
    fn test_normalize_style_array_single_element() {
        let value = json!(["slides"]);
        assert_eq!(normalize_style_value(&value), "slides");
    }

    #[test]
    fn test_normalize_style_array_empty() {
        let value = json!([]);
        assert_eq!(normalize_style_value(&value), "");
    }

    #[test]
    fn test_normalize_style_null() {
        let value = json!(null);
        assert_eq!(normalize_style_value(&value), "null");
    }

    #[test]
    fn test_normalize_style_number() {
        let value = json!(42);
        assert_eq!(normalize_style_value(&value), "42");
    }

    #[test]
    fn test_humanize_date_full() {
        assert_eq!(humanize_date("1855-10-30"), "October 30, 1855");
        assert_eq!(humanize_date("1902-01-10"), "January 10, 1902");
    }

    #[test]
    fn test_humanize_date_full_strips_leading_zero_day() {
        assert_eq!(humanize_date("1855-10-05"), "October 5, 1855");
        assert_eq!(humanize_date("2000-12-01"), "December 1, 2000");
    }

    #[test]
    fn test_humanize_date_year_month() {
        assert_eq!(humanize_date("1855-10"), "October 1855");
        assert_eq!(humanize_date("1902-01"), "January 1902");
    }

    #[test]
    fn test_humanize_date_year_only_unchanged() {
        assert_eq!(humanize_date("1855"), "1855");
    }

    #[test]
    fn test_humanize_date_invalid_passthrough() {
        // Out-of-range month/day, partials, prose, and already-formatted strings
        // all pass through unchanged.
        assert_eq!(humanize_date("2020-13-40"), "2020-13-40");
        assert_eq!(humanize_date("2020-00-10"), "2020-00-10");
        assert_eq!(humanize_date("2020-02-32"), "2020-02-32");
        assert_eq!(humanize_date("circa 1855"), "circa 1855");
        assert_eq!(humanize_date("October 30, 1855"), "October 30, 1855");
        assert_eq!(humanize_date("1855-1-1"), "1855-1-1"); // not zero-padded
        assert_eq!(humanize_date("55-10-30"), "55-10-30"); // 2-digit year
        assert_eq!(humanize_date(""), "");
    }

    #[test]
    fn test_humandate_filter_string_and_passthrough() {
        let args = HashMap::new();
        assert_eq!(
            humandate_filter(&json!("1855-10-30"), &args).unwrap(),
            json!("October 30, 1855")
        );
        // Non-string values pass through unchanged.
        assert_eq!(humandate_filter(&json!(1855), &args).unwrap(), json!(1855));
        assert_eq!(humandate_filter(&json!(null), &args).unwrap(), json!(null));
    }

    #[test]
    fn test_humandate_filter_registered_in_tera() {
        let mut tera = Tera::default();
        tera.register_filter("humandate", humandate_filter);
        tera.add_raw_template("t", "{{ born | humandate }}")
            .unwrap();
        let mut ctx = Context::new();
        ctx.insert("born", "1855-10-30");
        assert_eq!(tera.render("t", &ctx).unwrap(), "October 30, 1855");
    }
}
