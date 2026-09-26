//! Server code a template runs while it renders, when it reads what a client
//! sends or who is signed in: tags that write a value into the page without
//! escaping it, and the scriptlets of a JSP page. Judged by the security
//! rules as one unit, `template code`, as a PHP page script is: RailsGoat's layout writes `raw cookies[:font]` into a
//! style block and its header `current_user.first_name.html_safe`, DVJA's
//! product list `<%= request.getParameter("searchQuery") %>`, and
//! JavaVulnerableLab's pages query the database in scriptlets with request
//! parameters; no rule read any of them.
use super::{
    line_of,
    sites::{Setup, Site, clip},
};
use std::path::Path;

/// Names that read what a client sends, the session or the signed-in user.
const CLIENT: [&str; 12] = [
    "params",
    "cookies",
    "request",
    "session",
    "current_user",
    "getParameter",
    "getHeader",
    "getCookies",
    "getQueryString",
    "req.body",
    "req.query",
    "req.params",
];

/// A tag of `source`: its byte range and its text.
type Tag = (std::ops::Range<usize>, String);

/// The template's code that reads client data, as a setup of statements
/// (one per tag) and sites; empty for other files and templates.
pub fn template_code(path: &Path, source: &str) -> Setup {
    let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let tags: Vec<Tag> = match extension {
        "erb" => tags(source, "<%", "%>")
            .into_iter()
            .filter(|(_, tag)| erb_unescaped(tag))
            .collect(),
        // A JSP page's scriptlets and declarations are one program: once one
        // reads client data, all are shown, since the value it reads may
        // reach a query in another, as a declared method's HQL query did in
        // JavaVulnerableLab.
        "jsp" | "jspf" => {
            let code: Vec<Tag> = tags(source, "<%", "%>")
                .into_iter()
                .filter(|(_, tag)| !tag.starts_with("<%@") && !tag.starts_with("<%--"))
                .collect();
            if code.iter().any(|(_, tag)| reads_client(tag)) {
                return setup(source, &code);
            }
            Vec::new()
        }
        "ejs" => tags(source, "<%", "%>")
            .into_iter()
            .filter(|(_, tag)| tag.starts_with("<%-") && !compact(tag).starts_with("<%-include"))
            .collect(),
        "hbs" | "handlebars" | "mustache" => tags(source, "{{{", "}}}"),
        "twig" => filtered(tags(source, "{{", "}}"), "|raw"),
        _ if crate::components::server_template(path) => {
            filtered(tags(source, "{{", "}}"), "|safe")
        }
        _ => Vec::new(),
    };
    let reads: Vec<Tag> = tags
        .into_iter()
        .filter(|(_, tag)| reads_client(tag))
        .collect();
    setup(source, &reads)
}

fn reads_client(tag: &str) -> bool {
    CLIENT.iter().any(|name| tag.contains(name))
}

/// The tags as a setup: one statement and one site each.
fn setup(source: &str, reads: &[Tag]) -> Setup {
    Setup {
        statements: reads
            .iter()
            .map(|(range, _)| {
                (
                    range.clone(),
                    line_of(source, range.start),
                    line_of(source, range.end.saturating_sub(1)),
                )
            })
            .collect(),
        sites: reads
            .iter()
            .enumerate()
            .map(|(i, (range, tag))| Site {
                id: format!("S{}", i + 1),
                text: clip(tag),
                line: line_of(source, range.start),
                end_line: line_of(source, range.end.saturating_sub(1)),
            })
            .collect(),
        script: true,
        ..Setup::default()
    }
}

/// An ERB tag that writes a value unescaped: `<%== … %>`, `raw`, or
/// `html_safe`.
fn erb_unescaped(tag: &str) -> bool {
    let inner = compact(tag);
    inner.starts_with("<%==")
        || inner.starts_with("<%=raw(")
        || tag
            .trim_start_matches("<%=")
            .trim_start()
            .starts_with("raw ")
        || inner.starts_with("<%=") && inner.contains(".html_safe")
}

fn compact(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

/// The tags that hold a filter such as `|safe`.
fn filtered(tags: Vec<Tag>, filter: &str) -> Vec<Tag> {
    tags.into_iter()
        .filter(|(_, tag)| compact(tag).contains(filter))
        .collect()
}

/// Every span from `open` to the next `close`, both included.
fn tags(source: &str, open: &str, close: &str) -> Vec<Tag> {
    let mut found = Vec::new();
    let mut at = 0;
    while let Some(start) = source[at..].find(open).map(|i| at + i) {
        let Some(end) = source[start + open.len()..]
            .find(close)
            .map(|i| start + open.len() + i + close.len())
        else {
            break;
        };
        found.push((start..end, source[start..end].to_string()));
        at = end;
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(path: &str, source: &str) -> Vec<String> {
        template_code(Path::new(path), source)
            .sites
            .into_iter()
            .map(|s| s.text)
            .collect()
    }

    #[test]
    fn unescaped_writes_and_scriptlets_that_read_client_data_are_template_code() {
        let layout = "<style>body { font-size: <%= raw cookies[:font] %>; }</style>\n<p><%= current_user.first_name.html_safe %></p>\n<p><%= current_user.first_name %></p>\n<%== t('title') %>\n";
        assert_eq!(
            texts("app/views/layouts/application.html.erb", layout),
            [
                "<%= raw cookies[:font] %>",
                "<%= current_user.first_name.html_safe %>"
            ]
        );
        let jsp = "<%@ page import=\"java.sql.*\" %>\n<%-- a list --%>\n<p><%= request.getParameter(\"q\") %></p>\n<%\n  String id = request.getParameter(\"id\");\n  stmt.executeQuery(\"select * from posts where id=\" + id);\n%>\n<p><%= title %></p>\n<%! List find(String q) { return hql(\"from Post where title='\" + q + \"'\"); } %>\n";
        let found = template_code(Path::new("web/posts.jsp"), jsp);
        assert_eq!(
            found.sites.len(),
            4,
            "every scriptlet and declaration of a page that reads the request"
        );
        assert_eq!(found.sites[1].line, 4);
        assert_eq!(found.statements[1].2, 7);
        assert!(
            template_code(Path::new("web/about.jsp"), "<p><%= title %></p>\n")
                .sites
                .is_empty()
        );
        assert!(
            texts(
                "views/app/search.ejs",
                "<%- include('head') %>\n<p><%- req.query.q %></p>\n"
            ) == ["<%- req.query.q %>"]
        );
        assert_eq!(
            texts(
                "core/templates/search.html",
                "<p>{{ request.GET.q|safe }}</p>\n<p>{{ note|safe }}</p>\n"
            ),
            ["{{ request.GET.q|safe }}"]
        );
        assert!(texts("src/app.js", "const a = '<%= params[:x] %>';").is_empty());
    }
}
