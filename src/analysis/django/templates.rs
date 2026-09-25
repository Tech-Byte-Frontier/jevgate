//! Templates that write values without escaping them, and the views that
//! render them.
use std::path::Path;

/// A Django template that writes values without escaping them: `|safe`,
/// `|safeseq` or `{% autoescape off %}`. A view that renders it by name is
/// sent with those lines, since the markup a view's values reach is written
/// there, not in the view.
#[derive(Clone, Debug, PartialEq)]
pub struct Template {
    /// The name views render it by, such as `blog/post.html`.
    pub name: String,
    pub path: std::path::PathBuf,
    /// Its unescaped lines, as `line: text`.
    pub unescaped: Vec<String>,
}

/// Unescaped lines shown per template, at most.
const UNESCAPED_LINES: usize = 8;
/// Templates shown with one view, at most.
pub const TEMPLATES: usize = 3;

/// The name a template under a `templates` directory is rendered by: the
/// path after the last `templates` part.
pub fn template_name(relative: &Path) -> Option<String> {
    let parts: Vec<&str> = relative.iter().filter_map(|p| p.to_str()).collect();
    let at = parts.iter().rposition(|p| *p == "templates")?;
    let extension = relative.extension().and_then(|e| e.to_str())?;
    (at + 1 < parts.len() && matches!(extension, "html" | "htm" | "txt" | "xml" | "jinja" | "j2"))
        .then(|| parts[at + 1..].join("/"))
}

/// The lines of a template that output values without escaping.
pub fn unescaped_lines(text: &str) -> Vec<String> {
    text.lines()
        .enumerate()
        .filter(|(_, line)| {
            let compact: String = line.chars().filter(|c| !c.is_whitespace()).collect();
            compact.contains("|safe") || compact.contains("{%autoescapeoff%}")
        })
        .take(UNESCAPED_LINES)
        .map(|(index, line)| {
            format!(
                "{}: {}",
                index + 1,
                crate::analysis::sites::clip(line.trim())
            )
        })
        .collect()
}

/// The templates a function names in a string literal, such as
/// `render(request, 'blog/post.html', …)` or `template_name = "blog/post.html"`.
pub fn rendered<'t>(source: &str, templates: &'t [Template]) -> Vec<&'t Template> {
    templates
        .iter()
        .filter(|t| {
            source.contains(&format!("'{}'", t.name)) || source.contains(&format!("\"{}\"", t.name))
        })
        .collect()
}
