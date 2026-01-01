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

    pub async fn render_markdown(
        &self,
        html: &str,
        frontmatter: HashMap<String, String>,
        extra_context: HashMap<String, serde_json::Value>,
    ) -> Result<String, TemplateError> {
        tracing::debug!("frontmatter: {:?}", &frontmatter);

        // Create JSON from frontmatter BEFORE adding markdown to context
        // This avoids including the large markdown HTML in the frontmatter JSON
        let frontmatter_json =
            serde_json::to_string(&frontmatter).unwrap_or_else(|_| "{}".to_string());

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

    pub async fn render_section(
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
    pub async fn render_home(
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
    (
        "_info_panel.html",
        include_str!("../templates/_info_panel.html"),
    ),
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
