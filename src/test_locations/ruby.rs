//! Ruby tests: RSpec groups and examples, Rails `test` blocks, and
//! Minitest or Test::Unit test classes.
use super::child_text;
use crate::analysis::ruby;
use tree_sitter::Node;

/// A Ruby example group or example written as a statement with a block:
/// RSpec's `describe Order do`, `RSpec.describe`, `context`, `it "adds" do`
/// and `it { is_expected.to … }`, and a Rails `test "adds" do`. An example
/// is titled by a string or not at all, so `test(:unit) do` in a Rakefile
/// is not one.
pub(crate) fn ruby_test_call(node: Node<'_>, source: &str) -> bool {
    if node.kind() != "call" || node.child_by_field_name("block").is_none() {
        return false;
    }
    let method = ruby::method(node, source);
    let receiver = node
        .child_by_field_name("receiver")
        .map(|r| child_text(r, source));
    let statement = node
        .parent()
        .is_some_and(|p| matches!(p.kind(), "program" | "body_statement" | "block_body"));
    let titled = || match ruby::first_argument(node) {
        None => true,
        Some(title) => {
            title.kind() == "string" || (method == "its" && title.kind() == "simple_symbol")
        }
    };
    statement
        && receiver.is_none_or(|r| r == "RSpec")
        && (ruby::GROUPS.contains(&method) || ruby::CASES.contains(&method) && titled())
}

/// A Minitest, Test::Unit or Rails test class: its superclass ends in `Test`,
/// `TestCase` or `Spec`, as `Minitest::Test`, `ActiveSupport::TestCase` and
/// `ActionDispatch::IntegrationTest` do.
pub(crate) fn ruby_test_class(node: Node<'_>, source: &str) -> bool {
    node.kind() == "class"
        && node
            .child_by_field_name("superclass")
            .and_then(|s| s.named_child(0))
            .is_some_and(|base| {
                let name = child_text(base, source).rsplit("::").next().unwrap_or("");
                name.ends_with("Test") || name.ends_with("TestCase") || name == "Spec"
            })
}
