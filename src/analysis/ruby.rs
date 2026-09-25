//! Ruby syntax shared by the analyses: calls with blocks, `require` paths,
//! and the RSpec and Minitest words that declare tests rather than register
//! application callbacks. Syntax locates; it never judges.
use super::text;
use tree_sitter::Node;

/// Methods that declare example groups, examples, hooks and shared examples
/// in RSpec, Minitest and `test "…" do` suites.
const TEST_DSL: &[&str] = &[
    "describe",
    "context",
    "feature",
    "shared_examples",
    "shared_examples_for",
    "shared_context",
    "include_context",
    "include_examples",
    "it_behaves_like",
    "it_should_behave_like",
    "xdescribe",
    "fdescribe",
    "xcontext",
    "fcontext",
    "it",
    "its",
    "specify",
    "example",
    "scenario",
    "xit",
    "fit",
    "xspecify",
    "xexample",
    "xscenario",
    "pending",
    "skip",
    "test",
    "should",
    "let",
    "let!",
    "subject",
    "subject!",
    "before",
    "after",
    "around",
    "background",
    "given",
    "given!",
    "setup",
    "teardown",
];

/// Methods whose block groups examples, with a title: `describe Order do`.
pub(crate) const GROUPS: &[&str] = &[
    "describe",
    "context",
    "feature",
    "shared_examples",
    "shared_examples_for",
    "shared_context",
    "xdescribe",
    "fdescribe",
    "xcontext",
    "fcontext",
];

/// Methods whose block is one example: `it "adds" do`, `test "adds" do`.
pub(crate) const CASES: &[&str] = &[
    "it",
    "its",
    "specify",
    "example",
    "scenario",
    "xit",
    "fit",
    "xspecify",
    "xexample",
    "xscenario",
    "test",
    "should",
];

/// The method a Ruby call names: `get` in `get '/' do … end`.
pub(crate) fn method<'a>(call: Node<'_>, source: &'a str) -> &'a str {
    if call.kind() != "call" {
        return "";
    }
    call.child_by_field_name("method")
        .map_or("", |m| text(m, source))
}

/// The body of the block a Ruby call passes: `do … end` or `{ … }`.
pub(crate) fn block_body(call: Node<'_>) -> Option<Node<'_>> {
    if call.kind() != "call" {
        return None;
    }
    call.child_by_field_name("block")?
        .child_by_field_name("body")
}

/// A test declaration such as `describe`, `it`, `let` or `before`, or any
/// call on `RSpec`: `RSpec.describe`, and `RSpec.configure`, which sets up
/// the test framework rather than defining the program's logic.
pub(crate) fn test_dsl(call: Node<'_>, source: &str) -> bool {
    match call
        .child_by_field_name("receiver")
        .map(|r| text(r, source))
    {
        Some(receiver) => receiver == "RSpec",
        None => TEST_DSL.contains(&method(call, source)),
    }
}

/// The first argument of a call.
pub(crate) fn first_argument(call: Node<'_>) -> Option<Node<'_>> {
    call.child_by_field_name("arguments")?.named_child(0)
}

/// The path a `require`, `require_relative` or `load` call names.
pub(crate) fn required<'a>(call: Node<'_>, source: &'a str) -> Option<&'a str> {
    if !matches!(
        method(call, source),
        "require" | "require_relative" | "load" | "autoload"
    ) || call.child_by_field_name("receiver").is_some()
    {
        return None;
    }
    let arguments = call.child_by_field_name("arguments")?;
    let mut cursor = arguments.walk();
    let path = arguments
        .named_children(&mut cursor)
        .find(|a| a.kind() == "string")?;
    Some(text(path, source).trim_matches(['"', '\'']))
}

/// A title as written: a string's text, or a constant, symbol or other
/// argument as source.
pub(crate) fn title(argument: Node<'_>, source: &str) -> String {
    let written = text(argument, source);
    if argument.kind() == "string" {
        written.trim_matches(['"', '\'']).to_string()
    } else {
        written.to_string()
    }
}

/// Definitions a body holds directly: methods, classes and modules.
pub(crate) fn defines(body: Node<'_>) -> bool {
    let mut cursor = body.walk();
    body.named_children(&mut cursor).any(|child| {
        matches!(
            child.kind(),
            "method" | "singleton_method" | "class" | "module" | "singleton_class"
        )
    })
}

/// Local names a Ruby test case binds: assignment targets and block and
/// method parameters. A bare identifier that is not one of these is a call.
pub(crate) fn locals(node: Node<'_>, source: &str, names: &mut Vec<String>) {
    match node.kind() {
        "assignment" | "operator_assignment" => {
            if let Some(left) = node.child_by_field_name("left") {
                bound(left, source, names);
            }
        }
        "block_parameters" | "method_parameters" | "lambda_parameters" => {
            bound(node, source, names);
        }
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        locals(child, source, names);
    }
}

fn bound(node: Node<'_>, source: &str, names: &mut Vec<String>) {
    if node.kind() == "identifier" {
        names.push(text(node, source).to_string());
        return;
    }
    if node.kind() == "call" {
        return;
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        bound(child, source, names);
    }
}
