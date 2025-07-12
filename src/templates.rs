use std::path::Path;

use tera::{Context, Tera};

pub struct Templates {
    tera: Tera,
}

impl Templates {
    pub fn new(root_path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let globs = root_path.join(".mbr/**/*.html");
        let mut tera = Tera::new(globs.to_str().unwrap()).unwrap_or_default();

        for (name, tpl) in DEFAULT_TEMPLATES.iter() {
            if tera.get_template(name).is_err() {
                tera.add_raw_template(name, tpl)?;
            }
        }

        Ok(Templates { tera })
    }

    pub async fn render_markdown(&self, html: &str) -> Result<String, Box<dyn std::error::Error>> {
        let mut context = Context::new();
        context.insert("markdown", html);
        let html_output = self.tera.render("index.html", &context)?;
        Ok(html_output)
    }
}

const DEFAULT_TEMPLATES: &[(&str, &str)] =
    &[("index.html", include_str!("../templates/index.html"))];
