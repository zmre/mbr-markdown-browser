use scraper::{Html, Selector};

// TODO: use actual oembed for some links and make a visual link for others with image and title and description
// draw inspiration from https://crates.io/crates/oembed-rs but it needs to be redone
// I think I just need a few things like the youtube endpoint (and I can use their html directly) and maybe giphy

#[derive(Default)]
pub struct PageInfo {
    pub url: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub image: Option<String>,
}

impl PageInfo {
    pub async fn new_from_url(url: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let response = reqwest::get(url).await?;
        let body = response.text().await?;
        let document = Html::parse_document(&body);
        let mut title: Option<String> = None;
        let mut image: Option<String> = None;
        let mut description: Option<String> = None;
        let meta_selector = Selector::parse("meta").unwrap();
        for element in document.select(&meta_selector) {
            if let Some(property) = element.value().attr("property") {
                match property {
                    "og:title" => {
                        if let Some(content) = element.value().attr("content") {
                            title = Some(html_escape::encode_text(content).to_string());
                        }
                    }
                    "og:image" => {
                        if let Some(content) = element.value().attr("content") {
                            image = Some(content.to_string());
                        }
                    }
                    "og:description" => {
                        if let Some(content) = element.value().attr("content") {
                            description = Some(content.to_string());
                        }
                    }
                    _ => {}
                }
            }
        }
        Ok(PageInfo {
            url: url.to_string(),
            title,
            description,
            image,
        })
    }

    pub fn text(&self) -> String {
        format!(
            "{}: {}",
            self.title.clone().unwrap_or("no title".to_string()),
            self.url
        )
    }

    pub fn html(&self) -> String {
        // TODO: make this a table with image on left, title on right, and description under title.  Or else use a custom element and
        // just pass all the things to the custom-element web component to let something else handle it
        format!(
            "<a href='{}'>{}</a>",
            &self.url,
            self.title.clone().unwrap_or(self.url.clone())
        )
    }
}
