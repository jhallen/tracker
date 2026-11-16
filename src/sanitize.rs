//! Sanitising of user-supplied rich text from the editor.

use std::collections::HashSet;

/// Clean HTML produced by the in-browser editor down to a safe subset.
/// Plain text (no-JS fallback textarea) is escaped and line breaks kept.
pub fn clean_rich_text(input: &str) -> String {
    let input = input.trim();
    if !input.contains('<') {
        let escaped = crate::error::html_escape(input);
        return escaped
            .split("\n\n")
            .map(|p| format!("<p>{}</p>", p.trim().replace('\n', "<br>")))
            .collect::<Vec<_>>()
            .join("");
    }
    let tags: HashSet<&str> = [
        "p", "br", "div", "span", "b", "strong", "i", "em", "u", "s", "strike", "ul", "ol",
        "li", "h3", "h4", "blockquote", "code", "pre", "a", "hr",
    ]
    .into_iter()
    .collect();
    ammonia::Builder::default()
        .tags(tags)
        .link_rel(Some("noopener noreferrer nofollow"))
        .url_schemes(["http", "https", "mailto", "tel"].into_iter().collect())
        .clean(input)
        .to_string()
}

/// True if the HTML has no visible text.
pub fn is_blank_html(html: &str) -> bool {
    let text = ammonia::Builder::empty().clean(html).to_string();
    text.replace("&nbsp;", " ").trim().is_empty()
}

/// Trim and turn empty strings into `None`.
pub fn opt(s: Option<&String>) -> Option<String> {
    s.map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}
