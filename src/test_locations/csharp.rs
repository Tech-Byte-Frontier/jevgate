//! C# tests: methods marked by an xUnit, NUnit or MSTest attribute and the
//! classes that hold them.
use super::child_text;
use tree_sitter::Node;

/// Attributes that make a C# method a test in xUnit (`[Fact]`, `[Theory]`),
/// NUnit (`[Test]`, `[TestCase]`, `[TestCaseSource]`) or MSTest
/// (`[TestMethod]`, `[DataTestMethod]`), and classes a runner collects
/// (`[TestFixture]`, `[TestClass]`).
const CSHARP_TEST_ATTRIBUTES: [&str; 7] = [
    "Fact",
    "Theory",
    "Test",
    "TestCase",
    "TestCaseSource",
    "TestMethod",
    "DataTestMethod",
];
const CSHARP_TEST_CLASS_ATTRIBUTES: [&str; 2] = ["TestFixture", "TestClass"];

/// The names of the attributes on a C# declaration, without a namespace or
/// the `Attribute` suffix: `[Xunit.FactAttribute]` is `Fact`.
fn csharp_attributes<'a>(node: Node<'_>, source: &'a str) -> Vec<&'a str> {
    let mut names = Vec::new();
    let mut cursor = node.walk();
    for list in node
        .named_children(&mut cursor)
        .filter(|c| c.kind() == "attribute_list")
    {
        let mut inner = list.walk();
        for attribute in list
            .named_children(&mut inner)
            .filter(|c| c.kind() == "attribute")
        {
            if let Some(name) = attribute.child_by_field_name("name") {
                let name = child_text(name, source);
                let name = name.rsplit('.').next().unwrap_or(name);
                let name = name.split('<').next().unwrap_or(name);
                names.push(name.strip_suffix("Attribute").unwrap_or(name));
            }
        }
    }
    names
}

/// A C# test method: one with a test attribute of a supported framework.
pub(crate) fn csharp_test_method(node: Node<'_>, source: &str) -> bool {
    node.kind() == "method_declaration"
        && csharp_attributes(node, source)
            .iter()
            .any(|name| CSHARP_TEST_ATTRIBUTES.contains(name))
}

/// A C# test class: marked as a fixture, or holding test methods. Its setup,
/// fields and helpers are test code as well.
pub(crate) fn csharp_test_class(node: Node<'_>, source: &str) -> bool {
    if !matches!(node.kind(), "class_declaration" | "record_declaration") {
        return false;
    }
    if csharp_attributes(node, source)
        .iter()
        .any(|name| CSHARP_TEST_CLASS_ATTRIBUTES.contains(name))
    {
        return true;
    }
    let Some(body) = node.child_by_field_name("body") else {
        return false;
    };
    let mut cursor = body.walk();
    body.named_children(&mut cursor)
        .any(|member| csharp_test_method(member, source))
}

/// A C# test class, whole; nested test classes are inside its span.
pub(super) fn csharp_test_span(node: Node<'_>, source: &str) -> Option<(usize, usize)> {
    csharp_test_class(node, source).then(|| (node.start_byte(), node.end_byte()))
}
