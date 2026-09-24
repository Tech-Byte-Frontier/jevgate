//! Literal values written into code: the candidates for hardcoded-value
//! questions. Eligibility skips trivial values (0, 1, 2, one-character or
//! blank strings), documentation and attributes; Jev judges every other value.
use super::{is_comment, line_of, text};
use tree_sitter::Node;

/// At most this many distinct values are listed per function or file.
pub const MAX_LITERALS: usize = 16;
/// Longer values are clipped in the list; the source still holds them whole.
const MAX_TEXT: usize = 120;
/// A constant's value is shown whole up to this length; longer values are
/// shown as their literals so no value is cut off.
const MAX_VALUE: usize = 400;

const LITERAL_KINDS: &[&str] = &[
    "string_literal",
    "interpreted_string_literal",
    "int_literal",
    "raw_string_literal",
    "integer_literal",
    "float_literal",
    "string",
    "template_string",
    "integer",
    "float",
    "number",
];

/// Syntax whose literals are not program values: documentation, attributes,
/// decorators and import paths.
const SKIPPED_KINDS: &[&str] = &[
    "attribute_item",
    "inner_attribute_item",
    "decorator",
    "use_declaration",
    "import_statement",
    "import_from_statement",
    "import_declaration",
    "package_clause",
];

#[derive(Clone, Debug, PartialEq)]
pub struct Literal {
    pub text: String,
    pub line: usize,
}

/// A module-level constant or binding whose value holds a literal.
#[derive(Clone, Debug, PartialEq)]
pub struct Constant {
    pub name: String,
    /// The value's source when it is short enough to show whole.
    pub value: Option<String>,
    pub values: Vec<String>,
    pub line: usize,
    pub end_line: usize,
}

/// Distinct eligible literals under `node`, in source order.
pub fn in_node(node: Node<'_>, source: &str) -> Vec<Literal> {
    let mut found = Vec::new();
    collect(node, source, &mut found);
    found
}

fn collect(node: Node<'_>, source: &str, found: &mut Vec<Literal>) {
    if found.len() == MAX_LITERALS
        || is_comment(node)
        || SKIPPED_KINDS.contains(&node.kind())
        || docstring(node)
    {
        return;
    }
    if LITERAL_KINDS.contains(&node.kind()) {
        let value = text(node, source);
        if eligible(node.kind(), value) && !found.iter().any(|l: &Literal| l.text == value) {
            found.push(Literal {
                text: clip(value),
                line: line_of(source, node.start_byte()),
            });
        }
        return;
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect(child, source, found);
    }
}

/// A Python docstring: a string that is the first statement of a body or module.
fn docstring(node: Node<'_>) -> bool {
    node.kind() == "expression_statement"
        && node.named_child_count() == 1
        && node.named_child(0).is_some_and(|n| n.kind() == "string")
        && node.prev_named_sibling().is_none()
        && node
            .parent()
            .is_some_and(|p| matches!(p.kind(), "block" | "module"))
}

fn eligible(kind: &str, value: &str) -> bool {
    if matches!(
        kind,
        "integer_literal" | "int_literal" | "float_literal" | "integer" | "float" | "number"
    ) {
        let digits = value.trim_end_matches(|c: char| c.is_ascii_alphabetic() || c == '_');
        return !matches!(digits, "0" | "1" | "2" | "0.0" | "1.0" | "2.0");
    }
    let content = value
        .trim_start_matches(['r', 'b', 'f', 'u', 'R', 'B', 'F', 'U', '#'])
        .trim_matches(['"', '\'', '`', '#']);
    content.trim().chars().count() > 1 && !single_escape(content)
}

/// The longest Unicode escape body: braces around six hex digits.
const MAX_UNICODE_ESCAPE: usize = "{10FFFF}".len();

/// One character written as an escape, such as `\n`, `\0`, `\x1b` or `\u{0}`.
fn single_escape(content: &str) -> bool {
    let Some(rest) = content.strip_prefix('\\') else {
        return false;
    };
    let hex = |digits: &str| !digits.is_empty() && digits.chars().all(|c| c.is_ascii_hexdigit());
    rest.chars().count() == 1
        || rest
            .strip_prefix('x')
            .is_some_and(|d| d.len() == 2 && hex(d))
        || rest.strip_prefix('u').is_some_and(|d| {
            hex(d.trim_start_matches('{').trim_end_matches('}')) && d.len() <= MAX_UNICODE_ESCAPE
        })
}

fn clip(value: &str) -> String {
    if value.chars().count() <= MAX_TEXT {
        return value.to_string();
    }
    format!("{}…", value.chars().take(MAX_TEXT).collect::<String>())
}

/// Top-level constants and bindings whose value holds an eligible literal:
/// Rust `const`/`static`, JavaScript and TypeScript `const`/`let`/`var`
/// (exported or not), and Python module assignments. Functions and classes
/// bound to a name are not values.
pub fn constants(root: Node<'_>, source: &str) -> Vec<Constant> {
    let mut found = Vec::new();
    let mut cursor = root.walk();
    for node in root.named_children(&mut cursor) {
        let node = if node.kind() == "export_statement" {
            match node.child_by_field_name("declaration") {
                Some(declaration) => declaration,
                None => continue,
            }
        } else {
            node
        };
        for (name, value) in bindings(node) {
            if found.len() == MAX_LITERALS {
                return found;
            }
            let values = in_node(value, source);
            if values.is_empty() || !is_value(value) {
                continue;
            }
            let whole = text(value, source);
            found.push(Constant {
                name: text(name, source).to_string(),
                value: (whole.chars().count() <= MAX_VALUE).then(|| whole.to_string()),
                values: values.into_iter().map(|l| l.text).collect(),
                line: line_of(source, node.start_byte()),
                end_line: line_of(source, node.end_byte().saturating_sub(1)),
            });
        }
    }
    found
}

/// Name and value nodes a top-level statement binds.
fn bindings(node: Node<'_>) -> Vec<(Node<'_>, Node<'_>)> {
    match node.kind() {
        "const_item" | "static_item" => node
            .child_by_field_name("name")
            .zip(node.child_by_field_name("value"))
            .into_iter()
            .collect(),
        "lexical_declaration" | "variable_declaration" => {
            let mut cursor = node.walk();
            node.named_children(&mut cursor)
                .filter(|d| d.kind() == "variable_declarator")
                .filter_map(|d| {
                    d.child_by_field_name("name")
                        .zip(d.child_by_field_name("value"))
                })
                .collect()
        }
        // Go: `const maxRows = 500` or a parenthesized group of specs.
        "const_declaration" | "var_declaration" => {
            let mut cursor = node.walk();
            let mut specs: Vec<Node<'_>> = node.named_children(&mut cursor).collect();
            if let [list] = specs[..]
                && list.kind() == "var_spec_list"
            {
                let mut inner = list.walk();
                specs = list.named_children(&mut inner).collect();
            }
            specs
                .into_iter()
                .filter(|s| matches!(s.kind(), "const_spec" | "var_spec"))
                .filter_map(|s| {
                    s.child_by_field_name("name")
                        .zip(s.child_by_field_name("value"))
                })
                .collect()
        }
        "expression_statement" => node
            .named_child(0)
            .filter(|a| a.kind() == "assignment")
            .and_then(|a| {
                a.child_by_field_name("left")
                    .zip(a.child_by_field_name("right"))
            })
            .filter(|(left, _)| left.kind() == "identifier")
            .into_iter()
            .collect(),
        _ => Vec::new(),
    }
}

fn is_value(node: Node<'_>) -> bool {
    !matches!(
        node.kind(),
        "arrow_function" | "function_expression" | "function" | "class" | "lambda"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn texts(path: &str, source: &str) -> Vec<String> {
        let tree = crate::syntax::parse(Path::new(path), source)
            .unwrap()
            .unwrap();
        in_node(tree.root_node(), source)
            .into_iter()
            .map(|l| l.text)
            .collect()
    }

    #[test]
    fn trivial_values_documentation_and_attributes_are_not_candidates() {
        let rust = "#[serde(rename = \"kind\")]\nstruct A;\n/// Uses \"docs\".\nfn f() -> u64 {\n    let _ = (0, 1, 2, \",\", \"\", \"\\n\", \"\\u{0}\");\n    connect(\"db.internal:5432\", 30_000)\n}\n";
        assert_eq!(texts("a.rs", rust), ["\"db.internal:5432\"", "30_000"]);
        let python = "import os\n\ndef f():\n    \"\"\"Docstring.\"\"\"\n    return os.environ.get('HOME', '/srv/app')\n";
        assert_eq!(texts("a.py", python), ["'HOME'", "'/srv/app'"]);
    }

    #[test]
    fn repeated_values_are_listed_once() {
        let js =
            "function f(a) {\n  if (a === 'admin') return 42\n  return a === 'admin' ? 42 : 7\n}\n";
        assert_eq!(texts("a.js", js), ["'admin'", "42", "7"]);
    }

    #[test]
    fn module_constants_hold_literals_and_skip_bound_functions() {
        let ts = "export const API = 'https://api.example.com'\nconst handler = () => 'x'\nlet retries = 5\nconst EMPTY = ''\n";
        let tree = crate::syntax::parse(Path::new("a.ts"), ts)
            .unwrap()
            .unwrap();
        let names: Vec<_> = constants(tree.root_node(), ts)
            .into_iter()
            .map(|c| c.name)
            .collect();
        assert_eq!(names, ["API", "retries"]);
        let python = "TIMEOUT = 30\nname = 'service'\ndef f():\n    pass\n";
        let tree = crate::syntax::parse(Path::new("a.py"), python)
            .unwrap()
            .unwrap();
        assert_eq!(constants(tree.root_node(), python).len(), 2);
        let rust = "const LIMIT: usize = 64;\nstatic HOST: &str = \"example.org\";\n";
        let tree = crate::syntax::parse(Path::new("a.rs"), rust)
            .unwrap()
            .unwrap();
        assert_eq!(
            constants(tree.root_node(), rust)[1].value.as_deref(),
            Some("\"example.org\"")
        );
    }
}
