//! Views a Node server renders by name, such as Express's
//! `res.render('app/products', …)`: templates under a `views` directory,
//! named without their extension, with the lines that write values without
//! escaping in each engine's syntax. A handler that renders one is sent
//! with those lines, as a Django view is (`django::Template`): DVNA's
//! reflected and stored XSS sat in `<%- output.searchTerm %>` and the
//! product fields of `views/app/products.ejs`, which no rule read.
use super::django::Template;
use std::path::Path;

/// Unescaped lines shown per view, at most.
const UNESCAPED_LINES: usize = 8;

/// The name a view under a `views` directory is rendered by: the path after
/// the last `views` part, without its extension, as in `app/products`.
pub fn view_name(relative: &Path) -> Option<String> {
    let extension = relative.extension().and_then(|e| e.to_str())?;
    if unescaped_marks(extension).is_empty() {
        return None;
    }
    let name = super::django::name_under(relative, "views")?;
    Some(name[..name.len() - extension.len() - 1].to_string())
}

/// How each engine writes a value without escaping it: EJS `<%- … %>`,
/// Handlebars and Mustache `{{{ … }}}`, Pug `!=` and `!{…}`, and
/// Nunjucks's and Swig's `|safe` filter or turned-off autoescaping.
fn unescaped_marks(extension: &str) -> &'static [&'static str] {
    match extension {
        "ejs" => &["<%-"],
        "hbs" | "handlebars" | "mustache" => &["{{{"],
        "pug" | "jade" => &["!=", "!{"],
        "njk" | "nunjucks" | "html" | "swig" => &[
            "|safe",
            "autoescape false",
            "autoescape off",
            "autoescape=false",
        ],
        _ => &[],
    }
}

/// The view at `relative`, when it writes a value without escaping.
pub fn view(relative: &Path, text: &str) -> Option<Template> {
    let name = view_name(relative)?;
    let extension = relative.extension().and_then(|e| e.to_str())?;
    let marks = unescaped_marks(extension);
    let unescaped: Vec<String> = text
        .lines()
        .enumerate()
        .filter(|(_, line)| {
            let compact: String = line.chars().filter(|c| !c.is_whitespace()).collect();
            let spaced = line.to_ascii_lowercase();
            marks.iter().any(|mark| {
                let found = if mark.contains(' ') {
                    spaced.contains(mark)
                } else {
                    compact.contains(mark)
                };
                // `<%- include('header') %>` pulls in another view.
                found && !(*mark == "<%-" && compact.contains("<%-include"))
            })
        })
        .take(UNESCAPED_LINES)
        .map(|(index, line)| format!("{}: {}", index + 1, super::sites::clip(line.trim())))
        .collect();
    (!unescaped.is_empty()).then(|| Template {
        name,
        path: relative.to_path_buf(),
        unescaped,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn views_are_named_as_express_renders_them_and_list_unescaped_lines() {
        let products = "<h1>Products</h1>\n<%- include('../common/head') %>\n<p>Search: <%- output.searchTerm %></p>\n<td><%= product.name %></td>\n";
        let found = view(Path::new("views/app/products.ejs"), products).unwrap();
        assert_eq!(found.name, "app/products");
        assert_eq!(
            found.unescaped,
            ["3: <p>Search: <%- output.searchTerm %></p>"]
        );
        assert!(view(Path::new("views/app/list.ejs"), "<td><%= name %></td>\n").is_none());
        assert_eq!(
            view_name(Path::new("server/views/email/reset.hbs")).as_deref(),
            Some("email/reset")
        );
        assert!(view_name(Path::new("views/style.css")).is_none());
        let swig = "{% autoescape false %}\n<p>{{ user.firstName }}</p>\n{% endautoescape %}\n";
        assert_eq!(
            view(Path::new("app/views/profile.html"), swig)
                .unwrap()
                .unescaped,
            ["1: {% autoescape false %}"]
        );
    }
}
