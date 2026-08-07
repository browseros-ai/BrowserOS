use serde::Serialize;

const DOM_WALKER_SCRIPT: &str = include_str!("assets/content-markdown.js");

#[derive(Debug, Clone, Default, Serialize)]
pub struct ContentMarkdownOptions {
    pub selector: Option<String>,
    pub viewport_only: Option<bool>,
    pub include_links: Option<bool>,
    pub include_images: Option<bool>,
}

#[derive(Serialize)]
struct InjectedOptions<'a> {
    selector: Option<&'a str>,
    viewport: bool,
    links: bool,
    images: bool,
}

pub fn build_content_markdown_expression(opts: &ContentMarkdownOptions) -> String {
    let injected = InjectedOptions {
        selector: opts.selector.as_deref(),
        viewport: opts.viewport_only.unwrap_or(false),
        links: opts.include_links.unwrap_or(true),
        images: opts.include_images.unwrap_or(false),
    };
    let json = match serde_json::to_string(&injected) {
        Ok(json) => json,
        Err(_err) => "{}".to_string(),
    };
    format!("{DOM_WALKER_SCRIPT}({json})")
}
