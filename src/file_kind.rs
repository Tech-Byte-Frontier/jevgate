//! Deterministic file eligibility, then a test-versus-mix classification when a
//! test path still contains other code. Parsers locate structural tests; they
//! do not decide maintainability. The result is a gate view: which code the
//! application rules and the test rules judge.
use crate::line_ranges::{blank_lines, excerpt, merge_ranges, overlaps, ranges_phrase};
use crate::test_locations::locate_tests;
use crate::{
    inventory::Input,
    options::CheckArgs,
    policy,
    schema::{FileResult, SourceRange, Status},
    token_budget::TokenBudget,
};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::Path;

pub const VERSION: &str = "file-kind-v5";
const PORTION_PRESENT: f64 = policy::REVIEW_PROBABILITY;
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
    if input.result.role == crate::inventory::INSTRUCTIONS {
        return Ok(Plan::Ready(document(
            INSTRUCTIONS,
            "Agent instruction file. Documentation rules judge its sections.",
        )));
    }
    if input.result.role == crate::inventory::DOCS {
        return Ok(Plan::Ready(document(
            DOCS,
            "Project documentation. Documentation rules judge its sections.",
        )));
    }
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

/// A documentation file: only the documentation rules judge it.
fn document(kind: &str, reason: &str) -> View {
    View {
        classification: classification(kind, "deterministic", "documentation", reason, "Markdown"),
        application: false,
        tests: false,
        test_lines: Vec::new(),
    }
}

/// The classification kinds of an agent instruction file and project documentation.
pub(crate) const INSTRUCTIONS: &str = "instructions";
pub(crate) const DOCS: &str = "docs";

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
    let (answers, units, structural) = take_purpose(file)?;
    let choice = &answers["file_purpose"];
    let tests = probability(choice, "tests");
    let mixed = probability(choice, "mixed");
    let application = probability(choice, "application");
    let original = input.source.as_deref().unwrap_or("");
    let confident = |value| policy::probability_at_least(value, PORTION_PRESENT);
    if confident(tests) {
        return Ok(finish_tests(input, args, file));
    }
    if confident(mixed) {
        // Portion answers are speculative. Use them only on the mixed branch.
        let (portions, unresolved) = test_portions(&units, &answers)?;
        let separated = merge_ranges(structural.into_iter().chain(portions).collect());
        if !has_implementation(&input.result.path, &blank_lines(original, &separated)) {
            return Ok(finish_tests(input, args, file));
        }
        return Ok(Some(settle(
            input, args, file, "mixed", separated, unresolved, None,
        )));
    }
    if confident(application) && structural.is_empty() {
        let reason = "Classified as application or library code.".to_string();
        return Ok(Some(settle(
            input,
            args,
            file,
            "application",
            Vec::new(),
            Vec::new(),
            Some(reason),
        )));
    }
    if !has_implementation(&input.result.path, &blank_lines(original, &structural)) {
        return Ok(finish_tests(input, args, file));
    }
    if !structural.is_empty() && confident(application) {
        return Ok(Some(settle(
            input,
            args,
            file,
            "mixed",
            structural,
            Vec::new(),
            None,
        )));
    }
    let reason = if units.as_array().is_none_or(|items| items.is_empty())
        && mixed > application
        && mixed > tests
    {
        "The file looks mixed, but it has no separable test boundaries, so the rules judge the whole file."
    } else {
        "File purpose stayed below 0.80, so the rules judge the source without dropping unresolved regions."
    };
    Ok(Some(settle(
        input,
        args,
        file,
        "unresolved",
        structural,
        Vec::new(),
        Some(reason.into()),
    )))
}

/// The recorded purpose answers, the units they cover and the structural test
/// ranges; the file is marked as classified by the model.
fn take_purpose(file: &mut FileResult) -> Result<(Value, Value, Vec<SourceRange>)> {
    file.contains_tests = true;
    let class = file
        .classification
        .as_mut()
        .context("Missing file classification")?;
    let answers = class
        .purpose
        .clone()
        .context("Missing file-purpose answer")?;
    let units = std::mem::replace(&mut class.units, Value::Null);
    class.basis = "model".into();
    class.purpose = Some(answers["file_purpose"].clone());
    class.stage.clear();
    Ok((answers, units, class.separated_tests.clone()))
}

/// Units the purpose answer marks as tests, and units it leaves undecided.
fn test_portions(units: &Value, answers: &Value) -> Result<(Vec<SourceRange>, Vec<String>)> {
    let mut portions = Vec::new();
    let mut unresolved = Vec::new();
    for (index, unit) in units.as_array().into_iter().flatten().enumerate() {
        let noul = answers[&format!("test_portion_{index}")]["noul"]
            .as_f64()
            .context("Missing test-portion probability")?;
        if policy::probability_at_least(noul, PORTION_PRESENT) {
            portions.push(SourceRange {
                start_line: unit["start_line"].as_u64().unwrap_or(1) as usize,
                end_line: unit["end_line"].as_u64().unwrap_or(1) as usize,
            });
        } else if noul > PORTION_ABSENT {
            unresolved.push(unit["name"].as_str().unwrap_or("unit").to_string());
        }
    }
    Ok((portions, unresolved))
}

/// Record the classification the application rules will judge under.
fn settle(
    input: &Input,
    args: &CheckArgs,
    file: &mut FileResult,
    kind: &str,
    separated: Vec<SourceRange>,
    unresolved: Vec<String>,
    reason: Option<String>,
) -> View {
    let class = file.classification.as_mut().unwrap();
    class.kind = kind.into();
    class.gate = "application".into();
    class.reason = reason.unwrap_or_else(|| separated_reason(&separated));
    class.separated_tests = separated;
    class.unresolved_units = unresolved;
    view(input, args, class.clone())
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

/// A test file: judged by the test rules with `--include-tests`, otherwise skipped.
fn tests_prepared(path: &Path, args: &CheckArgs) -> Prepared {
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
}

fn prepare(input: &Input, args: &CheckArgs) -> Result<Prepared> {
    let original = input.source.clone().unwrap_or_default();
    let path = &input.result.path;
    let located = locate_tests(path, &original)?;
    if located.whole_file {
        return Ok(tests_prepared(path, args));
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
        return Ok(tests_prepared(path, args));
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

fn has_implementation(path: &Path, source: &str) -> bool {
    if !crate::context_units::review_targets(path, source).is_empty() {
        return true;
    }
    source.lines().any(code_line)
}

/// A line that is neither blank, a comment nor an import.
fn code_line(line: &str) -> bool {
    const NOT_CODE: &[&str] = &["//", "#", "/*", "*", "use ", "pub use ", "import ", "from "];
    let line = line.trim();
    !line.is_empty() && !NOT_CODE.iter().any(|start| line.starts_with(start))
}

fn separated_reason(ranges: &[SourceRange]) -> String {
    format!(
        "Tests were separated from the application code.{}",
        ranges_phrase(ranges)
    )
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
        options.rules = vec![crate::catalog::FUNCTION_SIMPLIFICATION.into()];
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

    impl PurposeEval {
        /// The file-purpose Choice for this mode.
        fn purpose(&self) -> Value {
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
            json!({"type":"choice","choice":choice,"confidence":0.9,"probabilities":{"tests":tests,"mixed":mixed,"application":application}})
        }
    }

    /// Units named like `helper` are application code; every other unit is a test.
    fn portion(request: &Value, name: &str) -> Value {
        let index = name
            .rsplit_once('_')
            .and_then(|(_, index)| index.parse::<usize>().ok())
            .unwrap_or(0);
        let unit = request["state"]["units"][index]["name"]
            .as_str()
            .unwrap_or("");
        let noul = if unit.contains("helper") { 0.05 } else { 0.95 };
        json!({"type":"noul","noul":noul})
    }

    impl crate::transport::Evaluator for PurposeEval {
        fn evaluate(&mut self, request: &Value) -> Result<Value> {
            self.calls += 1;
            if request["questions"]["file_purpose"].is_object() {
                let mut answers = serde_json::Map::new();
                answers.insert("file_purpose".into(), self.purpose());
                for (name, question) in request["questions"].as_object().unwrap() {
                    if question["type"] == "noul" {
                        answers.insert(name.clone(), portion(request, name));
                    }
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
