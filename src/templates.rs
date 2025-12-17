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
        let mut tera = Tera::new(globs_str).unwrap_or_default();

        for (name, tpl) in DEFAULT_TEMPLATES.iter() {
            if tera.get_template(name).is_err() {
                println!("Adding default template {}", name);
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
        eprintln!("frontmatter: {:?}", &frontmatter);
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
}

const DEFAULT_TEMPLATES: &[(&str, &str)] =
    &[("index.html", include_str!("../templates/index.html"))];
