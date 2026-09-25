//! Test cases, the non-test functions they call, and candidate redundant pairs.
//! Rust `#[test]`-family functions, JavaScript and TypeScript `it`/`test`
//! (including `.each`), Python `test_*` functions, Go `Test…` functions, C#
//! methods marked `[Fact]`, `[Theory]`, `[Test]` or `[TestMethod]`, Ruby
//! RSpec examples (`it`, `specify`), Rails `test "…" do` blocks and Minitest
//! `test_*` methods, and Java `@Test`-family methods (or JUnit 3 `test…`
//! methods of a `TestCase`).
use super::{call_name, callee_name, fast_hash, is_comment, line_of, macro_calls, ruby, text};
use anyhow::Result;
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range,
    path::Path,
};
use tree_sitter::Node;

/// Candidate redundant pairs per test file.
pub const PAIR_CAP: usize = 12;
const SIMILARITY: f64 = 0.5;
/// Tokens in a shingle: a pair's similarity is the share of these runs of
/// tokens the two tests have in common.
const SHINGLE_TOKENS: usize = 3;

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
    /// Setup its enclosing groups declare for it, in source order: RSpec
    /// `before` and `around`, the `let` and `subject` it reads, and Minitest
    /// `setup`. Only Ruby cases have these; other languages' setup is found in
    /// the file's text.
    pub hooks: Vec<Range<usize>>,
    /// Names its hooks call, for the test helpers they use. Only Ruby cases
    /// have these.
    pub hook_calls: BTreeSet<String>,
    /// Requests the test sends to a web route, such as MockMvc's `get("/owners")`.
    pub requests: Vec<super::routes::Route>,
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
    qualify_repeated_names(&mut found);
    Ok(found)
}

/// Cases titled alike in different suites, as RSpec examples often are
/// (`it "can be invoked with a string"` under two contexts), are named with
/// as many of their innermost suite titles as tell them apart:
/// `when trait is a symbol > can be invoked with a string`.
fn qualify_repeated_names(cases: &mut [TestCase]) {
    let names: Vec<String> = cases.iter().map(|c| c.name.clone()).collect();
    for name in names.iter().collect::<BTreeSet<_>>() {
        let alike: Vec<usize> = (0..cases.len()).filter(|&i| &names[i] == name).collect();
        if alike.len() < 2 {
            continue;
        }
        let deepest = alike
            .iter()
            .map(|&i| cases[i].suite.len())
            .max()
            .unwrap_or(0);
        let qualified = |depth: usize| -> Vec<String> {
            alike
                .iter()
                .map(|&i| {
                    let suite = &cases[i].suite;
                    let groups = &suite[suite.len().saturating_sub(depth)..];
                    groups
                        .iter()
                        .chain(std::iter::once(name))
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(" > ")
                })
                .collect()
        };
        // The shallowest qualification that tells the most of them apart;
        // cases alike within one suite keep their shared name.
        let distinct = |named: &Vec<String>| named.iter().collect::<BTreeSet<_>>().len();
        let Some(chosen) = (1..=deepest)
            .map(qualified)
            .rev()
            .max_by_key(distinct)
            .filter(|named| distinct(named) > 1)
        else {
            continue;
        };
        for (&i, named) in alike.iter().zip(chosen) {
            cases[i].name = named;
        }
    }
}

/// Blocks that group test cases, with their titles, in source order: a
/// `describe`/`context`/`suite` call with a literal title, a Python or Java
/// class, or a Rust module.
fn collect_suites(node: Node<'_>, source: &str, suites: &mut Vec<(Range<usize>, String)>) {
    let title = match node.kind() {
        "call_expression" => suite_call(node, source),
        "call" if crate::test_locations::ruby_test_call(node, source) => {
            Some(ruby::method(node, source))
                .filter(|method| ruby::GROUPS.contains(method))
                .and_then(|_| ruby::first_argument(node))
                .map(|title| ruby::title(title, source))
        }
        "class_definition" | "mod_item" | "class" | "module" => {
            Some(name(node, source)).filter(|name| !name.is_empty())
        }
        "class_declaration" if crate::analysis::php::test_class(node, source) => {
            Some(name(node, source))
        }
        "expression_statement" => crate::analysis::php::pest_statement(node, source)
            .filter(|pest| pest.suite)
            .map(|pest| pest.title),
        // A C# test class, or a nested Java class (JUnit 5 `@Nested`): the
        // top-level Java class is the file itself, which every case would
        // share. TypeScript classes (`class_body`) do not group tests.
        "class_declaration"
            if node
                .child_by_field_name("body")
                .is_some_and(|b| b.kind() == "declaration_list")
                || node.parent().is_some_and(|p| p.kind() == "class_body") =>
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
        // PHP: a test method of a PHPUnit test class.
        "method_declaration"
            if in_test_class && crate::analysis::php::test_method(node, source) =>
        {
            push(node, node.start_byte(), name(node, source), source, found);
            return;
        }
        "method_declaration" => {
            if crate::test_locations::csharp_test_method(node, source)
                || crate::test_locations::java_test_method(node, source)
                || in_test_class && name(node, source).starts_with("test")
            {
                push(node, node.start_byte(), name(node, source), source, found);
            }
            return;
        }
        // A JUnit 3 `TestCase` subclass: its `test…` methods are tests.
        "class_declaration" if crate::test_locations::junit3_class(node, source) => {
            visit_children(node, source, pytest, true, found);
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
            visit_children(node, source, pytest, test_class, found);
            return;
        }
        // Ruby: `class OrderTest < Minitest::Test` holds `test_*` methods.
        "class" if crate::test_locations::ruby_test_class(node, source) => {
            visit_children(node, source, pytest, true, found);
            return;
        }
        "method" => {
            let name = name(node, source);
            if in_test_class && name.starts_with("test_") {
                push(node, node.start_byte(), name, source, found);
                ruby_context(node, source, found);
            }
            return;
        }
        "call"
            if crate::test_locations::ruby_test_call(node, source)
                && ruby::CASES.contains(&ruby::method(node, source)) =>
        {
            push(
                node,
                node.start_byte(),
                ruby_case_name(node, source),
                source,
                found,
            );
            ruby_context(node, source, found);
            return;
        }
        "class_declaration" if crate::analysis::php::test_class(node, source) => {
            visit_children(node, source, pytest, true, found);
            return;
        }
        "expression_statement" => {
            if let Some(case) = crate::analysis::php::pest_statement(node, source)
                && !case.suite
            {
                push(node, node.start_byte(), case.title, source, found);
                return;
            }
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
    visit_children(node, source, pytest, in_test_class, found);
}

fn visit_children(
    node: Node<'_>,
    source: &str,
    pytest: bool,
    in_test_class: bool,
    found: &mut Vec<TestCase>,
) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        visit(child, source, pytest, in_test_class, found);
    }
}

/// A Ruby example's title: its string, `its(:name)`, or the first line of a
/// one-line example such as `it { is_expected.to be_valid }`.
fn ruby_case_name(node: Node<'_>, source: &str) -> String {
    let method = ruby::method(node, source);
    match ruby::first_argument(node) {
        Some(title) if title.kind() == "string" => ruby::title(title, source),
        Some(title) => format!("{method} {}", text(title, source)),
        None => {
            let body = ruby::block_body(node).map_or("", |b| text(b, source));
            let line = body.lines().next().unwrap_or("").trim();
            if line.is_empty() {
                method.to_string()
            } else {
                format!("{method} {{ {} }}", clip_name(line))
            }
        }
    }
}

fn clip_name(line: &str) -> String {
    const NAME_CHARS: usize = 80;
    if line.chars().count() <= NAME_CHARS {
        return line.to_string();
    }
    format!("{}…", line.chars().take(NAME_CHARS).collect::<String>())
}

/// The setup and calls of the last case, a Ruby example or test method.
/// Its enclosing groups' `before` and `around` blocks, `let!`, and a
/// Minitest `setup` run before it and are its hooks; a `let` or `subject` is
/// lazy, so it is a hook only when the case or another hook reads it. Ruby
/// calls a method without parentheses or arguments as a bare name (`total`),
/// so the case calls each bare name that is not a local it binds or a `let`
/// of its groups, and the calls of each `let` and `subject` it reads.
fn ruby_context(node: Node<'_>, source: &str, found: &mut [TestCase]) {
    let Some(case) = found.last_mut() else {
        return;
    };
    let mut bound = Vec::new();
    ruby::locals(node, source, &mut bound);
    let (eager, lazy) = group_setup(node, source);
    bound.extend(lazy.iter().map(|(name, _)| name.clone()));
    let mut read = Vec::new();
    identifiers(node, source, &mut read);
    case.calls
        .extend(read.iter().filter(|name| !bound.contains(name)).cloned());
    for hook in &eager {
        identifiers(*hook, source, &mut read);
    }
    let used = lazy_read(&lazy, &mut read, source, &bound, &mut case.calls);
    let mut hooks: Vec<Node<'_>> = eager;
    hooks.extend(
        used.iter()
            .filter(|u| !hooks.contains(u))
            .copied()
            .collect::<Vec<_>>(),
    );
    hooks.sort_by_key(|hook| hook.start_byte());
    hooks.dedup();
    for hook in &hooks {
        read_calls(*hook, source, &bound, &mut case.hook_calls);
    }
    case.hooks = hooks.iter().map(|hook| hook.byte_range()).collect();
}

/// The setup a Ruby case's enclosing groups declare, innermost first: the
/// hooks that run before it (`before`, `around`, `let!`, `subject!` and a
/// Minitest `setup`), and each `let` and `subject` by name.
type GroupSetup<'tree> = (Vec<Node<'tree>>, Vec<(String, Node<'tree>)>);

fn group_setup<'tree>(node: Node<'tree>, source: &str) -> GroupSetup<'tree> {
    let mut eager = Vec::new();
    // Innermost first, so an inner `let` hides an outer one of the same name.
    let mut lazy: Vec<(String, Node<'tree>)> = Vec::new();
    let mut ancestor = node.parent();
    while let Some(scope) = ancestor {
        if matches!(scope.kind(), "body_statement" | "block_body" | "program") {
            let mut cursor = scope.walk();
            for sibling in scope.named_children(&mut cursor) {
                let method = ruby::method(sibling, source);
                let with_block = sibling.child_by_field_name("block").is_some();
                if matches!(method, "let" | "let!" | "subject" | "subject!") && with_block {
                    let name = ruby::first_argument(sibling)
                        .filter(|n| n.kind() == "simple_symbol")
                        .map(|n| text(n, source).trim_start_matches(':').to_string())
                        .or_else(|| method.starts_with("subject").then(|| "subject".into()));
                    if let Some(name) = name {
                        if method.ends_with('!') {
                            eager.push(sibling);
                        }
                        if !lazy.iter().any(|(known, _)| *known == name) {
                            lazy.push((name, sibling));
                        }
                    }
                    continue;
                }
                let setup_method = sibling.kind() == "method"
                    && sibling
                        .child_by_field_name("name")
                        .is_some_and(|n| text(n, source) == "setup");
                if setup_method || matches!(method, "before" | "around" | "setup") && with_block {
                    eager.push(sibling);
                }
            }
        }
        ancestor = scope.parent();
    }
    (eager, lazy)
}

/// The `let` and `subject` definitions the names in `read` reach, directly
/// or through another: each one adds the names it reads to `read` and what
/// it calls to `calls`.
fn lazy_read<'tree>(
    lazy: &[(String, Node<'tree>)],
    read: &mut Vec<String>,
    source: &str,
    bound: &[String],
    calls: &mut BTreeSet<String>,
) -> Vec<Node<'tree>> {
    let mut used: Vec<Node<'tree>> = Vec::new();
    let mut next = 0;
    while next < read.len() {
        let name = read[next].clone();
        next += 1;
        if let Some((_, definition)) = lazy.iter().find(|(known, _)| *known == name)
            && !used.contains(definition)
        {
            used.push(*definition);
            identifiers(*definition, source, read);
            read_calls(*definition, source, bound, calls);
        }
    }
    used
}

/// What a hook or definition calls: its calls, and the bare names it reads
/// that are not locals or `let`s (`bound`).
fn read_calls(node: Node<'_>, source: &str, bound: &[String], calls: &mut BTreeSet<String>) {
    walk(node, source, calls, &mut Vec::new(), &mut Vec::new());
    let mut names = Vec::new();
    identifiers(node, source, &mut names);
    calls.extend(names.into_iter().filter(|name| !bound.contains(name)));
}

fn identifiers(node: Node<'_>, source: &str, names: &mut Vec<String>) {
    if node.kind() == "identifier" {
        names.push(text(node, source).to_string());
        return;
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        identifiers(child, source, names);
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
    let mut requests = Vec::new();
    walk(node, source, &mut calls, &mut tokens, &mut requests);
    let shingles = tokens.windows(SHINGLE_TOKENS).map(fast_hash).collect();
    found.push(TestCase {
        name,
        span: start..node.end_byte(),
        line: line_of(source, node.start_byte()),
        end_line: line_of(source, node.end_byte().saturating_sub(1)),
        calls,
        subjects: Vec::new(),
        suite: Vec::new(),
        hooks: Vec::new(),
        hook_calls: BTreeSet::new(),
        requests,
        shingles,
    });
}

fn walk<'a>(
    node: Node<'_>,
    source: &'a str,
    calls: &mut BTreeSet<String>,
    tokens: &mut Vec<&'a str>,
    requests: &mut Vec<super::routes::Route>,
) {
    if is_comment(node) {
        return;
    }
    match node.kind() {
        "call_expression" | "call" | "invocation_expression" => {
            if let Some(name) = call_name(node, source) {
                calls.insert(name);
            }
        }
        "method_invocation" => {
            if let Some(name) = node.child_by_field_name("name") {
                calls.insert(text(name, source).to_string());
            }
            requests.extend(super::routes::request(node, source));
        }
        // C#, PHP and Java: constructing the class under test calls its constructor.
        "object_creation_expression" => {
            if let Some(name) = match node.child_by_field_name("type") {
                Some(t) => callee_name(t, source),
                None => crate::analysis::php::callee_name(node, source),
            } {
                calls.insert(name);
            }
        }
        "jsx_opening_element" | "jsx_self_closing_element" => {
            if let Some(name) = node.child_by_field_name("name") {
                calls.insert(text(name, source).to_string());
            }
        }
        "token_tree"
            if node
                .parent()
                .is_some_and(|p| p.kind() == "macro_invocation") =>
        {
            macro_calls(node, source, calls);
        }
        kind if crate::analysis::php::CALLS.contains(&kind) => {
            calls.extend(crate::analysis::php::callee_name(node, source));
        }
        _ => {}
    }
    if node.child_count() == 0 {
        // Identifiers are normalized so renamed locals still compare as
        // similar; PHP names them `name`.
        tokens.push(
            if node.kind().ends_with("identifier") || node.kind() == "name" {
                "\u{1}id"
            } else {
                text(node, source)
            },
        );
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk(child, source, calls, tokens, requests);
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

/// A camelCase getter or setter, such as Java's `setBirthDate` or `isNew`.
fn accessor(name: &str) -> bool {
    ["get", "set", "is"].iter().any(|prefix| {
        name.strip_prefix(prefix)
            .is_some_and(|rest| rest.starts_with(|c: char| c.is_ascii_uppercase()))
    })
}

/// Pairs that share a subject and look alike, most similar first, capped.
pub fn pairs(cases: &[TestCase]) -> (Vec<TestPair>, usize) {
    let mut uses = BTreeMap::<&str, usize>::new();
    for subject in cases.iter().flat_map(|c| &c.subjects) {
        *uses.entry(subject).or_default() += 1;
    }
    let mut found = Vec::new();
    for a in 0..cases.len() {
        for b in a + 1..cases.len() {
            // Tests set up their objects through setters, fixtures and
            // clients that most of the file's tests call, such as
            // `setBirthDate` or `create_user`; the function a pair checks is
            // another one it shares, the one the fewest tests call.
            let setup = |s: &str| {
                let common = uses[s] * 2 > cases.len();
                (accessor(s), common, if common { 0 } else { uses[s] })
            };
            let Some(subject) = cases[a]
                .subjects
                .iter()
                .filter(|s| cases[b].subjects.contains(s))
                .min_by_key(|s| setup(s))
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

    /// Each case's name and suite.
    fn named(found: &[TestCase]) -> Vec<(&str, Vec<&str>)> {
        found
            .iter()
            .map(|c| {
                (
                    c.name.as_str(),
                    c.suite.iter().map(String::as_str).collect(),
                )
            })
            .collect()
    }

    /// The first and last lines of the test code located in a file.
    fn located(path: &str, source: &str) -> Vec<(usize, usize)> {
        crate::test_locations::locate_tests(Path::new(path), source)
            .unwrap()
            .ranges
            .iter()
            .map(|r| (r.start_line, r.end_line))
            .collect()
    }

    #[test]
    fn phpunit_methods_and_pest_calls_are_test_cases() {
        let phpunit = "<?php\nnamespace Tests;\n\nuse PHPUnit\\Framework\\TestCase;\n\nfinal class TotalTest extends TestCase\n{\n    private function rows(): array { return [1]; }\n\n    public function testAdds(): void\n    {\n        $this->assertSame(3, total([1, 2]));\n    }\n\n    /** @test */\n    public function it_is_empty(): void\n    {\n        $this->assertSame(0, (new Summer())->total([]));\n    }\n\n    #[Test]\n    public function keeps_order(): void {}\n}\n\nclass Helper { public function testLike() {} }\n";
        assert_eq!(
            names("tests/TotalTest.php", phpunit),
            ["testAdds", "it_is_empty", "keeps_order"]
        );
        let found = cases(Path::new("tests/TotalTest.php"), phpunit).unwrap();
        assert!(found[0].calls.contains("total"));
        assert!(found[1].calls.contains("Summer"));
        assert_eq!(found[0].suite, ["TotalTest"]);
        let classes = located("tests/TotalTest.php", phpunit);
        assert_eq!(classes.len(), 1, "the TestCase class, not the helper");
        assert_eq!(classes[0].0, 6);
        let pest = "<?php\n\ndescribe('total', function () {\n    it('adds', function () {\n        expect(total([1, 2]))->toBe(3);\n    });\n});\n\ntest('empty', fn () => expect(total([]))->toBe(0))->skip();\n\nfunction helper() { return 1; }\n";
        assert_eq!(names("tests/Unit/TotalTest.php", pest), ["adds", "empty"]);
        let found = cases(Path::new("tests/Unit/TotalTest.php"), pest).unwrap();
        assert_eq!(found[0].suite, ["total"]);
        let calls = located("tests/Unit/TotalTest.php", pest);
        assert_eq!(calls.len(), 2, "{calls:?}");
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
        assert_eq!(located("total_test.go", go).len(), 2);
        let csharp = "using Xunit;\n\nnamespace Shop.Tests;\n\npublic class OrderTotal\n{\n    [Fact]\n    public void IsZeroForNewOrder()\n    {\n        Assert.Equal(0, new Order().Total());\n    }\n\n    [Theory]\n    [InlineData(1)]\n    public void Adds(int count) => Assert.Equal(count, Build(count).Total());\n\n    [Xunit.FactAttribute]\n    public void Qualified() { }\n\n    private static Order Build(int count) => new Order(count);\n}\n\n[TestClass]\npublic class Checks\n{\n    [TestMethod]\n    public void Runs() { }\n\n    [NUnit.Framework.TestCase(2)]\n    public void Case(int n) { }\n}\n";
        assert_eq!(
            names("OrderTotal.cs", csharp),
            ["IsZeroForNewOrder", "Adds", "Qualified", "Runs", "Case"]
        );
        let found = cases(Path::new("OrderTotal.cs"), csharp).unwrap();
        assert_eq!(found[0].suite, ["OrderTotal"]);
        assert!(found[0].calls.contains("Order") && found[0].calls.contains("Total"));
        assert!(found[1].calls.contains("Build"));
        assert_eq!(
            located("OrderTotal.cs", csharp),
            [(5, 21), (23, 31)],
            "whole test classes"
        );
    }

    const RSPEC: &str = "require 'spec_helper'\n\nRSpec.describe Invoice do\n  let(:rows) { [Row.new(2)] }\n  let(:unused) { expensive_fixture }\n  subject { Invoice.new(rows) }\n\n  before do\n    Currency.reset\n  end\n\n  it \"totals its rows\" do\n    expect(subject.total).to eq 2\n  end\n\n  context \"when empty\" do\n    let(:rows) { [] }\n\n    it { is_expected.to be_empty }\n    its(:total) { is_expected.to eq 0 }\n\n    it \"totals its rows\" do\n      expect(subject.total).to eq 0\n    end\n  end\n\n  describe \"#currency\" do\n    it \"is euro\" do\n      expect(currency_of(subject)).to eq \"EUR\"\n    end\n  end\nend\n";

    #[test]
    fn rspec_examples_get_their_groups_hooks_and_the_calls_of_what_they_read() {
        let found = cases(Path::new("spec/invoice_spec.rb"), RSPEC).unwrap();
        assert_eq!(
            named(&found),
            [
                ("Invoice > totals its rows", vec!["Invoice"]),
                (
                    "it { is_expected.to be_empty }",
                    vec!["Invoice", "when empty"]
                ),
                ("its :total", vec!["Invoice", "when empty"]),
                (
                    "when empty > totals its rows",
                    vec!["Invoice", "when empty"]
                ),
                ("is euro", vec!["Invoice", "#currency"]),
            ],
            "examples titled alike are told apart by their innermost groups"
        );
        let first = &found[0];
        let hooks: Vec<&str> = first.hooks.iter().map(|h| &RSPEC[h.clone()]).collect();
        assert_eq!(
            hooks,
            [
                "let(:rows) { [Row.new(2)] }",
                "subject { Invoice.new(rows) }",
                "before do\n    Currency.reset\n  end"
            ],
            "a `let` the example never reads is not its setup"
        );
        assert!(first.calls.contains("Invoice") && first.calls.contains("Row"));
        assert!(first.calls.contains("total") && !first.calls.contains("rows"));
        assert!(first.hook_calls.contains("reset"));
        let empty = &found[3];
        assert_eq!(
            &RSPEC[empty.hooks[0].clone()],
            "subject { Invoice.new(rows) }"
        );
        assert_eq!(&RSPEC[empty.hooks[2].clone()], "let(:rows) { [] }");
        assert!(found[4].calls.contains("currency_of"), "a bare call");
        assert_eq!(
            located("spec/invoice_spec.rb", RSPEC).len(),
            1,
            "the outer group holds every example"
        );
    }

    #[test]
    fn minitest_methods_and_rails_test_blocks_are_cases_and_rake_tasks_are_not() {
        let minitest = "require_relative 'test_helper'\n\nclass InvoiceTest < Minitest::Test\n  def setup\n    @invoice = Invoice.new\n  end\n\n  def test_total\n    assert_equal 0, @invoice.total\n  end\n\n  def build_row\n    Row.new\n  end\nend\n\nclass Helper\n  def test_like\n  end\nend\n";
        let found = cases(Path::new("test/invoice_test.rb"), minitest).unwrap();
        assert_eq!(named(&found), [("test_total", vec!["InvoiceTest"])]);
        assert_eq!(
            &minitest[found[0].hooks[0].clone()],
            "def setup\n    @invoice = Invoice.new\n  end"
        );
        let rails = "class InvoiceTest < ActiveSupport::TestCase\n  test \"totals rows\" do\n    assert_equal 2, Invoice.new([2]).total\n  end\nend\n";
        assert_eq!(names("test/models/invoice_test.rb", rails), ["totals rows"]);
        let rakefile = "task :default => :test\n\ntest(:unit) do |t|\n  t.pattern = 'test/**/*_test.rb'\nend\n";
        assert!(names("tasks.rb", rakefile).is_empty());
        assert!(located("tasks.rb", rakefile).is_empty());
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

    const JUNIT: &str = "package app;\n\nimport org.junit.jupiter.api.Test;\n\nclass TotalTests {\n\n\tprivate final Totals totals = new Totals();\n\n\t@Test\n\tvoid adds() {\n\t\tassertEquals(3, totals.sum(1, 2));\n\t}\n\n\t@ParameterizedTest\n\t@ValueSource(ints = {1, 2})\n\tvoid keeps(int value) {\n\t\tassertEquals(value, totals.sum(value));\n\t}\n\n\t@MultiLocaleTest\n\tvoid formats(Locale locale) {\n\t\tassertEquals(\"1\", totals.format(1, locale));\n\t}\n\n\tprivate int helper() {\n\t\treturn totals.sum();\n\t}\n\n\t@Nested\n\tclass Empty {\n\t\t@org.junit.jupiter.api.Test\n\t\tvoid isZero() {\n\t\t\tassertEquals(0, new Totals().sum());\n\t\t}\n\t}\n}\n";

    #[test]
    fn junit_methods_are_cases_and_nested_classes_are_their_suites() {
        let found = cases(Path::new("src/test/java/app/TotalTests.java"), JUNIT).unwrap();
        assert_eq!(
            named(&found),
            [
                ("adds", vec![]),
                ("keeps", vec![]),
                ("formats", vec![]),
                ("isZero", vec!["Empty"])
            ]
        );
        assert!(found[0].calls.contains("sum") && found[3].calls.contains("Totals"));
        // The whole test class is test code, its fields and helpers included.
        assert_eq!(located("TotalTests.java", JUNIT), [(5, 36)]);
        let junit3 = "public class TotalTest extends junit.framework.TestCase {\n\tpublic void testAdds() {\n\t\tassertEquals(3, Totals.sum(1, 2));\n\t}\n\n\tprivate void check() {}\n}\n";
        assert_eq!(names("TotalTest.java", junit3), ["testAdds"]);
        let subclass = "class HttpSessionTest extends SessionTest {\n\t@BeforeAll\n\tstatic void useHttp() {\n\t\tenableHttp();\n\t}\n}\n";
        assert_eq!(located("HttpSessionTest.java", subclass).len(), 1);
        let application = "class Totals {\n\tint sum(int... values) {\n\t\treturn 0;\n\t}\n\n\tvoid testConnection() {}\n}\n";
        assert!(names("Totals.java", application).is_empty());
        assert!(located("Totals.java", application).is_empty());
    }

    #[test]
    fn a_pair_is_about_the_function_its_tests_check_not_the_setters_they_call() {
        let java = "class PetValidatorTests {\n\t@Test\n\tvoid validateWithInvalidPetName() {\n\t\tpet.setBirthDate(birthDate);\n\t\tpet.setName(\"\");\n\t\tvalidator.validate(pet, errors);\n\t\tassertTrue(errors.hasFieldErrors(\"name\"));\n\t}\n\n\t@Test\n\tvoid validateWithLongPetName() {\n\t\tpet.setBirthDate(birthDate);\n\t\tpet.setName(\"A\".repeat(31));\n\t\tvalidator.validate(pet, errors);\n\t\tassertTrue(errors.hasFieldErrors(\"name\"));\n\t}\n}\n";
        let mut found = cases(Path::new("PetValidatorTests.java"), java).unwrap();
        link(
            &mut found,
            &BTreeSet::from(["setBirthDate".into(), "setName".into(), "validate".into()]),
        );
        assert_eq!(pairs_of(&found)[0], "validate");
        // Accessors alone are still a shared subject.
        link(
            &mut found,
            &BTreeSet::from(["setBirthDate".into(), "setName".into()]),
        );
        assert_eq!(pairs_of(&found), ["setBirthDate"]);
        assert!(!accessor("settle") && !accessor("isolate") && accessor("isNew"));
        // A function most of the file's tests call is setup: all four build
        // a document, two of them transcode bytes first.
        let decoding = "class DataUtilTest {\n\t@Test\n\tvoid discardsMark() {\n\t\tDocument doc = Documents.build(Codec.transcode(bytes, \"UTF-8\"));\n\t\tassertEquals(\"One\", doc.title());\n\t}\n\n\t@Test\n\tvoid discardsMarkWithoutCharset() {\n\t\tDocument doc = Documents.build(Codec.transcode(bytes, null));\n\t\tassertEquals(\"One\", doc.title());\n\t}\n\n\t@Test\n\tvoid readsTitle() {\n\t\tDocument doc = Documents.build(text);\n\t\tassertEquals(\"OK\", doc.title());\n\t}\n\n\t@Test\n\tvoid readsBody() {\n\t\tDocument doc = Documents.build(html);\n\t\tassertEquals(\"Two\", doc.body());\n\t}\n\n}\n";
        let mut found = cases(Path::new("DataUtilTest.java"), decoding).unwrap();
        link(
            &mut found,
            &BTreeSet::from(["build".into(), "transcode".into()]),
        );
        assert_eq!(subject_of(&found, (0, 1)), "transcode");
        // Among names most tests call, none is more the subject than another:
        // the first in order stays.
        link(
            &mut found,
            &BTreeSet::from(["build".into(), "title".into()]),
        );
        assert_eq!(subject_of(&found, (0, 1)), "build");
    }

    fn pairs_of(found: &[TestCase]) -> Vec<String> {
        pairs(found).0.into_iter().map(|p| p.subject).collect()
    }

    /// The subject of the pair of cases at `tests`.
    fn subject_of(found: &[TestCase], tests: (usize, usize)) -> String {
        pairs(found)
            .0
            .into_iter()
            .find(|p| (p.a, p.b) == tests)
            .unwrap()
            .subject
    }
}
