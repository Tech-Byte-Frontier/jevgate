//! Test cases, the non-test functions they call, and candidate redundant pairs.
//! Rust `#[test]`-family functions, JavaScript and TypeScript `it`/`test`
//! (including `.each`), Python `test_*` functions, Go `Test…` functions and
//! C# methods marked `[Fact]`, `[Theory]`, `[Test]` or `[TestMethod]`.
use super::{callee_name, fast_hash, is_comment, line_of, macro_calls, text};
use anyhow::Result;
use std::{collections::BTreeSet, ops::Range, path::Path};
use tree_sitter::Node;

/// Candidate redundant pairs per test file.
pub const PAIR_CAP: usize = 12;
const SIMILARITY: f64 = 0.5;

#[derive(Clone, Debug)]
pub struct TestCase {
    pub name: String,
    pub span: Range<usize>,
    pub line: usize,
    pub end_line: usize,
    pub calls: BTreeSet<String>,
    /// Called non-test functions in scope, by name.
    pub subjects: Vec<String>,
    /// Titles of the enclosing `describe` blocks, test classes or modules,
    /// outermost first.
    pub suite: Vec<String>,
    shingles: BTreeSet<u64>,
}

impl TestCase {
    pub fn source<'a>(&self, source: &'a str) -> &'a str {
        &source[self.span.clone()]
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TestPair {
    pub a: usize,
    pub b: usize,
    pub subject: String,
    pub similarity: f64,
}

pub fn cases(path: &Path, source: &str) -> Result<Vec<TestCase>> {
    let Some(tree) = crate::syntax::parse(path, source)? else {
        return Ok(Vec::new());
    };
    let mut found = Vec::new();
    let pytest = crate::test_locations::pytest_file(path);
    visit(tree.root_node(), source, pytest, false, &mut found);
    let mut suites = Vec::new();
    collect_suites(tree.root_node(), source, &mut suites);
    for case in &mut found {
        case.suite = suites
            .iter()
            .filter(|(span, _)| span.start <= case.span.start && case.span.end <= span.end)
            .map(|(_, title)| title.clone())
            .collect();
    }
    Ok(found)
}

/// Blocks that group test cases, with their titles, in source order: a
/// `describe`/`context`/`suite` call with a literal title, a Python class,
/// or a Rust module.
fn collect_suites(node: Node<'_>, source: &str, suites: &mut Vec<(Range<usize>, String)>) {
    let title = match node.kind() {
        "call_expression" => suite_call(node, source),
        "class_definition" | "mod_item" => Some(name(node, source)).filter(|name| !name.is_empty()),
        // A C# test class; TypeScript classes (`class_body`) do not group tests.
        "class_declaration"
            if node
                .child_by_field_name("body")
                .is_some_and(|b| b.kind() == "declaration_list") =>
        {
            Some(name(node, source)).filter(|name| !name.is_empty())
        }
        _ => None,
    };
    if let Some(title) = title {
        suites.push((node.start_byte()..node.end_byte(), title));
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_suites(child, source, suites);
    }
}

/// The title of `describe("title", fn)`, `describe.each(table)("title", fn)`
/// and their `context` and `suite` spellings.
fn suite_call(node: Node<'_>, source: &str) -> Option<String> {
    let callee = node.child_by_field_name("function")?;
    let base = if callee.kind() == "call_expression" {
        text(callee.child_by_field_name("function")?, source).trim_end_matches(".each")
    } else {
        text(callee, source)
    };
    let head = base.split('.').next().unwrap_or("");
    if !matches!(
        head,
        "describe" | "context" | "suite" | "xdescribe" | "fdescribe"
    ) {
        return None;
    }
    let first = node.child_by_field_name("arguments")?.named_child(0)?;
    matches!(first.kind(), "string" | "template_string").then(|| {
        text(first, source)
            .trim_matches(['"', '\'', '`'])
            .to_string()
    })
}

fn visit(
    node: Node<'_>,
    source: &str,
    pytest: bool,
    in_test_class: bool,
    found: &mut Vec<TestCase>,
) {
    match node.kind() {
        "function_item" => {
            let marked = crate::test_locations::preceding_attributes(node, source)
                .iter()
                .any(|attribute| crate::test_locations::attribute_marks_test(attribute));
            if marked {
                push(
                    node,
                    attribute_start(node),
                    name(node, source),
                    source,
                    found,
                );
            }
            return;
        }
        "function_declaration" if crate::test_locations::go_test_function(node, source) => {
            push(node, node.start_byte(), name(node, source), source, found);
            return;
        }
        "method_declaration" if crate::test_locations::csharp_test_method(node, source) => {
            push(node, node.start_byte(), name(node, source), source, found);
            return;
        }
        "function_definition" => {
            let test = name(node, source).starts_with("test");
            let outer = node
                .parent()
                .filter(|p| p.kind() == "decorated_definition")
                .unwrap_or(node);
            let top_level = outer.parent().is_some_and(|p| p.kind() == "module");
            if test && (top_level || in_test_class) {
                push(outer, outer.start_byte(), name(node, source), source, found);
            }
            return;
        }
        "class_definition" => {
            let test_class = crate::test_locations::python_test_class(node, source, pytest);
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                visit(child, source, pytest, test_class, found);
            }
            return;
        }
        "call_expression" => {
            if let Some(case) = javascript_case(node, source) {
                push(
                    statement(node),
                    statement(node).start_byte(),
                    case,
                    source,
                    found,
                );
                return;
            }
        }
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        visit(child, source, pytest, in_test_class, found);
    }
}

/// `it("name", fn)`, `test.only("name", fn)` or `it.each(table)("name", fn)`.
fn javascript_case(node: Node<'_>, source: &str) -> Option<String> {
    let callee = node.child_by_field_name("function")?;
    let base = if callee.kind() == "call_expression" {
        let inner = callee.child_by_field_name("function")?;
        let inner_text = text(inner, source);
        (inner_text.ends_with(".each")).then_some(inner_text.trim_end_matches(".each"))?
    } else {
        text(callee, source)
    };
    let head = base.split('.').next().unwrap_or("");
    if !matches!(head, "it" | "test" | "xit" | "fit" | "xtest") {
        return None;
    }
    let arguments = node.child_by_field_name("arguments")?;
    let first = arguments.named_child(0)?;
    matches!(first.kind(), "string" | "template_string").then(|| {
        text(first, source)
            .trim_matches(['"', '\'', '`'])
            .to_string()
    })
}

fn statement(node: Node<'_>) -> Node<'_> {
    node.parent()
        .filter(|p| p.kind() == "expression_statement")
        .unwrap_or(node)
}

fn attribute_start(node: Node<'_>) -> usize {
    let mut start = node.start_byte();
    let mut previous = node.prev_named_sibling();
    while let Some(sibling) = previous {
        if sibling.kind() != "attribute_item" && !is_comment(sibling) {
            break;
        }
        start = sibling.start_byte();
        previous = sibling.prev_named_sibling();
    }
    start
}

fn name(node: Node<'_>, source: &str) -> String {
    node.child_by_field_name("name")
        .map(|n| text(n, source).to_string())
        .unwrap_or_default()
}

fn push(node: Node<'_>, start: usize, name: String, source: &str, found: &mut Vec<TestCase>) {
    let mut calls = BTreeSet::new();
    let mut tokens = Vec::new();
    walk(node, source, &mut calls, &mut tokens);
    let shingles = tokens.windows(3).map(fast_hash).collect();
    found.push(TestCase {
        name,
        span: start..node.end_byte(),
        line: line_of(source, node.start_byte()),
        end_line: line_of(source, node.end_byte().saturating_sub(1)),
        calls,
        subjects: Vec::new(),
        suite: Vec::new(),
        shingles,
    });
}

fn walk<'a>(
    node: Node<'_>,
    source: &'a str,
    calls: &mut BTreeSet<String>,
    tokens: &mut Vec<&'a str>,
) {
    if is_comment(node) {
        return;
    }
    match node.kind() {
        "call_expression" | "call" | "invocation_expression" => {
            if let Some(name) = node
                .child_by_field_name("function")
                .and_then(|f| callee_name(f, source))
            {
                calls.insert(name);
            }
        }
        "jsx_opening_element" | "jsx_self_closing_element" => {
            if let Some(name) = node.child_by_field_name("name") {
                calls.insert(text(name, source).to_string());
            }
        }
        // C#: constructing the class under test calls its constructor.
        "object_creation_expression" => {
            if let Some(name) = node
                .child_by_field_name("type")
                .and_then(|t| callee_name(t, source))
            {
                calls.insert(name);
            }
        }
        "token_tree"
            if node
                .parent()
                .is_some_and(|p| p.kind() == "macro_invocation") =>
        {
            macro_calls(node, source, calls);
        }
        _ => {}
    }
    if node.child_count() == 0 {
        // Identifiers are normalized so renamed locals still compare as similar.
        tokens.push(if node.kind().ends_with("identifier") {
            "\u{1}id"
        } else {
            text(node, source)
        });
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk(child, source, calls, tokens);
    }
}

/// Keep calls that name a non-test function in scope.
pub fn link(cases: &mut [TestCase], scope: &BTreeSet<String>) {
    for case in cases {
        case.subjects = case
            .calls
            .iter()
            .filter(|call| scope.contains(*call))
            .cloned()
            .collect();
    }
}

/// Pairs that share a subject and look alike, most similar first, capped.
pub fn pairs(cases: &[TestCase]) -> (Vec<TestPair>, usize) {
    let mut found = Vec::new();
    for a in 0..cases.len() {
        for b in a + 1..cases.len() {
            let Some(subject) = cases[a]
                .subjects
                .iter()
                .find(|s| cases[b].subjects.contains(s))
            else {
                continue;
            };
            let (x, y) = (&cases[a].shingles, &cases[b].shingles);
            let union = x.union(y).count();
            if union == 0 {
                continue;
            }
            let similarity = x.intersection(y).count() as f64 / union as f64;
            if similarity >= SIMILARITY {
                found.push(TestPair {
                    a,
                    b,
                    subject: subject.clone(),
                    similarity,
                });
            }
        }
    }
    found.sort_by(|p, q| {
        q.similarity
            .total_cmp(&p.similarity)
            .then((p.a, p.b).cmp(&(q.a, q.b)))
    });
    let omitted = found.len().saturating_sub(PAIR_CAP);
    found.truncate(PAIR_CAP);
    (found, omitted)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(path: &str, source: &str) -> Vec<String> {
        cases(Path::new(path), source)
            .unwrap()
            .into_iter()
            .map(|c| c.name)
            .collect()
    }

    #[test]
    fn test_cases_are_found_in_rust_javascript_and_python() {
        let rust = "fn total(v: &[i32]) -> i32 { v.iter().sum() }\n#[cfg(test)]\nmod tests {\n    use super::*;\n    fn helper() -> Vec<i32> { vec![1] }\n    #[test]\n    fn sums() { assert_eq!(total(&helper()), 1); }\n    #[tokio::test]\n    async fn sums_async() { assert_eq!(total(&[]), 0); }\n}\n";
        assert_eq!(names("lib.rs", rust), ["sums", "sums_async"]);
        let script = "import { total } from './total';\ndescribe('total', () => {\n  it('adds', () => { expect(total([1, 2])).toBe(3); });\n  test.each([[1, 1]])('keeps %i', (a, b) => { expect(total([a])).toBe(b); });\n  beforeEach(() => {});\n});\ntest('empty', () => expect(total([])).toBe(0));\n";
        assert_eq!(
            names("total.test.ts", script),
            ["adds", "keeps %i", "empty"]
        );
        let python = "from app import total\n\ndef test_adds():\n    assert total([1, 2]) == 3\n\ndef helper():\n    return [1]\n\nclass TestTotal:\n    def test_empty(self):\n        assert total([]) == 0\n\nclass Other:\n    def test_like(self):\n        pass\n";
        assert_eq!(names("test_total.py", python), ["test_adds", "test_empty"]);
        let suites = |path: &str, source: &str| -> Vec<Vec<String>> {
            cases(Path::new(path), source)
                .unwrap()
                .into_iter()
                .map(|c| c.suite)
                .collect()
        };
        assert_eq!(
            suites("total.test.ts", script),
            [vec!["total"], vec!["total"], vec![]]
        );
        assert_eq!(suites("test_total.py", python), [vec![], vec!["TestTotal"]]);
        assert_eq!(suites("lib.rs", rust), [vec!["tests"], vec!["tests"]]);
        let go = "package total\n\nimport \"testing\"\n\nfunc TestAdds(t *testing.T) {\n\tif Total([]int{1, 2}) != 3 {\n\t\tt.Fatal(\"sum\")\n\t}\n}\n\nfunc BenchmarkTotal(b *testing.B) {\n\tfor i := 0; i < b.N; i++ {\n\t\tTotal(nil)\n\t}\n}\n\nfunc TestHelper() int { return 1 }\n";
        assert_eq!(names("total_test.go", go), ["TestAdds", "BenchmarkTotal"]);
        let located = crate::test_locations::locate_tests(Path::new("total_test.go"), go).unwrap();
        assert_eq!(located.ranges.len(), 2);
        let csharp = "using Xunit;\n\nnamespace Shop.Tests;\n\npublic class OrderTotal\n{\n    [Fact]\n    public void IsZeroForNewOrder()\n    {\n        Assert.Equal(0, new Order().Total());\n    }\n\n    [Theory]\n    [InlineData(1)]\n    public void Adds(int count) => Assert.Equal(count, Build(count).Total());\n\n    [Xunit.FactAttribute]\n    public void Qualified() { }\n\n    private static Order Build(int count) => new Order(count);\n}\n\n[TestClass]\npublic class Checks\n{\n    [TestMethod]\n    public void Runs() { }\n\n    [NUnit.Framework.TestCase(2)]\n    public void Case(int n) { }\n}\n";
        assert_eq!(
            names("OrderTotal.cs", csharp),
            ["IsZeroForNewOrder", "Adds", "Qualified", "Runs", "Case"]
        );
        let found = cases(Path::new("OrderTotal.cs"), csharp).unwrap();
        assert_eq!(found[0].suite, ["OrderTotal"]);
        assert!(found[0].calls.contains("Order") && found[0].calls.contains("Total"));
        assert!(found[1].calls.contains("Build"));
        let located =
            crate::test_locations::locate_tests(Path::new("OrderTotal.cs"), csharp).unwrap();
        let lines: Vec<(usize, usize)> = located
            .ranges
            .iter()
            .map(|r| (r.start_line, r.end_line))
            .collect();
        assert_eq!(lines, [(5, 21), (23, 31)], "whole test classes");
    }

    #[test]
    fn subjects_are_scope_functions_and_pairs_need_a_shared_subject() {
        let script = "it('adds two', () => { const value = total([1, 2]); expect(value).toBe(3); });\nit('adds three', () => { const result = total([1, 2, 3]); expect(result).toBe(6); });\nit('formats', () => { expect(label('a')).toBe('A'); });\n";
        let mut found = cases(Path::new("total.test.js"), script).unwrap();
        link(
            &mut found,
            &BTreeSet::from(["total".into(), "label".into()]),
        );
        assert_eq!(found[0].subjects, ["total"]);
        assert!(!found[0].subjects.contains(&"expect".to_string()));
        let (pairs, omitted) = pairs(&found);
        assert_eq!(omitted, 0);
        assert_eq!(pairs.len(), 1);
        assert_eq!((pairs[0].a, pairs[0].b), (0, 1));
        assert_eq!(pairs[0].subject, "total");
    }
}
