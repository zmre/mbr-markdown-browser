use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use crate::errors::TemplateError;
use crate::markdown::SimpleMetadata;
use itertools::Itertools;
use parking_lot::RwLock;
use tera::{Context, Tera};

#[derive(Clone)]
pub struct Templates {
    tera: Arc<RwLock<Tera>>,
    /// Path used for template loading (for hot reload)
    template_path: PathBuf,
}

/// Returns true if `dir` contains at least one `.html` file at any depth.
///
/// Only called when zero templates loaded, so it costs nothing on the happy
/// path.
fn contains_html_file(dir: &Path) -> bool {
    walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_map(Result::ok)
        .any(|entry| is_html_file(&entry))
}

/// Returns true when `entry` is a regular file with an `.html` extension.
fn is_html_file(entry: &walkdir::DirEntry) -> bool {
    entry.file_type().is_file()
        && entry
            .path()
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("html"))
}

/// Escapes a serialized JSON payload so it can be embedded verbatim inside an
/// inline `<script>` block.
///
/// `serde_json` leaves `<`, `>` and `&` untouched, so a value containing
/// `</script>` terminates the surrounding block: the rest of the script is
/// re-parsed as markup and every `window.*` assignment in it silently
/// disappears. A benign frontmatter title such as
/// `Avoiding </script> in templates` is enough to trigger it.
///
/// `<` / `>` / `&` are ordinary JSON string escapes, so the
/// decoded value is byte-for-byte unchanged — this only makes the payload
/// inert as markup. The replacements are order-independent because none of
/// the inserted escapes contains `<`, `>` or `&`.
pub fn escape_json_for_script(json: &str) -> String {
    json.replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026")
}

/// Serializes `value` as JSON escaped for an inline `<script>` block.
///
/// Serialization of a `serde_json::Value` cannot fail; the `null` fallback
/// exists only so the emitted JavaScript stays parseable no matter what.
fn json_for_script(value: &serde_json::Value) -> String {
    escape_json_for_script(&serde_json::to_string(value).unwrap_or_else(|_| "null".to_string()))
}

/// Serializes a `prev_page`/`next_page` context entry as a script-safe JSON
/// object, or the literal `null` when the page has no such sibling.
///
/// The `{url, title}` shape is fixed here rather than passed through so the
/// exposed `window.extendedMeta.prevPage` shape stays exactly what the
/// frontend expects, including the empty-string (never `null`) `url` the
/// hand-built JS literal used to produce.
fn nav_page_json(page: Option<&serde_json::Value>) -> String {
    match page {
        Some(page) => json_for_script(&serde_json::json!({
            "url": page.get("url").and_then(|v| v.as_str()).unwrap_or_default(),
            "title": page.get("title").and_then(|v| v.as_str()).unwrap_or_default(),
        })),
        None => "null".to_string(),
    }
}

/// Reads every `.html` file under `dir` as a `(template_name, contents)` pair,
/// sorted by name so load order (and therefore warning order) is deterministic.
///
/// Names mirror `Tera::load_from_glob`: the path relative to `dir`, with `\`
/// unified to `/`. Unreadable files are warned about and skipped.
fn collect_user_templates(dir: &Path) -> Vec<(String, String)> {
    let mut templates: Vec<(String, String)> = walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_map(Result::ok)
        .filter(is_html_file)
        .filter_map(|entry| {
            let name = entry
                .path()
                .strip_prefix(dir)
                .ok()?
                .to_string_lossy()
                .replace('\\', "/");
            let contents = std::fs::read_to_string(entry.path())
                .inspect_err(|e| {
                    tracing::warn!(
                        "Skipping unreadable user template {}: {}",
                        entry.path().display(),
                        e
                    )
                })
                .ok()?;
            Some((name, contents))
        })
        .collect();
    templates.sort_by(|(a, _), (b, _)| a.cmp(b));
    templates
}

/// Loads user templates one file at a time, skipping only the ones that fail.
///
/// Cold path: only reached when the whole-glob load already failed, so the
/// per-candidate `Tera` clone below costs nothing on a healthy repository.
///
/// Files are retried in passes because `add_raw_template` also validates
/// `{% extends %}` chains — a child added before its parent fails the first
/// time and succeeds once the parent is present. A failed `add_raw_template`
/// leaves the offending template inserted (tera 1.20), which would poison every
/// later add, so each attempt runs against a clone that is committed only on
/// success. Templates skipped here fall back to the compiled-in defaults.
fn load_tera_per_file(template_path: &Path) -> Tera {
    let mut tera = Tera::default();
    let mut pending = collect_user_templates(template_path);

    while !pending.is_empty() {
        let attempted = pending.len();
        let mut failed: Vec<(String, String, tera::Error)> = Vec::new();

        for (name, contents) in std::mem::take(&mut pending) {
            let mut candidate = tera.clone();
            match candidate.add_raw_template(&name, &contents) {
                Ok(()) => tera = candidate,
                Err(e) => failed.push((name, contents, e)),
            }
        }

        // No progress this pass means the remainder is genuinely broken rather
        // than merely out of dependency order.
        if failed.len() == attempted {
            for (name, _, e) in failed {
                tracing::warn!(
                    "Skipping malformed user template {}: {}. Using the built-in default instead.",
                    name,
                    e
                );
            }
            break;
        }

        pending = failed.into_iter().map(|(n, c, _)| (n, c)).collect();
    }

    tera
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
        let source_desc = format!("{}", template_path.display());
        let template_path_str = template_path
            .to_str()
            .ok_or(TemplateError::InvalidPathEncoding)?;

        // Build the glob as a string with `/` separators rather than via
        // `Path::join`. On Windows the joined form is
        // `C:\notes\.mbr\**\*.html`, and globwalk (which Tera uses) treats `\`
        // as an escape character, not a separator — so the pattern matches
        // zero files and every user template override is silently ignored.
        // Forward slashes work on all platforms. The rewrite is gated to
        // Windows because `\` is a legal filename character on Unix.
        #[cfg(windows)]
        let glob_base = template_path_str.replace('\\', "/");
        #[cfg(not(windows))]
        let glob_base = template_path_str.to_string();
        let globs = format!("{}/**/*.html", glob_base.trim_end_matches('/'));

        // `Tera::new` parses the whole glob as one unit and returns `Err` if
        // *any* file is malformed, so an unclosed `{% if %}` in a brand new
        // `.mbr/index.html` used to discard a perfectly good `.mbr/_nav.html`
        // too. Keep the glob as the fast path and degrade to per-file loading,
        // which skips only the files that actually fail.
        let mut tera = Tera::new(&globs).unwrap_or_else(|e| {
            tracing::warn!(
                "Failed to load user templates from {} as a set: {}. Retrying per file so one \
                 malformed template does not discard the others.",
                source_desc,
                e
            );
            load_tera_per_file(template_path)
        });

        // `Tera::new` *succeeds* with an empty template set when the glob
        // matches nothing, so a malformed pattern produces no error at all —
        // which is exactly what hid the Windows separator bug above. Only walk
        // the folder in the already-degenerate zero-template case, and only
        // complain when there really are `.html` files being missed (a `.mbr/`
        // containing just `config.toml` is perfectly normal).
        if tera.get_template_names().next().is_none() && contains_html_file(template_path) {
            tracing::warn!(
                "No templates matched glob {} even though {} contains .html files. \
                 Using built-in defaults; user template overrides will be ignored.",
                globs,
                source_desc
            );
        }

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
        frontmatter: SimpleMetadata,
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
        frontmatter: SimpleMetadata,
        extra_context: HashMap<String, serde_json::Value>,
    ) -> Result<String, TemplateError> {
        tracing::debug!("frontmatter: {:?}", &frontmatter);

        // Create JSON from frontmatter BEFORE adding markdown to context
        // This avoids including the large markdown HTML in the frontmatter JSON.
        // Escaped for `<script>` context: see `escape_json_for_script`.
        let frontmatter_json = escape_json_for_script(
            &serde_json::to_string(&frontmatter).unwrap_or_else(|_| "{}".to_string()),
        );

        // The head template used to hand-build these as JavaScript object and
        // string literals, which broke on any `"`/`\` in a heading, a file path
        // or a sibling URL — all of which are legal there. Serializing
        // server-side keeps the emitted script parseable whatever the input.
        let headings_json = extra_context
            .get("headings")
            .map(json_for_script)
            .unwrap_or_else(|| "[]".to_string());
        let file_path_json = extra_context
            .get("file_path")
            .map(json_for_script)
            .unwrap_or_else(|| "\"\"".to_string());
        let prev_page_json = nav_page_json(extra_context.get("prev_page"));
        let next_page_json = nav_page_json(extra_context.get("next_page"));

        let mut context = Context::new();
        frontmatter.iter().for_each(|(k, v)| {
            // `style` is skipped here and inserted below from `body_class_list`,
            // which also folds in `type` — so it has to be inserted even when the
            // frontmatter carries no `style` key at all.
            if k != "style" {
                context.insert(k, v);
            }
        });
        let body_class = body_class_list(&frontmatter);
        // Left undefined when there is nothing to say, so `{% if style %}` in a
        // custom template still distinguishes "no classes" from an empty string.
        if !body_class.is_empty() {
            context.insert("style", &body_class);
        }
        // Add extra context (breadcrumbs, current_dir_name, etc.)
        extra_context.iter().for_each(|(k, v)| {
            context.insert(k, v);
        });
        context.insert("markdown", html);
        context.insert("frontmatter_json", &frontmatter_json);
        context.insert("headings_json", &headings_json);
        context.insert("file_path_json", &file_path_json);
        context.insert("prev_page_json", &prev_page_json);
        context.insert("next_page_json", &next_page_json);

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

/// Builds the `<body>` class list from the `type` and `style` frontmatter keys.
///
/// The OKF `type:` field is folded into `style` rather than surfaced as a new
/// `body_class` variable so repositories that already ship a customized
/// `.mbr/index.html` get the feature without touching their template: the class
/// attribute they render today simply gains the type. The `type` *context*
/// variable is deliberately left exactly as authored — templates gate on its
/// value (`{% if type and type == "person" %}` in `_display_enhancements.html`
/// and `_person_infobox.html`), so handing them a slugified copy would silently
/// stop matching.
///
/// Entries are deduped first-seen, because `type: slides` with `style: slides`
/// is the natural way to write a page that is both and must not emit the class
/// twice. Type leads, styles follow: class order means nothing to CSS, but a
/// fixed one keeps rendered pages byte-stable across runs.
fn body_class_list(frontmatter: &SimpleMetadata) -> String {
    let type_class = frontmatter
        .get("type")
        .and_then(serde_json::Value::as_str)
        .map(|raw| collapse_dashes(&crate::markdown::slugify(raw)))
        .filter(|slug| !slug.is_empty());

    type_class
        .into_iter()
        .chain(frontmatter.get("style").into_iter().flat_map(style_classes))
        .unique()
        .join(" ")
}

/// Expands a `style` frontmatter value into individual class names.
///
/// Strings are split on whitespace rather than passed through whole so that
/// `style: "a b"` and `style: [a, b]` reach the deduper as the same two
/// classes. Non-string array elements are ignored; any other value keeps the
/// historical `to_string()` fallback.
fn style_classes(value: &serde_json::Value) -> Vec<String> {
    match value {
        serde_json::Value::String(s) => split_classes(s),
        serde_json::Value::Array(arr) => arr
            .iter()
            .filter_map(serde_json::Value::as_str)
            .flat_map(split_classes)
            .collect(),
        other => vec![other.to_string()],
    }
}

fn split_classes(s: &str) -> Vec<String> {
    s.split_whitespace().map(str::to_string).collect()
}

/// Squeezes runs of `-` out of a slug and trims them from both ends.
///
/// Applied to the type class only, never to `slugify` itself: `slugify` is
/// shared with heading anchors, whose doubled dashes are frozen because they
/// are the live `#anchor` targets of links already written in existing
/// repositories. A body class carries no such history and is free to be the
/// obvious thing, so `type: "Book Review (2024)"` yields `book-review-2024` —
/// what anyone hand-writing the matching CSS selector would guess — rather than
/// the anchor spelling `book-review--2024`.
fn collapse_dashes(slug: &str) -> String {
    slug.split('-').filter(|part| !part.is_empty()).join("-")
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

    /// User templates in `.mbr/` must actually be picked up by the glob.
    ///
    /// This is the regression guard for the Windows separator bug: building the
    /// pattern with `Path::join` yields `...\.mbr\**\*.html`, globwalk treats
    /// `\` as an escape rather than a separator, and `Tera::new` then *succeeds*
    /// with zero user templates — silently ignoring every override.
    #[test]
    fn test_user_templates_are_loaded_from_mbr_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let mbr_dir = tmp.path().join(".mbr");
        std::fs::create_dir_all(mbr_dir.join("partials")).unwrap();

        // Names deliberately absent from DEFAULT_TEMPLATES, so finding them
        // proves they came from disk rather than the compiled-in fallbacks.
        std::fs::write(mbr_dir.join("zz_custom_page.html"), "<p>custom</p>").unwrap();
        std::fs::write(
            mbr_dir.join("partials").join("zz_nested_page.html"),
            "<p>nested</p>",
        )
        .unwrap();

        let templates = Templates::new(tmp.path(), None).expect("templates should build");
        let tera = templates.tera.read();
        let names: Vec<String> = tera.get_template_names().map(String::from).collect();

        assert!(
            names.iter().any(|n| n == "zz_custom_page.html"),
            "top-level user template should be loaded, got: {names:?}"
        );
        // Asserted by suffix because Tera derives template names from the
        // relative path, whose separator is platform dependent.
        assert!(
            names.iter().any(|n| n.ends_with("zz_nested_page.html")),
            "nested user template should be loaded via `**`, got: {names:?}"
        );
    }

    /// A `.mbr/` folder holding no HTML at all is normal (e.g. only
    /// `config.toml`) and must still produce a working template set from the
    /// compiled-in defaults.
    #[test]
    fn test_templates_fall_back_to_defaults_without_user_html() {
        let tmp = tempfile::tempdir().unwrap();
        let mbr_dir = tmp.path().join(".mbr");
        std::fs::create_dir_all(&mbr_dir).unwrap();
        std::fs::write(mbr_dir.join("config.toml"), "theme = \"amber\"").unwrap();

        let templates = Templates::new(tmp.path(), None).expect("templates should build");
        let tera = templates.tera.read();

        assert!(
            tera.get_template_names().any(|n| n == "index.html"),
            "built-in defaults should still be registered"
        );
    }

    // ------------------------------------------------------------------
    // Body class list (`type` + `style` frontmatter)
    // ------------------------------------------------------------------

    /// Builds a `SimpleMetadata` from `(key, value)` pairs.
    fn fm(pairs: &[(&str, serde_json::Value)]) -> SimpleMetadata {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), v.clone()))
            .collect()
    }

    #[test]
    fn test_body_class_style_string() {
        assert_eq!(
            body_class_list(&fm(&[("style", json!("slides"))])),
            "slides"
        );
    }

    #[test]
    fn test_body_class_style_string_with_spaces() {
        assert_eq!(
            body_class_list(&fm(&[("style", json!("slides other"))])),
            "slides other"
        );
    }

    #[test]
    fn test_body_class_style_array() {
        assert_eq!(
            body_class_list(&fm(&[("style", json!(["slides", "other"]))])),
            "slides other"
        );
        assert_eq!(
            body_class_list(&fm(&[("style", json!(["slides"]))])),
            "slides"
        );
    }

    /// Array elements are themselves split, so the two spellings of the same
    /// class list collapse to the same result — and to the same dedupe keys.
    #[test]
    fn test_body_class_style_array_elements_are_split() {
        assert_eq!(
            body_class_list(&fm(&[("style", json!(["slides other", "third"]))])),
            "slides other third"
        );
    }

    /// Non-string array elements are dropped, matching the historical
    /// `filter_map(as_str)`; other value kinds keep the `to_string()` fallback.
    #[test]
    fn test_body_class_style_non_string_values() {
        assert_eq!(
            body_class_list(&fm(&[("style", json!(["slides", 42, null]))])),
            "slides"
        );
        assert_eq!(body_class_list(&fm(&[("style", json!(null))])), "null");
        assert_eq!(body_class_list(&fm(&[("style", json!(42))])), "42");
    }

    #[test]
    fn test_body_class_style_array_empty() {
        assert_eq!(body_class_list(&fm(&[("style", json!([]))])), "");
    }

    /// The point of the change: `type` alone produces a body class.
    #[test]
    fn test_body_class_type_only() {
        assert_eq!(body_class_list(&fm(&[("type", json!("slides"))])), "slides");
    }

    #[test]
    fn test_body_class_type_is_slugified() {
        assert_eq!(
            body_class_list(&fm(&[("type", json!("Meeting Notes"))])),
            "meeting-notes"
        );
        assert_eq!(
            body_class_list(&fm(&[("type", json!("Field Note!"))])),
            "field-note"
        );
    }

    /// Punctuation *between* words makes `slugify` emit a doubled dash, which
    /// heading anchors are stuck with. The body class is not: `collapse_dashes`
    /// gives the selector the author would actually write.
    #[test]
    fn test_body_class_type_collapses_doubled_dashes() {
        assert_eq!(
            crate::markdown::slugify("Book Review (2024)"),
            "book-review--2024",
            "guards the premise: the shared slugifier really does double here"
        );
        assert_eq!(
            body_class_list(&fm(&[("type", json!("Book Review (2024)"))])),
            "book-review-2024"
        );
        assert_eq!(
            body_class_list(&fm(&[("type", json!("Meeting, Notes"))])),
            "meeting-notes"
        );
    }

    /// Leading and trailing dashes are trimmed, and a type that is nothing but
    /// separators drops out entirely rather than emitting a bare `-`.
    #[test]
    fn test_body_class_type_trims_edge_dashes() {
        assert_eq!(body_class_list(&fm(&[("type", json!("-draft-"))])), "draft");
        assert_eq!(body_class_list(&fm(&[("type", json!("---"))])), "");
    }

    /// Type leads, styles follow.
    #[test]
    fn test_body_class_type_and_style_combined() {
        assert_eq!(
            body_class_list(&fm(&[
                ("style", json!(["wide", "dark"])),
                ("type", json!("person")),
            ])),
            "person wide dark"
        );
    }

    /// Writing a page as both `type: slides` and `style: slides` is natural and
    /// must not emit the class twice.
    #[test]
    fn test_body_class_type_and_style_dedupe() {
        assert_eq!(
            body_class_list(&fm(&[
                ("style", json!("slides")),
                ("type", json!("slides"))
            ])),
            "slides"
        );
        assert_eq!(
            body_class_list(&fm(&[
                ("style", json!("wide slides wide")),
                ("type", json!("slides")),
            ])),
            "slides wide"
        );
    }

    /// A non-string `type` (a list, a number) is not a class name; it is ignored
    /// rather than stringified, so `type: [a, b]` cannot inject markup-ish junk.
    #[test]
    fn test_body_class_non_string_type_ignored() {
        assert_eq!(
            body_class_list(&fm(&[("type", json!(["person", "place"]))])),
            ""
        );
        assert_eq!(body_class_list(&fm(&[("type", json!(42))])), "");
        // A type that slugifies to nothing leaves no empty class behind.
        assert_eq!(body_class_list(&fm(&[("type", json!("!!!"))])), "");
        assert_eq!(
            body_class_list(&fm(&[("type", json!("!!!")), ("style", json!("wide"))])),
            "wide"
        );
    }

    #[test]
    fn test_body_class_neither_field() {
        assert_eq!(body_class_list(&fm(&[("title", json!("Hello"))])), "");
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

    // ------------------------------------------------------------------
    // Script-context escaping (`<script>` payloads in _head_markdown.html)
    // ------------------------------------------------------------------

    /// Renders a markdown page through the real Tera pipeline with the
    /// compiled-in default templates.
    ///
    /// Only the page "chrome" keys the default templates require unconditionally
    /// are defaulted in; everything the assertions care about comes from the
    /// caller.
    fn render_page(
        frontmatter: SimpleMetadata,
        extra_context: HashMap<String, serde_json::Value>,
    ) -> String {
        let mut context = HashMap::from([("sidebar_style".to_string(), json!("auto"))]);
        context.extend(extra_context);

        let tmp = tempfile::tempdir().unwrap();
        let templates = Templates::new(tmp.path(), None).expect("templates should build");
        templates
            .render_markdown("<p>body</p>", frontmatter, context)
            .expect("page should render")
    }

    /// Extracts the JSON/JS text assigned to `field` in the inline head
    /// script, e.g. `js_value(html, "filePath: ")`.
    ///
    /// Line-based, which is sound because JSON never contains a raw newline —
    /// and is itself part of the regression guard: the old hand-built
    /// `window.headings` literal spanned several lines.
    fn js_value<'a>(html: &'a str, prefix: &str) -> &'a str {
        html.lines()
            .find_map(|line| line.trim().strip_prefix(prefix))
            .unwrap_or_else(|| panic!("no line starting with {prefix:?} in:\n{html}"))
            .trim_end()
            .trim_end_matches([',', ';'])
    }

    /// Parses a script payload, proving it is syntactically valid data rather
    /// than something that merely happens to look right.
    fn parse_js_value(html: &str, prefix: &str) -> serde_json::Value {
        let raw = js_value(html, prefix);
        serde_json::from_str(raw)
            .unwrap_or_else(|e| panic!("{prefix:?} payload is not parseable ({e}): {raw}"))
    }

    #[test]
    fn test_escape_json_for_script_neutralizes_markup_characters() {
        let escaped = escape_json_for_script(r#"{"title":"a </script> & <b>"}"#);
        assert!(
            !escaped.contains("</script>"),
            "script terminator must not survive: {escaped}"
        );
        assert!(
            escaped.contains("\\u003c"),
            "`<` must be escaped: {escaped}"
        );
        assert!(
            escaped.contains("\\u003e"),
            "`>` must be escaped: {escaped}"
        );
        assert!(
            escaped.contains("\\u0026"),
            "`&` must be escaped: {escaped}"
        );
    }

    #[test]
    fn test_escape_json_for_script_preserves_decoded_value() {
        // `\uXXXX` is an ordinary JSON escape, so the payload is unchanged as
        // data — only inert as markup.
        let original = json!({"title": "a </script> & <b> \\ \" done"});
        let escaped = escape_json_for_script(&serde_json::to_string(&original).unwrap());
        let round_tripped: serde_json::Value = serde_json::from_str(&escaped).unwrap();
        assert_eq!(round_tripped, original);
    }

    proptest::proptest! {
        /// Two invariants that together define the helper: the output can never
        /// carry markup-significant characters, and it always decodes back to
        /// the exact input value.
        #[test]
        fn prop_escape_json_for_script_is_inert_and_lossless(
            key in "\\PC*",
            value in "\\PC*",
        ) {
            let original = json!({ key: value });
            let escaped = escape_json_for_script(&serde_json::to_string(&original).unwrap());

            proptest::prop_assert!(
                !escaped.contains(['<', '>', '&']),
                "escaped payload must contain no markup characters: {escaped}"
            );
            let round_tripped: serde_json::Value = serde_json::from_str(&escaped).unwrap();
            proptest::prop_assert_eq!(round_tripped, original);
        }
    }

    /// A frontmatter value containing `</script>` used to terminate the inline
    /// script block, silently dropping `window.frontmatter`, `window.headings`
    /// and `window.extendedMeta`.
    #[test]
    fn test_frontmatter_json_cannot_break_out_of_script_block() {
        let frontmatter = SimpleMetadata::from([
            (
                "title".to_string(),
                json!("Avoiding </script> in templates"),
            ),
            ("note".to_string(), json!("<img src=x onerror=alert(1)>")),
        ]);
        let html = render_page(frontmatter, HashMap::new());

        let payload = js_value(&html, "window.frontmatter = ");
        assert!(
            !payload.contains("</script>"),
            "frontmatter payload must not contain a raw script terminator: {payload}"
        );
        assert!(
            payload.contains("\\u003c/script\\u003e"),
            "expected the escaped terminator in: {payload}"
        );
        assert!(
            !payload.contains("<img"),
            "raw markup must not survive into the script: {payload}"
        );

        // Still valid data with the original values intact.
        let parsed = parse_js_value(&html, "window.frontmatter = ");
        assert_eq!(parsed["title"], json!("Avoiding </script> in templates"));
        assert_eq!(parsed["note"], json!("<img src=x onerror=alert(1)>"));
    }

    /// `| safe` on `title` let a frontmatter title emit live markup and
    /// mangled every benign `&`; the explicit `| escape` on the neighboring
    /// `<meta>` double-escaped because Tera autoescape runs after filters.
    #[test]
    fn test_title_is_html_escaped_exactly_once() {
        let html = render_page(
            SimpleMetadata::from([("title".to_string(), json!("Tips & Tricks"))]),
            HashMap::new(),
        );
        assert!(
            html.contains("<title>Tips &amp; Tricks</title>"),
            "title should be escaped exactly once, got:\n{html}"
        );
        assert!(
            !html.contains("&amp;amp;"),
            "no value should be double-escaped, got:\n{html}"
        );
    }

    #[test]
    fn test_title_markup_is_not_emitted_live() {
        let html = render_page(
            SimpleMetadata::from([(
                "title".to_string(),
                json!("</title><img src=x onerror=alert(1)>"),
            )]),
            HashMap::new(),
        );
        assert!(
            !html.contains("<img src=x"),
            "a frontmatter title must never produce live markup, got:\n{html}"
        );
        assert!(
            html.contains("&lt;img src=x onerror=alert(1)&gt;"),
            "title should render as escaped text, got:\n{html}"
        );
    }

    /// `"` and `\` are legal in filenames, and the head script used to splice
    /// the raw path into a JS string literal — a syntax error that killed the
    /// whole block.
    #[test]
    fn test_file_path_and_sibling_urls_are_json_serialized() {
        let file_path = r#"docs/He said "hi" \ again.md"#;
        let extra = HashMap::from([
            ("file_path".to_string(), json!(file_path)),
            (
                "prev_page".to_string(),
                json!({"url": r#"/a"b\c/"#, "title": r#"A "quoted" & <angled> title"#}),
            ),
            (
                "next_page".to_string(),
                json!({"url": "/plain/", "title": "Plain"}),
            ),
        ]);
        let html = render_page(SimpleMetadata::new(), extra);

        assert_eq!(parse_js_value(&html, "filePath: "), json!(file_path));

        let prev = parse_js_value(&html, "prevPage: ");
        assert_eq!(prev["url"], json!(r#"/a"b\c/"#));
        assert_eq!(prev["title"], json!(r#"A "quoted" & <angled> title"#));

        let next = parse_js_value(&html, "nextPage: ");
        assert_eq!(next["url"], json!("/plain/"));
        assert_eq!(next["title"], json!("Plain"));
    }

    /// Headings were emitted as a hand-built multi-line JS literal with
    /// `| escape`, so `&` arrived at the frontend as `&amp;` and a `"` in a
    /// heading broke the script.
    #[test]
    fn test_headings_are_json_serialized() {
        let extra = HashMap::from([(
            "headings".to_string(),
            json!([
                {"level": 2, "id": "q-and-a", "text": r#"Q & A: "the ' quote" </script>"#},
                {"level": 3, "id": "plain", "text": "Plain"},
            ]),
        )]);
        let html = render_page(SimpleMetadata::new(), extra);

        let headings = parse_js_value(&html, "window.headings = ");
        assert_eq!(headings[0]["level"], json!(2));
        assert_eq!(headings[0]["id"], json!("q-and-a"));
        assert_eq!(
            headings[0]["text"],
            json!(r#"Q & A: "the ' quote" </script>"#),
            "heading text must round-trip verbatim, not HTML-entity encoded"
        );
        assert_eq!(headings[1]["text"], json!("Plain"));
        assert!(
            !js_value(&html, "window.headings = ").contains("</script>"),
            "heading text must not terminate the script block"
        );
    }

    /// Callers that supply no page metadata (CLI stdout mode) must still emit
    /// the exact `window.*` shape the frontend expects.
    #[test]
    fn test_script_payload_defaults_without_extra_context() {
        let html = render_page(SimpleMetadata::new(), HashMap::new());
        assert_eq!(parse_js_value(&html, "window.headings = "), json!([]));
        assert_eq!(parse_js_value(&html, "filePath: "), json!(""));
        assert!(!html.contains("prevPage:"));
        assert!(!html.contains("nextPage:"));
        // Numeric fields keep their unquoted-key literal form.
        assert!(html.contains("fleschReadingEase: null"));
        assert!(html.contains("wordCount: 0"));
    }

    // ------------------------------------------------------------------
    // Per-file user template loading
    // ------------------------------------------------------------------

    /// `Tera::new` parses the whole glob as one unit, so a single malformed
    /// file used to discard *every* `.mbr/` override and silently swap in the
    /// built-in defaults.
    #[test]
    fn test_malformed_user_template_falls_back_to_defaults() {
        let tmp = tempfile::tempdir().unwrap();
        let mbr_dir = tmp.path().join(".mbr");
        std::fs::create_dir_all(&mbr_dir).unwrap();

        // A valid override of a built-in partial...
        std::fs::write(mbr_dir.join("_nav.html"), "<nav>ZZ_CUSTOM_NAV</nav>").unwrap();
        // ...next to a file with an unclosed `{% if %}`.
        std::fs::write(
            mbr_dir.join("index.html"),
            "{% if broken %}<p>ZZ_BROKEN</p>",
        )
        .unwrap();

        let templates = Templates::new(tmp.path(), None)
            .expect("a malformed user template must not fail construction");
        let html = templates
            .render_markdown(
                "<p>body</p>",
                SimpleMetadata::new(),
                HashMap::from([("sidebar_style".to_string(), json!("auto"))]),
            )
            .expect("index.html should render from the built-in default");

        assert!(
            html.contains("ZZ_CUSTOM_NAV"),
            "the valid override must survive its malformed sibling, got:\n{html}"
        );
        assert!(
            html.contains("<mbr-nav></mbr-nav>"),
            "the built-in index.html should be used for the malformed override, got:\n{html}"
        );
        assert!(
            !html.contains("ZZ_BROKEN"),
            "the malformed template must not be loaded, got:\n{html}"
        );
    }

    /// A user template that `{% extends %}` another must load regardless of
    /// which file the per-file loader reaches first.
    #[test]
    fn test_per_file_loading_resolves_inheritance_regardless_of_order() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        // `child.html` sorts before `zz_base.html`, so it is attempted first.
        std::fs::write(dir.join("child.html"), "{% extends \"zz_base.html\" %}").unwrap();
        std::fs::write(dir.join("zz_base.html"), "ZZ_BASE_OUTPUT").unwrap();
        std::fs::write(dir.join("busted.html"), "{% if never_closed %}oops").unwrap();

        let tera = load_tera_per_file(dir);

        assert_eq!(
            tera.render("child.html", &Context::new()).unwrap(),
            "ZZ_BASE_OUTPUT"
        );
        assert!(
            tera.get_template("busted.html").is_err(),
            "the malformed template should be skipped"
        );
    }

    // ------------------------------------------------------------------
    // Rendered `<body>` class attribute
    // ------------------------------------------------------------------

    /// Extracts the rendered `<body ...>` tag.
    fn body_tag(html: &str) -> &str {
        html.lines()
            .map(str::trim)
            .find(|line| line.starts_with("<body"))
            .unwrap_or_else(|| panic!("no <body> tag in:\n{html}"))
    }

    #[test]
    fn test_rendered_body_class_from_type_alone() {
        let html = render_page(
            SimpleMetadata::from([("type".to_string(), json!("slides"))]),
            HashMap::new(),
        );
        assert_eq!(body_tag(&html), r#"<body class="slides">"#);
    }

    #[test]
    fn test_rendered_body_class_from_type_and_style() {
        let html = render_page(
            SimpleMetadata::from([
                ("type".to_string(), json!("Meeting Notes")),
                ("style".to_string(), json!(["wide", "dark"])),
            ]),
            HashMap::new(),
        );
        assert_eq!(body_tag(&html), r#"<body class="meeting-notes wide dark">"#);
    }

    /// Arrays used to render as `[a, b]` and were patched up by a `replace`
    /// filter in `index.html`; the builder now hands the template a plain
    /// space-separated string, so no bracket may reach the attribute.
    #[test]
    fn test_rendered_body_class_from_style_array_has_no_brackets() {
        let html = render_page(
            SimpleMetadata::from([("style".to_string(), json!(["slides", "other"]))]),
            HashMap::new(),
        );
        assert_eq!(body_tag(&html), r#"<body class="slides other">"#);
    }

    #[test]
    fn test_rendered_body_has_no_class_without_type_or_style() {
        let html = render_page(SimpleMetadata::new(), HashMap::new());
        assert_eq!(body_tag(&html), "<body>");
    }

    /// The `type` context variable must survive untouched: templates gate on
    /// `{% if type and type == "person" %}`, so folding a slug into it (or
    /// dropping it in favor of the body class) silently disables the person
    /// infobox and the genealogy element.
    #[test]
    fn test_type_context_variable_is_passed_through_unmodified() {
        let html = render_page(
            SimpleMetadata::from([
                ("type".to_string(), json!("person")),
                ("title".to_string(), json!("Ada Lovelace")),
                ("born".to_string(), json!("1815-12-10")),
            ]),
            HashMap::new(),
        );

        assert!(
            html.contains("mbr-person-infobox"),
            "person infobox should render for `type: person`, got:\n{html}"
        );
        assert!(
            html.contains("<mbr-genealogy></mbr-genealogy>"),
            "genealogy element should render for `type: person`, got:\n{html}"
        );
        // …and the same value still reaches the body class.
        assert_eq!(body_tag(&html), r#"<body class="person">"#);
        assert_eq!(
            parse_js_value(&html, "window.frontmatter = ")["type"],
            json!("person"),
            "frontmatter payload should carry the authored type"
        );
    }
}
