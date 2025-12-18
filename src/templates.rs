use std::{collections::HashMap, path::Path};

use crate::errors::TemplateError;
use tera::{Context, Tera};

#[derive(Clone)]
pub struct Templates {
    tera: Tera,
}

impl Templates {
    pub fn new(root_path: &Path) -> Result<Self, TemplateError> {
        let globs = root_path.join(".mbr/**/*.html");
        let globs_str = globs.to_str().ok_or(TemplateError::InvalidPathEncoding)?;
        let mut tera = Tera::new(globs_str).unwrap_or_else(|e| {
            tracing::warn!(
                "Failed to load user templates from {}: {}. Using built-in defaults.",
                globs_str,
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
    ) -> Result<String, TemplateError> {
        tracing::debug!("frontmatter: {:?}", &frontmatter);
        let mut context = Context::new();
        frontmatter.iter().for_each(|(k, v)| {
            context.insert(k, v);
        });
        context.insert("markdown", html);
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
    ("index.html", include_str!("../templates/index.html")),
    ("section.html", include_str!("../templates/section.html")),
    ("home.html", include_str!("../templates/home.html")),
];
