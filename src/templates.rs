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

    pub fn render_markdown(
        &self,
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

        let html_output = self
            .tera
            .read()
            .render("index.html", &context)
            .map_err(|e| TemplateError::RenderFailed {
                template_name: "index.html".to_string(),
                source: e,
            })?;
        Ok(html_output)
    }

    pub fn render_section(
        &self,
        context_data: HashMap<String, serde_json::Value>,
    ) -> Result<String, TemplateError> {
        let mut context = Context::new();
        context_data.iter().for_each(|(k, v)| {
            context.insert(k, v);
        });
        let html_output = self
            .tera
            .read()
            .render("section.html", &context)
            .map_err(|e| TemplateError::RenderFailed {
                template_name: "section.html".to_string(),
                source: e,
            })?;
        Ok(html_output)
    }

    /// Renders the home page (root directory) using home.html template.
    /// This allows users to customize their home page differently from section pages.
    pub fn render_home(
        &self,
        context_data: HashMap<String, serde_json::Value>,
    ) -> Result<String, TemplateError> {
        let mut context = Context::new();
        context_data.iter().for_each(|(k, v)| {
            context.insert(k, v);
        });
        let html_output = self
            .tera
            .read()
            .render("home.html", &context)
            .map_err(|e| TemplateError::RenderFailed {
                template_name: "home.html".to_string(),
                source: e,
            })?;
        Ok(html_output)
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
        let mut context = Context::new();
        context_data.iter().for_each(|(k, v)| {
            context.insert(k, v);
        });
        let html_output = self
            .tera
            .read()
            .render("error.html", &context)
            .map_err(|e| TemplateError::RenderFailed {
                template_name: "error.html".to_string(),
                source: e,
            })?;
        Ok(html_output)
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
        let mut context = Context::new();
        context_data.iter().for_each(|(k, v)| {
            context.insert(k, v);
        });
        let html_output = self.tera.read().render("tag.html", &context).map_err(|e| {
            TemplateError::RenderFailed {
                template_name: "tag.html".to_string(),
                source: e,
            }
        })?;
        Ok(html_output)
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
        let mut context = Context::new();
        context_data.iter().for_each(|(k, v)| {
            context.insert(k, v);
        });
        let html_output = self
            .tera
            .read()
            .render("tag_index.html", &context)
            .map_err(|e| TemplateError::RenderFailed {
                template_name: "tag_index.html".to_string(),
                source: e,
            })?;
        Ok(html_output)
    }
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
}
