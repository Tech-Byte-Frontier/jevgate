//! PHP syntax the shared analysis reads differently: the names a `use`
//! declaration imports, callees of its call expressions, callbacks
//! registered through calls (`$app->get('/users', function (…) {…})`,
//! `Route::get(…)`), and the top-level statements of a page script, which
//! run on every request.
use super::text;
use std::collections::BTreeSet;
use tree_sitter::Node;

/// Call expressions, whose callee is a `function` or `name` field.
pub(crate) const CALLS: &[&str] = &[
    "function_call_expression",
    "member_call_expression",
    "nullsafe_member_call_expression",
    "scoped_call_expression",
];

/// Constructs that write text into the page or run another file: sites
/// even without a call.
pub(crate) const OUTPUT: &[&str] = &[
    "echo_statement",
    "print_intrinsic",
    "include_expression",
    "include_once_expression",
    "require_expression",
    "require_once_expression",
];

/// Top-level statements that define or import rather than run.
const DEFINITIONS: &[&str] = &[
    "php_tag",
    "php_end_tag",
    "text_interpolation",
    "text",
    "function_definition",
    "class_declaration",
    "interface_declaration",
    "trait_declaration",
    "enum_declaration",
    "namespace_use_declaration",
    "declare_statement",
    "const_declaration",
    "empty_statement",
];

/// Whether this file is PHP, by its extension.
pub(crate) fn file(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| matches!(e, "php" | "phtml"))
}

/// The node that names what a PHP call or `new` expression calls.
pub(crate) fn callee(node: Node<'_>) -> Option<Node<'_>> {
    match node.kind() {
        "function_call_expression" => node.child_by_field_name("function"),
        "member_call_expression" | "nullsafe_member_call_expression" | "scoped_call_expression" => {
            node.child_by_field_name("name")
        }
        "object_creation_expression" => {
            let mut cursor = node.walk();
            node.named_children(&mut cursor)
                .find(|c| matches!(c.kind(), "name" | "qualified_name"))
        }
        _ => None,
    }
}

/// The last name segment of a PHP callee: `query` in `$db->query(…)`,
/// `Response` in `new \Slim\Psr7\Response()`.
pub(crate) fn callee_name(node: Node<'_>, source: &str) -> Option<String> {
    let callee = callee(node)?;
    let name = match callee.kind() {
        "name" => callee,
        "qualified_name" => {
            let mut cursor = callee.walk();
            callee
                .named_children(&mut cursor)
                .filter(|c| c.kind() == "name")
                .last()?
        }
        _ => return None,
    };
    Some(text(name, source).to_string())
}

/// The argument list of a PHP call or `new` expression.
pub(crate) fn arguments(node: Node<'_>) -> Option<Node<'_>> {
    node.child_by_field_name("arguments").or_else(|| {
        let mut cursor = node.walk();
        node.named_children(&mut cursor)
            .find(|c| c.kind() == "arguments")
    })
}

/// The value an `argument` node passes, past a named argument's label.
pub(crate) fn argument_value(argument: Node<'_>) -> Option<Node<'_>> {
    if argument.kind() != "argument" {
        return Some(argument);
    }
    let mut cursor = argument.walk();
    argument.named_children(&mut cursor).last()
}

/// The names a `use` declaration brings into scope: each clause's alias or
/// the last segment of its name.
pub(crate) fn imports(node: Node<'_>, source: &str, names: &mut BTreeSet<String>) {
    if node.kind() == "namespace_use_clause" {
        let name = node
            .child_by_field_name("alias")
            .or_else(|| {
                let mut cursor = node.walk();
                let last = node.named_children(&mut cursor).last()?;
                match last.kind() {
                    "qualified_name" => {
                        let mut inner = last.walk();
                        last.named_children(&mut inner)
                            .filter(|c| c.kind() == "name")
                            .last()
                    }
                    _ => Some(last),
                }
            })
            .map(|n| text(n, source).to_string());
        names.extend(name);
        return;
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        imports(child, source, names);
    }
}

fn closure(node: Node<'_>) -> bool {
    matches!(node.kind(), "anonymous_function" | "arrow_function")
}

/// Whether an expression statement's expression is PHP syntax that can
/// register or bind a closure: a call, or an assignment to a `$variable`.
pub(crate) fn registers(expression: Node<'_>) -> bool {
    CALLS.contains(&expression.kind())
        || expression.kind() == "assignment_expression"
            && expression
                .child_by_field_name("left")
                .is_some_and(|l| l.kind() == "variable_name")
}

/// Closures a top-level statement registers through a call, named by the
/// registration: `$app->get('/users/{id}')` or `Route::post('/pages')`.
/// Each call of a chain (`$app->get(…)->add(…)`) registers its last closure
/// argument. A closure assigned to a variable (`$handler = function …`) is
/// named by the variable.
pub(crate) fn registered_callbacks<'t>(
    statement: Node<'t>,
    source: &str,
) -> Vec<(String, Node<'t>)> {
    let Some(expression) = statement.named_child(0) else {
        return Vec::new();
    };
    if expression.kind() == "assignment_expression" {
        return expression
            .child_by_field_name("right")
            .filter(|r| closure(*r))
            .zip(expression.child_by_field_name("left"))
            .map(|(right, left)| vec![(text(left, source).to_string(), right)])
            .unwrap_or_default();
    }
    let mut found = Vec::new();
    let mut call = Some(expression).filter(|e| CALLS.contains(&e.kind()));
    while let Some(current) = call {
        let values: Vec<Node<'t>> = arguments(current)
            .map(|a| {
                let mut cursor = a.walk();
                a.named_children(&mut cursor)
                    .filter_map(argument_value)
                    .collect()
            })
            .unwrap_or_default();
        if let Some(handler) = values.iter().rev().find(|n| closure(**n)) {
            let path = values
                .first()
                .filter(|a| matches!(a.kind(), "string" | "encapsed_string"))
                .map_or("…", |a| text(*a, source));
            let receiver = current
                .child_by_field_name("object")
                .or_else(|| current.child_by_field_name("scope"))
                .map(|o| root(o, source));
            let method = callee_name(current, source).unwrap_or_default();
            let name = match (receiver, current.kind()) {
                (Some(receiver), "scoped_call_expression") => {
                    format!("{receiver}::{method}({path})")
                }
                (Some(receiver), _) => format!("{receiver}->{method}({path})"),
                (None, _) => format!("{method}({path})"),
            };
            found.push((name, *handler));
        }
        call = current
            .child_by_field_name("object")
            .filter(|o| CALLS.contains(&o.kind()));
    }
    found.reverse();
    found
}

/// The leftmost receiver of a chain such as `$app->group(…)->add(…)`.
fn root(node: Node<'_>, source: &str) -> String {
    let mut node = node;
    while let Some(inner) = node
        .child_by_field_name("object")
        .or_else(|| node.child_by_field_name("scope"))
    {
        node = inner;
    }
    text(node, source).to_string()
}

/// The closure a top-level `return function (…) {…};` hands to the file
/// that includes it, as configuration files of Slim and Laravel do.
pub(crate) fn returned_closure(statement: Node<'_>) -> Option<Node<'_>> {
    (statement.kind() == "return_statement"
        && statement.parent().is_some_and(|p| p.kind() == "program"))
    .then(|| statement.named_child(0))
    .flatten()
    .filter(|n| closure(*n))
}

/// The name of the unit a returned closure becomes.
pub(crate) const RETURNED_CLOSURE: &str = "returned closure";

/// Top-level statements of a PHP file that run when it is requested or
/// included, inside namespace blocks too: everything but definitions,
/// imports, inline HTML and statements holding a unit.
pub(crate) fn script_statements<'t>(root: Node<'t>, found: &mut Vec<Node<'t>>) {
    let mut cursor = root.walk();
    for node in root.named_children(&mut cursor) {
        if super::is_comment(node) || DEFINITIONS.contains(&node.kind()) {
            continue;
        }
        if node.kind() == "namespace_definition" {
            if let Some(body) = node.child_by_field_name("body") {
                script_statements(body, found);
            }
            continue;
        }
        found.push(node);
    }
}

/// `<?= $value ?>`: an expression statement that a short echo tag opens.
pub(crate) fn short_echo(node: Node<'_>, source: &str) -> bool {
    node.kind() == "expression_statement"
        && node.prev_sibling().is_some_and(|previous| {
            let tag = if previous.kind() == "text_interpolation" {
                previous.named_children(&mut previous.walk()).last()
            } else {
                Some(previous)
            };
            tag.is_some_and(|t| t.kind() == "php_tag" && text(t, source) == "<?=")
        })
}

/// A double-quoted string or heredoc that interpolates a value.
pub(crate) fn interpolates(node: Node<'_>) -> bool {
    let parts = match node.kind() {
        "encapsed_string" => Some(node),
        "heredoc" => node.child_by_field_name("value"),
        _ => None,
    };
    parts.is_some_and(|parts| {
        let mut cursor = parts.walk();
        parts.named_children(&mut cursor).any(|c| {
            !matches!(
                c.kind(),
                "string_content" | "escape_sequence" | "heredoc_start" | "heredoc_end"
            )
        })
    })
}

/// A PHPUnit test class: `class …Test extends TestCase` or any `…TestCase` base.
pub(crate) fn test_class(node: Node<'_>, source: &str) -> bool {
    node.kind() == "class_declaration"
        && node
            .named_children(&mut node.walk())
            .any(|c| c.kind() == "base_clause" && text(c, source).trim_end().ends_with("TestCase"))
}

/// A test method of a PHPUnit class: named `test…`, or marked with a
/// `@test` doc comment or a `#[Test]` attribute.
pub(crate) fn test_method(node: Node<'_>, source: &str) -> bool {
    if node.kind() != "method_declaration" {
        return false;
    }
    let named = node
        .child_by_field_name("name")
        .is_some_and(|n| text(n, source).starts_with("test"));
    let attributed = node
        .named_children(&mut node.walk())
        .any(|c| c.kind() == "attribute_list" && text(c, source).contains("Test"));
    let documented = node
        .prev_named_sibling()
        .is_some_and(|c| super::is_comment(c) && text(c, source).contains("@test"));
    named || attributed || documented
}

/// A Pest statement: `test('…', fn)` or `it('…', fn)`, a case, or
/// `describe('…', fn)`, a suite; also with chained calls such as `->skip()`.
pub(crate) struct Pest {
    pub title: String,
    pub suite: bool,
}

pub(crate) fn pest_statement(node: Node<'_>, source: &str) -> Option<Pest> {
    if node.kind() != "expression_statement" {
        return None;
    }
    let mut call = node.named_child(0)?;
    while call.kind() == "member_call_expression" {
        call = call.child_by_field_name("object")?;
    }
    let function = call
        .child_by_field_name("function")
        .filter(|_| call.kind() == "function_call_expression")?;
    let suite = match text(function, source) {
        "test" | "it" => false,
        "describe" => true,
        _ => return None,
    };
    let first = arguments(call)?.named_child(0).and_then(argument_value)?;
    matches!(first.kind(), "string" | "encapsed_string").then(|| Pest {
        title: text(first, source).trim_matches(['"', '\'']).to_string(),
        suite,
    })
}
