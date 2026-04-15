//! Shared base HTML layout used by both skeet-inspect and skeet-feed.
//!
//! The layout emits a minimal HTML shell and loads the vendored htmx script
//! from the static files URL. Consumer crates pre-render their page body
//! into an HTML string and wrap it with [`BaseLayout`].

use cot::Template;

/// Base HTML layout. `content` is treated as pre-rendered HTML and is not
/// escaped; callers are responsible for rendering their page body via a
/// child template before wrapping it with this layout.
#[derive(Debug, Template)]
#[template(
    source = r#"<!DOCTYPE html>
<html lang="en">
    <head>
        <meta charset="UTF-8">
        <meta name="viewport" content="width=device-width, initial-scale=1.0">
        <title>{{ title }}</title>
        <script src="/static/htmx.min.js" defer></script>
        <style>
            body {
                font-family: system-ui, sans-serif;
                max-width: 1400px;
                margin: 2rem auto;
                padding: 0 1rem;
            }
            h1 { text-align: center; }
            nav { text-align: center; margin-bottom: 1rem; }
            .empty { color: #666; text-align: center; margin-top: 2rem; }
            table { width: 100%; border-collapse: collapse; }
            th, td { border: 1px solid #ddd; padding: 0.5rem; vertical-align: top; }
            th { background: #f5f5f5; text-align: left; }
            td.id { font-family: monospace; font-size: 0.85rem; white-space: nowrap; }
            td.annotated img { max-width: 300px; max-height: 300px; }
            td.version { font-family: monospace; font-size: 0.85rem; }
            td.score { font-family: monospace; font-size: 0.85rem; text-align: right; }
        </style>
    </head>
    <body>
{{ content|safe }}
    </body>
</html>
"#,
    ext = "html"
)]
pub struct BaseLayout<'a> {
    pub title: &'a str,
    pub content: &'a str,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_title_and_body() {
        let layout = BaseLayout {
            title: "Hello",
            content: "<p>World</p>",
        };
        let html = layout.render().expect("render");
        assert!(html.contains("<title>Hello</title>"));
        assert!(html.contains("<p>World</p>"));
        assert!(html.contains("src=\"/static/htmx.min.js\""));
    }
}
