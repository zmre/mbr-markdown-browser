use std::{collections::HashMap, path::Path};

use crate::errors::TemplateError;
use tera::{Context, Tera};

#[derive(Clone)]
pub struct Templates {
    tera: Tera,
}

impl Templates {
    /// Creates a new Templates instance.
    ///
    /// Template loading priority:
    /// 1. If `template_folder` is provided, load from `{template_folder}/**/*.html`
    /// 2. Otherwise, load from `{root_path}/.mbr/**/*.html`
    /// 3. Fall back to compiled defaults for any missing templates
    pub fn new(root_path: &Path, template_folder: Option<&Path>) -> Result<Self, TemplateError> {
        let (globs, source_desc) = if let Some(tf) = template_folder {
            (tf.join("**/*.html"), format!("template folder {}", tf.display()))
        } else {
            (root_path.join(".mbr/**/*.html"), format!(".mbr in {}", root_path.display()))
        };

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
                tera.add_raw_template(name, tpl)?;
            }
        }

        Ok(Templates { tera })
    }

    pub async fn render_markdown(
        &self,
        html: &str,
        frontmatter: HashMap<String, String>,
        extra_context: HashMap<String, serde_json::Value>,
    ) -> Result<String, TemplateError> {
        tracing::debug!("frontmatter: {:?}", &frontmatter);

        // Create JSON from frontmatter BEFORE adding markdown to context
        // This avoids including the large markdown HTML in the frontmatter JSON
        let frontmatter_json = serde_json::to_string(&frontmatter).unwrap_or_else(|_| "{}".to_string());

        let mut context = Context::new();
        frontmatter.iter().for_each(|(k, v)| {
            context.insert(k, v);
        });
        // Add extra context (breadcrumbs, current_dir_name, etc.)
        extra_context.iter().for_each(|(k, v)| {
            context.insert(k, v);
        });
        context.insert("markdown", html);
        context.insert("frontmatter_json", &frontmatter_json);

        let html_output = self.tera.render("index.html", &context).map_err(|e| {
            TemplateError::RenderFailed {
                template_name: "index.html".to_string(),
                source: e,
            }
        })?;
        Ok(html_output)
    }

    pub async fn render_section(
        &self,
        context_data: HashMap<String, serde_json::Value>,
    ) -> Result<String, TemplateError> {
        let mut context = Context::new();
        context_data.iter().for_each(|(k, v)| {
            context.insert(k, v);
        });
        let html_output = self.tera.render("section.html", &context).map_err(|e| {
            TemplateError::RenderFailed {
                template_name: "section.html".to_string(),
                source: e,
            }
        })?;
        Ok(html_output)
    }

    /// Renders the home page (root directory) using home.html template.
    /// This allows users to customize their home page differently from section pages.
    pub async fn render_home(
        &self,
        context_data: HashMap<String, serde_json::Value>,
    ) -> Result<String, TemplateError> {
        let mut context = Context::new();
        context_data.iter().for_each(|(k, v)| {
            context.insert(k, v);
        });
        let html_output = self.tera.render("home.html", &context).map_err(|e| {
            TemplateError::RenderFailed {
                template_name: "home.html".to_string(),
                source: e,
            }
        })?;
        Ok(html_output)
    }
}

const DEFAULT_TEMPLATES: &[(&str, &str)] = &[
    // Partials (underscore prefix indicates internal-only templates)
    ("_head.html", include_str!("../templates/_head.html")),
    (
        "_head_markdown.html",
        include_str!("../templates/_head_markdown.html"),
    ),
    ("_nav.html", include_str!("../templates/_nav.html")),
    ("_footer.html", include_str!("../templates/_footer.html")),
    ("_scripts.html", include_str!("../templates/_scripts.html")),
    (
        "_scripts_markdown.html",
        include_str!("../templates/_scripts_markdown.html"),
    ),
    // Main templates
    ("index.html", include_str!("../templates/index.html")),
    ("section.html", include_str!("../templates/section.html")),
    ("home.html", include_str!("../templates/home.html")),
];
