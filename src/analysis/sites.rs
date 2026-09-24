//! Sites in a function body where a value reaches another program or a
//! setting is chosen: text built from values, calls and field assignments.
//! They are syntax only, never API names, and serve only as options for
//! locating a security finding; Jev judges what each one does.
use super::{is_comment, line_of, text};
use std::{collections::BTreeMap, ops::Range};
use tree_sitter::Node;

/// At most this many sites are offered per function, text-building first.
pub const MAX_SITES: usize = 16;
/// Longer site text is clipped; the function source still holds it whole.
const MAX_TEXT: usize = 160;
/// A statement longer than this many lines is shown as the site node alone.
const MAX_STATEMENT_LINES: usize = 3;

#[derive(Clone, Debug, PartialEq)]
pub struct Site {
    pub id: String,
    pub text: String,
    pub line: usize,
    pub end_line: usize,
}

/// How directly a node shows a value reaching another program; lower first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Priority {
    /// Text built from values: a template, f-string, concatenation or
    /// formatting macro.
    BuiltText,
    /// A call with an argument that is not a literal.
    CallWithValue,
    /// Another call, such as one that chooses a setting with literals.
    Call,
    /// An assignment to a field or property.
    FieldAssignment,
}

const LITERALS: &[&str] = &[
    "string",
    "string_literal",
    "interpreted_string_literal",
    "int_literal",
    "nil",
    "raw_string_literal",
    "number",
    "integer",
    "float",
    "integer_literal",
    "float_literal",
    "true",
    "false",
    "null",
    "none",
    "undefined",
    "boolean_literal",
];

const STATEMENTS: &[&str] = &[
    "expression_statement",
    "lexical_declaration",
    "variable_declaration",
    "let_declaration",
    "return_statement",
    "return_expression",
    "short_var_declaration",
    "assignment_statement",
    "var_declaration",
    "defer_statement",
    "go_statement",
];

/// Rust's standard formatting macros build text from their arguments.
const FORMAT_MACROS: &[&str] = &[
    "format",
    "format_args",
    "write",
    "writeln",
    "print",
    "println",
    "eprint",
    "eprintln",
    "concat",
];

/// Go functions that build text from a format string and values.
const GO_FORMAT_CALLS: &[&str] = &["Sprintf", "Sprint", "Sprintln", "Errorf", "Fprintf"];

/// Sites of the body, one per statement, in source order with ids `S1…`.
pub fn in_node(body: Node<'_>, source: &str) -> Vec<Site> {
    let mut best = BTreeMap::<usize, (Priority, Node<'_>)>::new();
    collect(body, body, source, &mut best);
    numbered(best, source)
}

fn numbered(best: BTreeMap<usize, (Priority, Node<'_>)>, source: &str) -> Vec<Site> {
    let mut chosen: Vec<(Priority, Node<'_>)> = best.into_values().collect();
    chosen.sort_by_key(|(priority, node)| (*priority, node.start_byte()));
    chosen.truncate(MAX_SITES);
    chosen.sort_by_key(|(_, node)| node.start_byte());
    chosen
        .into_iter()
        .enumerate()
        .map(|(index, (_, node))| Site {
            id: format!("S{}", index + 1),
            text: clip(text(node, source)),
            line: line_of(source, node.start_byte()),
            end_line: line_of(source, node.end_byte().saturating_sub(1)),
        })
        .collect()
}

/// Top-level statements that run when the module loads and call something,
/// such as `app.use(cors(options))`; definitions, imports and anything inside
/// a function unit are left out.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Setup {
    /// Byte range, first and last line of each statement.
    pub statements: Vec<(Range<usize>, usize, usize)>,
    pub sites: Vec<Site>,
}

const SETUP_STATEMENTS: &[&str] = &[
    "expression_statement",
    "lexical_declaration",
    "variable_declaration",
    "if_statement",
    "with_statement",
];

pub fn setup(root: Node<'_>, source: &str, units: &[Range<usize>]) -> Setup {
    let mut statements = Vec::new();
    let mut best = BTreeMap::<usize, (Priority, Node<'_>)>::new();
    let mut cursor = root.walk();
    for node in root.named_children(&mut cursor) {
        let node = match node.kind() {
            "export_statement" => match node.child_by_field_name("declaration") {
                Some(declaration) => declaration,
                None => continue,
            },
            _ => node,
        };
        let range = node.byte_range();
        if !SETUP_STATEMENTS.contains(&node.kind())
            || units
                .iter()
                .any(|u| u.start < range.end && range.start < u.end)
            || !calls_something(node)
        {
            continue;
        }
        statements.push((
            range,
            line_of(source, node.start_byte()),
            line_of(source, node.end_byte().saturating_sub(1)),
        ));
        collect(node, root, source, &mut best);
    }
    Setup {
        statements,
        sites: numbered(best, source),
    }
}

fn calls_something(node: Node<'_>) -> bool {
    matches!(
        node.kind(),
        "call_expression" | "call" | "new_expression" | "macro_invocation"
    ) || {
        let mut cursor = node.walk();
        node.named_children(&mut cursor).any(calls_something)
    }
}

fn collect<'t>(
    node: Node<'t>,
    body: Node<'t>,
    source: &str,
    best: &mut BTreeMap<usize, (Priority, Node<'t>)>,
) {
    if is_comment(node) {
        return;
    }
    if let Some(priority) = priority(node, source) {
        let shown = statement(node, body, source);
        let entry = best.entry(shown.start_byte()).or_insert((priority, shown));
        entry.0 = entry.0.min(priority);
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect(child, body, source, best);
    }
}

fn priority(node: Node<'_>, source: &str) -> Option<Priority> {
    match node.kind() {
        "template_string" if has_child(node, "template_substitution") => Some(Priority::BuiltText),
        "string" if has_child(node, "interpolation") => Some(Priority::BuiltText),
        "binary_expression" | "binary_operator" if concatenates(node, source) => {
            Some(Priority::BuiltText)
        }
        "macro_invocation" => node
            .child_by_field_name("macro")
            .map(|name| text(name, source).rsplit("::").next().unwrap_or(""))
            .filter(|name| FORMAT_MACROS.contains(name))
            .map(|_| Priority::BuiltText),
        "call_expression"
            if node
                .child_by_field_name("function")
                .filter(|f| f.kind() == "selector_expression")
                .and_then(|f| f.child_by_field_name("field"))
                .is_some_and(|f| GO_FORMAT_CALLS.contains(&text(f, source))) =>
        {
            Some(Priority::BuiltText)
        }
        "call_expression" | "call" | "new_expression" => Some(
            if node
                .child_by_field_name("arguments")
                .is_some_and(|args| has_value(args))
            {
                Priority::CallWithValue
            } else {
                Priority::Call
            },
        ),
        "assignment_expression" | "assignment" | "augmented_assignment_expression"
            if node.child_by_field_name("left").is_some_and(|left| {
                matches!(
                    left.kind(),
                    "member_expression" | "attribute" | "field_expression" | "subscript_expression"
                )
            }) =>
        {
            Some(Priority::FieldAssignment)
        }
        _ => None,
    }
}

fn has_child(node: Node<'_>, kind: &str) -> bool {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).any(|c| c.kind() == kind)
}

/// `+` or `%` joining a string literal with something that is not a literal.
fn concatenates(node: Node<'_>, source: &str) -> bool {
    let operator = node
        .child_by_field_name("operator")
        .map_or("", |o| text(o, source));
    let (Some(left), Some(right)) = (
        node.child_by_field_name("left"),
        node.child_by_field_name("right"),
    ) else {
        return false;
    };
    let string = |n: Node<'_>| {
        matches!(
            n.kind(),
            "string"
                | "string_literal"
                | "template_string"
                | "interpreted_string_literal"
                | "raw_string_literal"
        )
    };
    matches!(operator, "+" | "%")
        && (string(left) || string(right))
        && !(LITERALS.contains(&left.kind()) && LITERALS.contains(&right.kind()))
}

/// Whether an argument list holds something other than literals.
fn has_value(arguments: Node<'_>) -> bool {
    let mut cursor = arguments.walk();
    arguments.named_children(&mut cursor).any(|argument| {
        let value = if argument.kind() == "keyword_argument" {
            argument.child_by_field_name("value")
        } else {
            Some(argument)
        };
        value.is_some_and(|v| !LITERALS.contains(&v.kind()) && !is_comment(v))
    })
}

/// The enclosing statement, so a site reads in context; the node itself when
/// the statement is long or the body is reached first.
fn statement<'t>(node: Node<'t>, body: Node<'t>, source: &str) -> Node<'t> {
    let mut current = node;
    while let Some(parent) = current.parent() {
        if parent.id() == body.id() {
            break;
        }
        if STATEMENTS.contains(&parent.kind()) {
            let lines = line_of(source, parent.end_byte()) - line_of(source, parent.start_byte());
            return if lines < MAX_STATEMENT_LINES {
                parent
            } else {
                node
            };
        }
        current = parent;
    }
    node
}

pub(super) fn clip(value: &str) -> String {
    let collapsed = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= MAX_TEXT {
        return collapsed;
    }
    format!("{}…", collapsed.chars().take(MAX_TEXT).collect::<String>())
}

#[cfg(test)]
mod tests {
    use crate::analysis::units::parse;
    use std::path::Path;

    fn sites(path: &str, source: &str) -> Vec<(String, usize)> {
        let file = parse(Path::new(path), source).unwrap();
        file.units[0]
            .sites
            .iter()
            .map(|s| (s.text.clone(), s.line))
            .collect()
    }

    #[test]
    fn built_text_and_calls_are_sites_in_every_language() {
        let python = sites(
            "a.py",
            "def find(name):\n    sql = f\"SELECT id FROM users WHERE name = '{name}'\"\n    rows = db.execute(sql)\n    log(\"done\")\n    return rows\n",
        );
        assert_eq!(
            python.iter().map(|(_, line)| *line).collect::<Vec<_>>(),
            [2, 3, 4]
        );
        assert!(python[0].0.starts_with("sql = f\"SELECT"));
        let script = sites(
            "a.ts",
            "function run(file: string) {\n  exec(`convert ${file}`, done);\n  el.innerHTML = file;\n}\n",
        );
        assert_eq!(
            script,
            [
                ("exec(`convert ${file}`, done);".to_string(), 2),
                ("el.innerHTML = file;".to_string(), 3)
            ]
        );
        let rust = sites(
            "a.rs",
            "fn load(conn: &Connection, table: &str) -> Result<()> {\n    let sql = format!(\"SELECT * FROM {table}\");\n    conn.execute(&sql, [])?;\n    Ok(())\n}\n",
        );
        assert_eq!(rust.len(), 3, "{rust:?}");
        assert!(rust[0].0.starts_with("let sql = format!"));
    }

    #[test]
    fn module_setup_holds_top_level_calls_outside_functions() {
        let source = "import express from 'express'\nconst app = express()\napp.use(cors({ origin: '*', credentials: true }))\nconst limit = 10\nexport const handler = (req) => send(req)\n";
        let file = parse(Path::new("server.ts"), source).unwrap();
        let lines: Vec<usize> = file.setup.statements.iter().map(|s| s.1).collect();
        assert_eq!(lines, [2, 3]);
        assert_eq!(file.setup.sites.len(), 2);
        assert!(file.setup.sites[1].text.starts_with("app.use(cors("));
        let python = parse(
            Path::new("main.py"),
            "app = FastAPI()\napp.add_middleware(CORSMiddleware, allow_origins=['*'])\n\ndef run():\n    serve(app)\n",
        )
        .unwrap();
        assert_eq!(python.setup.statements.len(), 2);
    }

    #[test]
    fn built_text_is_kept_first_when_a_body_has_many_calls() {
        let mut body = String::from("def many(value):\n");
        for i in 0..30 {
            body.push_str(&format!("    step_{i}(value)\n"));
        }
        body.push_str("    run(\"echo \" + value)\n");
        let found = sites("a.py", &body);
        assert_eq!(found.len(), super::MAX_SITES);
        assert_eq!(found.last().unwrap().0, "run(\"echo \" + value)");
    }
}
