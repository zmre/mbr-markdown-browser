use tera::{Context, Tera};

pub struct Templates {
    tera: Tera,
}

impl Templates {
    pub fn new() -> Self {
        let mut tera = Tera::new(".mbr/**/*.html").unwrap_or_default();
        if !tera.get_template_names().any(|x| x == "index.html") {
            let defaultindex = r#"
                <!doctype html>
                <html>
                <head>
                    <title>Templates Work!</title>
                </head>
                <body>
                    {{ markdown | safe}}
                </body>
                </html>
            "#;
            tera.add_raw_template("index.html", defaultindex)
                .expect("Could not add default template");
        }

        Templates { tera }
    }

    pub async fn render_markdown(&self, html: &str) -> Result<String, Box<dyn std::error::Error>> {
        let mut context = Context::new();
        context.insert("markdown", html);
        let html_output = self.tera.render("index.html", &context)?;
        Ok(html_output)
    }
}
