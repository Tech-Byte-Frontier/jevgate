//! Deterministic file eligibility, then a test-versus-mix classification when a
//! test path still contains other code. Parsers locate structural tests; they
//! do not decide maintainability. The result is a gate view: which code the
//! application rules and the test rules judge.
use crate::{
    inventory::Input,
    options::CheckArgs,
    requests::TokenBudget,
    response,
    schema::{FileResult, SourceRange, Status},
};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::Path;
use tree_sitter::Node;

pub const VERSION: &str = "file-kind-v5";
const PORTION_PRESENT: f64 = response::REVIEW_PROBABILITY;
const PORTION_ABSENT: f64 = 0.20;
const PURPOSE_UNITS: usize = 24;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Classification {
    #[serde(default)]
    pub version: String,
    pub kind: String,
    #[serde(default)]
    pub basis: String,
    #[serde(default)]
    pub gate: String,
    #[serde(default)]
    pub language: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub separated_tests: Vec<SourceRange>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purpose: Option<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unresolved_units: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reason: String,
    /// In-memory handoff from the purpose response to the gate view.
    #[serde(skip)]
    pub stage: String,
    #[serde(skip)]
    pub units: Value,
}

/// Which code of one file the rules judge.
#[derive(Clone, Debug)]
pub(crate) struct View {
    pub classification: Classification,
    /// Application rules judge the code outside `test_lines`.
    pub application: bool,
    /// Test rules judge the code inside `test_lines` (with `--include-tests`).
    pub tests: bool,
    /// Test code: the whole file for a test file, else the separated tests.
    pub test_lines: Vec<SourceRange>,
}

enum Action {
    Skip,
    Purpose,
    Judge,
}

struct Prepared {
    classification: Classification,
    action: Action,
}

pub(crate) enum Plan {
    Skip(Classification),
    /// The source was not sent. The file is needs-context, not a verdict.
    Unsent(Classification),
    Purpose(Classification, Value),
    Ready(View),
}

pub fn excluded_reason(role: &str) -> &'static str {
    match role {
        "script" => {
            "Operational script. Maintainability gates apply to application and library source."
        }
        "declarations" => "Type declarations have no implementation for these gates.",
        "generated" => "Generated code. Review its generator or source definitions instead.",
        _ => "Outside source/test semantic scope",
    }
}

pub fn excluded(role: &str, path: &Path) -> Classification {
    classification(
        role,
        "deterministic",
        "excluded",
        excluded_reason(role),
        language(path),
    )
}

const LISTED_OPERATIONS: usize = 12;

pub fn operation_phrase(path: &Path, source: &str) -> String {
    let names: Vec<String> = crate::context_units::review_targets(path, source)
        .into_iter()
        .map(|(name, _, _)| name)
        .collect();
    if names.is_empty() {
        return "No operations were parsed.".into();
    }
    let extra = names.len().saturating_sub(LISTED_OPERATIONS);
    let shown = names
        .into_iter()
        .take(LISTED_OPERATIONS)
        .collect::<Vec<_>>();
    if extra == 0 {
        format!("Operations: {}.", shown.join(", "))
    } else {
        format!("Operations: {}, and {extra} more.", shown.join(", "))
    }
}

pub fn unsent(path: &Path, named_source: &str, detail: &str) -> Classification {
    let mut class = classification("oversized", "deterministic", "unsent", "", language(path));
    class.reason = format!("{detail} {}", operation_phrase(path, named_source));
    class
}

pub fn language(path: &Path) -> &'static str {
    match extension(path).as_str() {
        "rs" => "Rust",
        "py" => "Python",
        "js" | "jsx" | "mjs" | "cjs" => "JavaScript",
        "ts" | "tsx" | "mts" | "cts" => "TypeScript",
        "go" => "Go",
        "java" => "Java",
        "kt" | "kts" => "Kotlin",
        "scala" => "Scala",
        "c" | "h" => "C",
        "cpp" | "cc" | "cxx" | "hpp" => "C++",
        "cs" => "C#",
        "rb" => "Ruby",
        "php" => "PHP",
        "swift" => "Swift",
        "dart" => "Dart",
        "lua" => "Lua",
        "ex" | "exs" => "Elixir",
        "zig" => "Zig",
        "vue" => "Vue",
        "svelte" => "Svelte",
        "sql" => "SQL",
        "sh" | "bash" | "zsh" | "fish" | "ksh" | "csh" | "ps1" | "bat" | "cmd" => "shell",
        _ => "unknown",
    }
}

pub(crate) fn plan(input: &Input, args: &CheckArgs, budget: &TokenBudget) -> Result<Plan> {
    let prepared = prepare(input, args)?;
    match prepared.action {
        Action::Skip => Ok(Plan::Skip(prepared.classification)),
        Action::Purpose => {
            let request = purpose_request(input, args, &prepared.classification)?;
            if !budget.fits(&request) {
                let source = input.source.as_deref().unwrap_or("");
                return Ok(Plan::Unsent(unsent(
                    &input.result.path,
                    source,
                    "The file-purpose request does not fit the provider limit, so it was not sent.",
                )));
            }
            Ok(Plan::Purpose(prepared.classification, request))
        }
        Action::Judge => Ok(Plan::Ready(view(input, args, prepared.classification))),
    }
}

fn view(input: &Input, args: &CheckArgs, classification: Classification) -> View {
    if classification.kind == "tests" {
        let lines = input.source.as_deref().unwrap_or("").lines().count().max(1);
        return View {
            application: false,
            tests: args.include_tests,
            test_lines: vec![SourceRange {
                start_line: 1,
                end_line: lines,
            }],
            classification,
        };
    }
    View {
        application: true,
        tests: args.include_tests && !classification.separated_tests.is_empty(),
        test_lines: classification.separated_tests.clone(),
        classification,
    }
}

pub fn record_purpose(file: &mut FileResult, request: &Value, body: &Value) -> Result<()> {
    let class = file
        .classification
        .as_mut()
        .context("Missing file classification")?;
    class.purpose = Some(body["answers"].clone());
    class.units = request["state"]["units"].clone();
    class.stage = "answered".into();
    Ok(())
}

pub(crate) fn decide_after_purpose(
    input: &Input,
    args: &CheckArgs,
    file: &mut FileResult,
) -> Result<Option<View>> {
    let answers = file
        .classification
        .as_ref()
        .and_then(|class| class.purpose.clone())
        .context("Missing file-purpose answer")?;
    let units = file
        .classification
        .as_ref()
        .map(|class| class.units.clone())
        .unwrap_or(Value::Null);
    let structural = file
        .classification
        .as_ref()
        .map(|class| class.separated_tests.clone())
        .unwrap_or_default();
    let choice = &answers["file_purpose"];
    let tests = probability(choice, "tests");
    let mixed = probability(choice, "mixed");
    let application = probability(choice, "application");
    let original = input.source.as_deref().unwrap_or("");
    let confident = |value| response::probability_at_least(value, PORTION_PRESENT);
    file.contains_tests = true;
    {
        let class = file
            .classification
            .as_mut()
            .context("Missing file classification")?;
        class.basis = "model".into();
        class.purpose = Some(choice.clone());
        class.units = Value::Null;
        class.stage.clear();
    }

    if confident(tests) {
        return Ok(finish_tests(input, args, file));
    }
    let mut separated = structural;
    let mut unresolved = Vec::new();
    if confident(mixed) {
        // Portion answers are speculative. Use them only on the mixed branch.
        let listed = units.as_array().map(|items| items.len()).unwrap_or(0);
        for index in 0..listed {
            let unit = &units[index];
            let name = unit["name"].as_str().unwrap_or("unit");
            let noul = answers[&format!("test_portion_{index}")]["noul"]
                .as_f64()
                .context("Missing test-portion probability")?;
            if response::probability_at_least(noul, PORTION_PRESENT) {
                separated.push(SourceRange {
                    start_line: unit["start_line"].as_u64().unwrap_or(1) as usize,
                    end_line: unit["end_line"].as_u64().unwrap_or(1) as usize,
                });
            } else if noul > PORTION_ABSENT {
                unresolved.push(name.to_string());
            }
        }
        separated = merge_ranges(separated);
        if !has_implementation(&input.result.path, &blank_lines(original, &separated)) {
            return Ok(finish_tests(input, args, file));
        }
        let class = file.classification.as_mut().unwrap();
        class.kind = "mixed".into();
        class.gate = "application".into();
        class.separated_tests = separated;
        class.unresolved_units = unresolved;
        class.reason = separated_reason(&class.separated_tests);
        return Ok(Some(view(input, args, class.clone())));
    }

    if confident(application) && separated.is_empty() {
        let class = file.classification.as_mut().unwrap();
        class.kind = "application".into();
        class.gate = "application".into();
        class.separated_tests.clear();
        class.reason = "Classified as application or library code.".into();
        return Ok(Some(view(input, args, class.clone())));
    }
    if !has_implementation(&input.result.path, &blank_lines(original, &separated)) {
        return Ok(finish_tests(input, args, file));
    }
    let class = file.classification.as_mut().unwrap();
    class.kind = if !separated.is_empty() && confident(application) {
        "mixed"
    } else {
        "unresolved"
    }
    .into();
    class.gate = "application".into();
    class.separated_tests = separated;
    class.unresolved_units = unresolved;
    class.reason = if class.kind == "mixed" {
        separated_reason(&class.separated_tests)
    } else if units.as_array().is_none_or(|items| items.is_empty())
        && mixed > application
        && mixed > tests
    {
        "The file looks mixed, but it has no separable test boundaries, so the rules judge the whole file.".into()
    } else {
        "File purpose stayed below 0.80, so the rules judge the source without dropping unresolved regions.".into()
    };
    Ok(Some(view(input, args, class.clone())))
}

fn finish_tests(input: &Input, args: &CheckArgs, file: &mut FileResult) -> Option<View> {
    file.contains_tests = true;
    let class = file.classification.as_mut().unwrap();
    class.kind = "tests".into();
    class.separated_tests.clear();
    class.unresolved_units.clear();
    if !args.include_tests {
        class.gate = "excluded".into();
        class.reason = "Test file. Pass --include-tests to judge tests.".into();
        file.status = Status::NotApplicable;
        return None;
    }
    class.gate = "tests".into();
    class.reason = "Test file. Test rules judge this code.".into();
    Some(view(input, args, class.clone()))
}

fn prepare(input: &Input, args: &CheckArgs) -> Result<Prepared> {
    let original = input.source.clone().unwrap_or_default();
    let path = &input.result.path;
    let located = locate_tests(path, &original)?;
    let tests_class = || {
        let mut class = classification(
            "tests",
            "deterministic",
            "tests",
            "Test file. Test rules judge this code.",
            language(path),
        );
        if !args.include_tests {
            class.gate = "excluded".into();
            class.reason = "Test file. Pass --include-tests to judge tests.".into();
        }
        let action = if args.include_tests {
            Action::Judge
        } else {
            Action::Skip
        };
        Prepared {
            classification: class,
            action,
        }
    };
    if located.whole_file {
        return Ok(tests_class());
    }
    let separated = merge_ranges(located.ranges);
    let remaining = has_implementation(path, &blank_lines(&original, &separated));
    let structural_tests = !separated.is_empty();
    let mut class = classification(
        "application",
        "deterministic",
        "application",
        "",
        language(path),
    );
    class.separated_tests = separated;
    class.unresolved_units = located.unresolved;
    // A test path with structural tests holds their support code; only a test
    // path without any asks what the file contains.
    if input.result.role == "test" && remaining && !structural_tests {
        class.kind = "unresolved".into();
        class.stage = "purpose".into();
        class.reason =
            "This test path still contains other code. Its purpose is classified before the rules."
                .into();
        return Ok(Prepared {
            classification: class,
            action: Action::Purpose,
        });
    }
    let test_path = input.result.role == "test";
    if (test_path && structural_tests) || (!remaining && (test_path || structural_tests)) {
        return Ok(tests_class());
    }
    if structural_tests {
        class.kind = "mixed".into();
        class.reason = separated_reason(&class.separated_tests);
    }
    Ok(Prepared {
        classification: class,
        action: Action::Judge,
    })
}

fn purpose_request(input: &Input, args: &CheckArgs, class: &Classification) -> Result<Value> {
    let source = input.source.as_deref().context("Missing selected source")?;
    let mut units = Vec::new();
    for (name, range, _) in crate::context_units::review_targets(&input.result.path, source) {
        if overlaps(&range, &class.separated_tests) || units.len() == PURPOSE_UNITS {
            continue;
        }
        units.push(json!({
            "name": name,
            "start_line": range.start_line,
            "end_line": range.end_line,
            "excerpt": excerpt(source, &range),
        }));
    }
    let mut questions = serde_json::Map::new();
    questions.insert(
        "file_purpose".into(),
        crate::units::questions::file_purpose(),
    );
    for index in 0..units.len() {
        questions.insert(
            format!("test_portion_{index}"),
            crate::units::questions::test_portion(index),
        );
    }
    Ok(json!({
        "model": args.model,
        "state": {
            "file": {
                "path": input.result.path,
                "role": input.result.role,
                "language": class.language,
                "source": source,
            },
            "structural_tests": class.separated_tests,
            "units": units,
        },
        "questions": questions,
        "jevgate": {
            "stage": "file-purpose",
            "sources": [{"path": input.result.path, "source_hash": input.result.source_hash}],
        }
    }))
}

struct Located {
    ranges: Vec<SourceRange>,
    unresolved: Vec<String>,
    whole_file: bool,
}

fn locate_tests(path: &Path, source: &str) -> Result<Located> {
    let Some(tree) = crate::locations::parse(path, source)? else {
        return Ok(Located {
            ranges: Vec::new(),
            unresolved: Vec::new(),
            whole_file: false,
        });
    };
    let root = tree.root_node();
    if whole_file_cfg_test(root, source) {
        return Ok(Located {
            ranges: Vec::new(),
            unresolved: Vec::new(),
            whole_file: true,
        });
    }
    let mut spans = Vec::new();
    walk(root, source, pytest_file(path), &mut spans);
    let mut ranges = Vec::new();
    let mut unresolved = Vec::new();
    for (start, end) in merge_spans(spans) {
        if owns_lines(source, start, end) {
            ranges.push(line_range(source, start, end));
        } else {
            unresolved.push("test syntax shares a line with other code".into());
        }
    }
    Ok(Located {
        ranges,
        unresolved,
        whole_file: false,
    })
}

fn walk(node: Node<'_>, source: &str, pytest: bool, spans: &mut Vec<(usize, usize)>) {
    if let Some(span) = rust_test_span(node, source)
        .or_else(|| javascript_test_span(node, source))
        .or_else(|| python_test_span(node, source, pytest))
    {
        spans.push(span);
        return;
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        walk(child, source, pytest, spans);
    }
}

/// pytest collects top-level `test*` functions only from `test_*.py` and `*_test.py`.
fn pytest_file(path: &Path) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    name.ends_with(".py") && (name.starts_with("test_") || name.ends_with("_test.py"))
}

/// A pytest `Test*` class or a `unittest` `TestCase` subclass.
pub(crate) fn python_test_class(node: Node<'_>, source: &str) -> bool {
    node.kind() == "class_definition"
        && (node
            .child_by_field_name("name")
            .is_some_and(|name| child_text(name, source).starts_with("Test"))
            || node
                .child_by_field_name("superclasses")
                .is_some_and(|bases| child_text(bases, source).contains("TestCase")))
}

/// Python test classes anywhere, and top-level `test*` functions in pytest files.
fn python_test_span(node: Node<'_>, source: &str, pytest: bool) -> Option<(usize, usize)> {
    let definition = if node.kind() == "decorated_definition" {
        node.child_by_field_name("definition")?
    } else if matches!(node.kind(), "class_definition" | "function_definition")
        && node.parent()?.kind() != "decorated_definition"
    {
        node
    } else {
        return None;
    };
    let test = python_test_class(definition, source)
        || pytest
            && definition.kind() == "function_definition"
            && node.parent()?.kind() == "module"
            && definition
                .child_by_field_name("name")
                .is_some_and(|name| child_text(name, source).starts_with("test"));
    test.then(|| (node.start_byte(), node.end_byte()))
}

fn rust_test_span(node: Node<'_>, source: &str) -> Option<(usize, usize)> {
    if !matches!(
        node.kind(),
        "function_item" | "function_signature_item" | "mod_item" | "impl_item"
    ) {
        return None;
    }
    let marked = preceding_attributes(node, source)
        .iter()
        .any(|text| attribute_marks_test(text))
        || has_inner_cfg_test(node, source)
        || mod_contains_only_tests(node, source);
    if !marked {
        return None;
    }
    Some((attribute_start(node), node.end_byte()))
}

fn javascript_test_span(node: Node<'_>, source: &str) -> Option<(usize, usize)> {
    if node.kind() != "call_expression" || !is_test_call(&callee(node, source)) {
        return None;
    }
    let statement = statement_span(node);
    Some((statement.start_byte(), statement.end_byte()))
}

fn mod_contains_only_tests(node: Node<'_>, source: &str) -> bool {
    if node.kind() != "mod_item" {
        return false;
    }
    let Some(body) = node.child_by_field_name("body") else {
        return false;
    };
    let mut saw_test = false;
    let mut cursor = body.walk();
    for child in body.named_children(&mut cursor) {
        if child.kind().contains("comment")
            || matches!(
                child.kind(),
                "attribute_item" | "inner_attribute_item" | "use_declaration"
            )
        {
            continue;
        }
        if rust_test_span(child, source).is_some() {
            saw_test = true;
            continue;
        }
        return false;
    }
    saw_test
}

fn whole_file_cfg_test(root: Node<'_>, source: &str) -> bool {
    let mut cursor = root.walk();
    root.named_children(&mut cursor).any(|child| {
        child.kind() == "inner_attribute_item" && cfg_is_test_only(child_text(child, source))
    })
}

fn has_inner_cfg_test(node: Node<'_>, source: &str) -> bool {
    let Some(body) = node.child_by_field_name("body") else {
        return false;
    };
    let mut cursor = body.walk();
    body.named_children(&mut cursor).any(|child| {
        child.kind() == "inner_attribute_item" && cfg_is_test_only(child_text(child, source))
    })
}

pub(crate) fn preceding_attributes<'a>(node: Node<'_>, source: &'a str) -> Vec<&'a str> {
    let mut texts = Vec::new();
    let mut previous = node.prev_named_sibling();
    while let Some(sibling) = previous {
        if sibling.kind() == "attribute_item" {
            texts.push(child_text(sibling, source));
        } else if !sibling.kind().contains("comment") {
            break;
        }
        previous = sibling.prev_named_sibling();
    }
    texts
}

fn attribute_start(node: Node<'_>) -> usize {
    let mut start = node.start_byte();
    let mut previous = node.prev_named_sibling();
    while let Some(sibling) = previous {
        if sibling.kind() == "attribute_item" {
            start = sibling.start_byte();
        } else if !sibling.kind().contains("comment") {
            break;
        }
        previous = sibling.prev_named_sibling();
    }
    start
}

pub(crate) fn attribute_marks_test(text: &str) -> bool {
    cfg_is_test_only(text) || attribute_path(text).is_some_and(is_test_attribute)
}

fn attribute_path(text: &str) -> Option<&str> {
    let start = text.find('[')? + 1;
    let end = text.rfind(']')?;
    let body = text.get(start..end)?.trim();
    let path = body.split(['(', '=']).next()?.trim();
    (!path.is_empty()).then_some(path)
}

fn is_test_attribute(path: &str) -> bool {
    path == "test"
        || path.ends_with("::test")
        || path == "rstest"
        || path.ends_with("::rstest")
        || path == "test_case"
        || path.ends_with("::test_case")
}

/// `cfg(test)` and `all(..., test, ...)` compile only for tests.
/// `not(test)` and `any(test, ...)` can still be production code, so they stay.
fn cfg_is_test_only(text: &str) -> bool {
    let cleaned = strip_strings(text);
    let mut rest = cleaned.as_str();
    while let Some(index) = rest.find("cfg") {
        let boundary = index == 0
            || !rest.as_bytes()[index - 1].is_ascii_alphanumeric()
                && rest.as_bytes()[index - 1] != b'_';
        let after = rest[index + 3..].trim_start();
        if boundary
            && let Some(body) = after.strip_prefix('(')
            && let Some(end) = matching_paren(body)
            && predicate_is_test_only(&body[..end])
        {
            return true;
        }
        rest = &rest[index + 3..];
    }
    false
}

fn predicate_is_test_only(expr: &str) -> bool {
    let expr = expr.trim();
    if expr == "test" {
        return true;
    }
    strip_call(expr, "all").is_some_and(|inner| {
        split_top_level(inner)
            .iter()
            .any(|part| predicate_is_test_only(part))
    })
}

fn strip_call<'a>(expr: &'a str, name: &str) -> Option<&'a str> {
    let rest = expr.trim().strip_prefix(name)?.trim_start();
    let rest = rest.strip_prefix('(')?;
    let end = matching_paren(rest)?;
    rest[end..].trim().is_empty().then_some(rest[..end].trim())
}

fn split_top_level(text: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    for (index, character) in text.char_indices() {
        match character {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                let part = text[start..index].trim();
                if !part.is_empty() {
                    parts.push(part);
                }
                start = index + character.len_utf8();
            }
            _ => {}
        }
    }
    let tail = text[start..].trim();
    if !tail.is_empty() {
        parts.push(tail);
    }
    parts
}

fn matching_paren(text: &str) -> Option<usize> {
    let mut depth = 0i32;
    for (index, character) in text.char_indices() {
        match character {
            '(' => depth += 1,
            ')' if depth == 0 => return Some(index),
            ')' => depth -= 1,
            _ => {}
        }
    }
    None
}

fn strip_strings(text: &str) -> String {
    let mut cleaned = String::new();
    let mut chars = text.chars().peekable();
    while let Some(character) = chars.next() {
        if character == '"' || character == '\'' {
            let quote = character;
            while let Some(next) = chars.next() {
                if next == '\\' {
                    chars.next();
                    continue;
                }
                if next == quote {
                    break;
                }
            }
            cleaned.push(' ');
        } else {
            cleaned.push(character);
        }
    }
    cleaned
}

fn is_test_call(name: &str) -> bool {
    const NAMES: &[&str] = &[
        "describe",
        "xdescribe",
        "test",
        "xtest",
        "fdescribe",
        "it",
        "xit",
        "fit",
        "beforeEach",
        "afterEach",
        "beforeAll",
        "afterAll",
    ];
    NAMES
        .iter()
        .any(|candidate| name == *candidate || name.starts_with(&format!("{candidate}.")))
}

fn callee(node: Node<'_>, source: &str) -> String {
    node.child_by_field_name("function")
        .map(|child| child_text(child, source).trim().to_string())
        .unwrap_or_default()
}

fn statement_span(node: Node<'_>) -> Node<'_> {
    let mut current = node;
    while let Some(parent) = current.parent() {
        if parent.kind() == "expression_statement" {
            return parent;
        }
        if matches!(parent.kind(), "program" | "source_file" | "module") {
            break;
        }
        current = parent;
    }
    node
}

fn child_text<'a>(node: Node<'_>, source: &'a str) -> &'a str {
    node.utf8_text(source.as_bytes()).unwrap_or("")
}

fn owns_lines(source: &str, start: usize, end: usize) -> bool {
    if start > end || end > source.len() {
        return false;
    }
    let line_start = source[..start].rfind('\n').map_or(0, |index| index + 1);
    let line_end = source[end..]
        .find('\n')
        .map_or(source.len(), |index| end + index);
    source[line_start..start].trim().is_empty() && source[end..line_end].trim().is_empty()
}

fn line_range(source: &str, start: usize, end: usize) -> SourceRange {
    let start_line = source[..start]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count()
        + 1;
    let end_at = end.saturating_sub(1).max(start);
    let end_line = source[..end_at.min(source.len())]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count()
        + 1;
    SourceRange {
        start_line,
        end_line,
    }
}

fn has_implementation(path: &Path, source: &str) -> bool {
    if !crate::context_units::review_targets(path, source).is_empty() {
        return true;
    }
    source.lines().any(|line| {
        let line = line.trim();
        !line.is_empty()
            && !line.starts_with("//")
            && !line.starts_with('#')
            && !line.starts_with("/*")
            && !line.starts_with('*')
            && !line.starts_with("use ")
            && !line.starts_with("pub use ")
            && !line.starts_with("import ")
            && !line.starts_with("from ")
    })
}

fn overlaps(range: &SourceRange, ranges: &[SourceRange]) -> bool {
    ranges
        .iter()
        .any(|other| range.start_line <= other.end_line && other.start_line <= range.end_line)
}

fn excerpt(source: &str, range: &SourceRange) -> String {
    let text = source
        .lines()
        .skip(range.start_line.saturating_sub(1))
        .take(range.end_line.saturating_sub(range.start_line) + 1)
        .collect::<Vec<_>>()
        .join("\n");
    if text.len() <= 1500 {
        text
    } else {
        format!("{}…", text.chars().take(1500).collect::<String>())
    }
}

fn blank_lines(source: &str, ranges: &[SourceRange]) -> String {
    if ranges.is_empty() {
        return source.to_string();
    }
    let mut blank = vec![false; source.lines().count()];
    for range in ranges {
        for line in range.start_line..=range.end_line {
            if let Some(slot) = blank.get_mut(line.saturating_sub(1)) {
                *slot = true;
            }
        }
    }
    let mut out = String::with_capacity(source.len());
    for (index, line) in source.lines().enumerate() {
        if blank.get(index).copied().unwrap_or(false) {
            out.push_str(&" ".repeat(line.len()));
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    if !source.ends_with('\n') {
        out.pop();
    }
    out
}

fn merge_spans(mut spans: Vec<(usize, usize)>) -> Vec<(usize, usize)> {
    spans.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for span in spans {
        if let Some(last) = merged.last_mut()
            && span.0 <= last.1
        {
            last.1 = last.1.max(span.1);
            continue;
        }
        merged.push(span);
    }
    merged
}

fn merge_ranges(mut ranges: Vec<SourceRange>) -> Vec<SourceRange> {
    ranges.sort_by_key(|range| (range.start_line, range.end_line));
    let mut merged: Vec<SourceRange> = Vec::new();
    for range in ranges {
        if let Some(last) = merged.last_mut()
            && range.start_line <= last.end_line.saturating_add(1)
        {
            last.end_line = last.end_line.max(range.end_line);
            continue;
        }
        merged.push(range);
    }
    merged
}

fn separated_reason(ranges: &[SourceRange]) -> String {
    format!(
        "Tests were separated from the application code.{}",
        ranges_phrase(ranges)
    )
}

fn ranges_phrase(ranges: &[SourceRange]) -> String {
    if ranges.is_empty() {
        return String::new();
    }
    let listed = ranges
        .iter()
        .take(8)
        .map(|range| {
            if range.start_line == range.end_line {
                format!("line {}", range.start_line)
            } else {
                format!("lines {}–{}", range.start_line, range.end_line)
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!(" Separated tests: {listed}.")
}

fn probability(answer: &Value, key: &str) -> f64 {
    let Some(probabilities) = answer["probabilities"].as_object() else {
        return 0.0;
    };
    let mass: f64 = probabilities.values().filter_map(Value::as_f64).sum();
    if mass <= 0.0 {
        return 0.0;
    }
    probabilities
        .get(key)
        .and_then(Value::as_f64)
        .unwrap_or(0.0)
        / mass
}

fn classification(
    kind: &str,
    basis: &str,
    gate: &str,
    reason: &str,
    language_name: &str,
) -> Classification {
    Classification {
        version: VERSION.into(),
        kind: kind.into(),
        basis: basis.into(),
        gate: gate.into(),
        language: language_name.into(),
        separated_tests: Vec::new(),
        purpose: None,
        unresolved_units: Vec::new(),
        reason: reason.into(),
        stage: String::new(),
        units: Value::Null,
    }
}

fn extension(path: &Path) -> String {
    path.extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::Status;
    use crate::tests::{Project, args, run};

    const MIXED: &str = "fn production(value: &str) -> String {\n    value.trim().to_string()\n}\n\n#[cfg(test)]\nmod tests {\n    use super::production;\n\n    fn helper(value: &str) -> String {\n        production(value)\n    }\n\n    #[test]\n    fn checks_production() {\n        assert_eq!(helper(\" a \"), \"a\");\n    }\n}\n";

    fn view_of(project: &Project, options: &CheckArgs) -> View {
        let input = crate::inventory::collect(options, &project.context(), &[])
            .unwrap()
            .remove(0);
        match plan(&input, options, &TokenBudget::default()).unwrap() {
            Plan::Ready(view) => view,
            _ => panic!("expected a gate view"),
        }
    }

    #[test]
    fn scripts_and_declarations_are_reported_and_not_judged() {
        let project = Project::new();
        project.write("deploy.sh", "echo ready\n");
        project.write("tool.bash", "echo ready\n");
        project.write("types.d.ts", "export type Id = string;\n");
        project.write("src/lib.rs", "pub fn live() -> i32 { 1 }\n");
        let mut options = args();
        options.source_extension = vec!["bash".into()];
        let inputs = crate::inventory::collect(&options, &project.context(), &[]).unwrap();
        let file = |suffix: &str| {
            inputs
                .iter()
                .find(|input| input.result.path.ends_with(suffix))
                .unwrap()
        };
        let script = file("deploy.sh");
        assert_eq!(script.result.role, "script");
        assert_eq!(script.result.status, Status::Skipped);
        assert!(script.source.is_none());
        assert!(
            script
                .result
                .error
                .as_deref()
                .unwrap()
                .contains("Operational script")
        );
        assert_eq!(file("tool.bash").result.status, Status::Skipped);
        assert_eq!(file("types.d.ts").result.status, Status::Skipped);
        assert!(file("lib.rs").result.status == Status::Pending);
    }

    #[test]
    fn cfg_test_code_is_separated_from_the_application_view() {
        let project = Project::new();
        project.write(
            "lib.rs",
            &format!(
                "{MIXED}\n#[cfg(not(test))]\nfn also_production() -> i32 {{ 2 }}\n\n#[tokio::test]\nasync fn checks_live() {{\n    assert_eq!(production(\"a\"), \"a\");\n}}\n"
            ),
        );
        let view = view_of(&project, &args());
        assert!(view.application && !view.tests);
        assert_eq!(view.classification.kind, "mixed");
        assert_eq!(view.classification.language, "Rust");
        let lines: Vec<_> = view
            .test_lines
            .iter()
            .map(|r| (r.start_line, r.end_line))
            .collect();
        assert_eq!(lines, [(5, 17), (22, 25)]);
        let mut options = args();
        options.include_tests = true;
        assert!(view_of(&project, &options).tests);
    }

    #[test]
    fn javascript_describe_and_test_blocks_are_separated_from_the_implementation() {
        let project = Project::new();
        project.write(
            "label.ts",
            "export function label(name: string) { return name.trim(); }\n\ndescribe(\"label\", () => {\n  it(\"trims\", () => { expect(label(\" a \")).toBe(\"a\"); });\n});\ntest(\"empty\", () => expect(label(\"\")).toBe(\"\"));\n",
        );
        let view = view_of(&project, &args());
        assert_eq!(view.classification.kind, "mixed");
        let lines: Vec<_> = view
            .test_lines
            .iter()
            .map(|r| (r.start_line, r.end_line))
            .collect();
        assert_eq!(lines, [(3, 6)]);
    }

    #[test]
    fn python_test_classes_and_pytest_functions_are_structural_tests() {
        let project = Project::new();
        let source = "import unittest\n\ndef total(rows):\n    return sum(rows)\n\nclass TotalChecks(unittest.TestCase):\n    def test_sum(self):\n        self.assertEqual(total([1, 2]), 3)\n\ndef test_empty():\n    assert total([]) == 0\n";
        project.write("lib/checks.py", source);
        let lines = |view: View| -> Vec<_> {
            view.test_lines
                .iter()
                .map(|r| (r.start_line, r.end_line))
                .collect()
        };
        // Outside a pytest file only the TestCase class is a test.
        assert_eq!(lines(view_of(&project, &args())), [(6, 8)]);
        // A pytest file with structural tests is a test file, without a purpose request.
        std::fs::remove_file(project.0.join("lib/checks.py")).unwrap();
        project.write("lib/test_checks.py", source);
        let mut included = args();
        included.include_tests = true;
        assert_eq!(view_of(&project, &included).classification.kind, "tests");
    }

    #[test]
    fn pure_test_files_are_judged_only_with_include_tests() {
        let project = Project::new();
        project.write(
            "tests/checks.rs",
            "#[test]\nfn checks_answer() {\n    assert_eq!(1, 1);\n}\n",
        );
        let options = args();
        let mut mock = crate::tests::Mock::default();
        let report = run(&project, &options, &mut mock);
        assert_eq!(mock.calls, 0);
        assert_eq!(report.files[0].status, Status::NotApplicable);
        assert!(
            report.files[0]
                .classification
                .as_ref()
                .unwrap()
                .reason
                .contains("--include-tests")
        );
        let mut options = args();
        options.include_tests = true;
        let mut mock = crate::tests::Mock::default();
        let report = run(&project, &options, &mut mock);
        assert_eq!(mock.calls, 1, "one test-value request");
        assert!(mock.requests[0]["state"]["tests"].is_array());
        assert_eq!(
            report.files[0].classification.as_ref().unwrap().kind,
            "tests"
        );
        assert_eq!(report.files[0].status, Status::Clear);
        assert!(!report.files[0].dimensions.contains_key("file_organization"));
    }

    const AMBIGUOUS: &str = "fn helper(value: &str) -> String {\n    let trimmed = value.trim();\n    let lower = trimmed.to_lowercase();\n    let joined = lower.replace(' ', \"-\");\n    let limited = joined.chars().take(8).collect::<String>();\n    limited\n}\n\n#[test]\nfn checks_helper() {\n    assert_eq!(helper(\" a \"), \"a\");\n}\n";

    #[test]
    fn test_paths_with_structural_tests_hold_test_support_without_a_purpose_request() {
        let project = Project::new();
        project.write("tests/flow.rs", AMBIGUOUS);
        let mut options = args();
        options.refresh = true;
        let mut eval = PurposeEval::new("mixed");
        let skipped = run(&project, &options, &mut eval);
        assert_eq!(eval.calls, 0);
        assert_eq!(skipped.files[0].status, Status::NotApplicable);
        assert_eq!(
            skipped.files[0].classification.as_ref().unwrap().kind,
            "tests"
        );

        options.include_tests = true;
        let mut eval = PurposeEval::new("mixed");
        let report = run(&project, &options, &mut eval);
        assert!(eval.functions.contains("fn helper"), "support is judged");
        assert!(report.files[0].dimensions.contains_key("test_value"));
        assert!(
            report.files[0]
                .classification
                .as_ref()
                .unwrap()
                .separated_tests
                .is_empty()
        );
    }

    const SUPPORT: &str = "fn helper(value: &str) -> String {\n    let trimmed = value.trim();\n    let lower = trimmed.to_lowercase();\n    let joined = lower.replace(' ', \"-\");\n    let limited = joined.chars().take(8).collect::<String>();\n    limited\n}\n\nfn fixture() -> String {\n    let value = helper(\" a \");\n    let again = helper(&value);\n    let joined = format!(\"{value}{again}\");\n    let trimmed = joined.trim().to_string();\n    trimmed\n}\n";

    #[test]
    fn test_paths_without_structural_tests_are_classified_before_the_rules() {
        let project = Project::new();
        project.write("tests/support.rs", SUPPORT);
        let mut options = args();
        options.refresh = true;
        let mut tests_only = PurposeEval::new("tests");
        let skipped = run(&project, &options, &mut tests_only);
        assert_eq!(tests_only.calls, 1);
        assert_eq!(skipped.files[0].status, Status::NotApplicable);
        assert_eq!(skipped.api_requests, 1);

        for mode in ["unresolved", "mixed"] {
            let mut eval = PurposeEval::new(mode);
            let judged = run(&project, &options, &mut eval);
            assert_eq!(eval.calls, 2, "{mode}: purpose and functions");
            assert!(eval.functions.contains("fn helper"), "{mode}");
            assert_eq!(judged.files[0].classification.as_ref().unwrap().kind, mode);
        }
    }

    struct PurposeEval {
        mode: &'static str,
        calls: usize,
        functions: String,
    }

    impl PurposeEval {
        fn new(mode: &'static str) -> Self {
            Self {
                mode,
                calls: 0,
                functions: String::new(),
            }
        }
    }

    impl crate::transport::Evaluator for PurposeEval {
        fn evaluate(&mut self, request: &Value) -> Result<Value> {
            self.calls += 1;
            if request["questions"]["file_purpose"].is_object() {
                let mut answers = serde_json::Map::new();
                let (tests, mixed, application) = match self.mode {
                    "tests" => (1.0, 0.0, 0.0),
                    "mixed" => (0.0, 1.0, 0.0),
                    _ => (0.34, 0.33, 0.33),
                };
                let choice = if tests >= mixed && tests >= application {
                    "tests"
                } else if mixed >= application {
                    "mixed"
                } else {
                    "application"
                };
                answers.insert(
                    "file_purpose".into(),
                    json!({"type":"choice","choice":choice,"confidence":0.9,"probabilities":{"tests":tests,"mixed":mixed,"application":application}}),
                );
                for (name, question) in request["questions"].as_object().unwrap() {
                    if question["type"] != "noul" {
                        continue;
                    }
                    let index = name
                        .rsplit_once('_')
                        .and_then(|(_, index)| index.parse::<usize>().ok())
                        .unwrap_or(0);
                    let unit_name = request["state"]["units"][index]["name"]
                        .as_str()
                        .unwrap_or("");
                    let noul = if unit_name.contains("helper") {
                        0.05
                    } else {
                        0.95
                    };
                    answers.insert(name.clone(), json!({"type":"noul","noul":noul}));
                }
                return Ok(
                    json!({"model":request["model"],"answers":answers,"usage":{"input_tokens":8,"output_tokens":2}}),
                );
            }
            if request["state"]["functions"].is_array() {
                self.functions = request["state"]["functions"].to_string();
            }
            Ok(crate::tests::answer(request, 0))
        }
    }
}
