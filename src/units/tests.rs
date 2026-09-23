use super::*;
use crate::{
    catalog,
    inventory::Input,
    options::CheckArgs,
    schema::{Report, Status, Strength},
    tests::{Mock, Project, answer, args, function, run},
    token_budget::TokenBudget,
};
use anyhow::Result;
use serde_json::json;

fn planned(project: &Project, options: &CheckArgs) -> (Vec<Input>, Plan) {
    let inputs = crate::inventory::collect(options, &project.context(), &[]).unwrap();
    let budget = TokenBudget::default();
    let views = inputs
        .iter()
        .enumerate()
        .filter_map(
            |(i, input)| match crate::file_kind::plan(input, options, &budget) {
                Ok(crate::file_kind::Plan::Ready(view)) => Some((i, view)),
                _ => None,
            },
        )
        .collect();
    let plan = plan(&inputs, &views, options, &budget);
    (inputs, plan)
}

/// A `lib.rs` holding `count` judged functions `f0`, `f1`…
fn functions_project(count: usize) -> Project {
    let project = Project::new();
    let source: String = (0..count).map(|i| function(&format!("f{i}"))).collect();
    project.write("lib.rs", &source);
    project
}

fn numbers_in(value: &Value) -> bool {
    match value {
        Value::Number(_) => true,
        Value::Array(items) => items.iter().any(numbers_in),
        Value::Object(map) => map.values().any(numbers_in),
        _ => false,
    }
}

#[test]
fn requests_use_literal_paths_and_upload_no_numbers_hashes_or_local_metadata() {
    // Fourteen functions: two function packs, and enough lines for an outline.
    let project = functions_project(14);
    let options = args();
    let (inputs, plan) = planned(&project, &options);
    let functions: Vec<_> = plan
        .requests
        .iter()
        .filter(|p| p.request["jevgate"]["stage"] == "functions")
        .collect();
    assert_eq!(
        functions.len(),
        2,
        "fourteen functions pack into two requests"
    );
    assert_eq!(
        functions[0].request["state"]["functions"]
            .as_array()
            .unwrap()
            .len(),
        PACK_ITEMS
    );
    let budget = TokenBudget::default();
    for planned in &plan.requests {
        let request = &planned.request;
        assert!(budget.fits(request));
        let uploaded = crate::requests::provider_request(request);
        assert!(uploaded.get("jevgate").is_none());
        assert!(!numbers_in(&uploaded["state"]), "{}", uploaded["state"]);
        let text = uploaded.to_string();
        assert!(!text.contains(&inputs[0].result.source_hash));
        assert_eq!(
            request["jevgate"]["sources"][0]["source_hash"],
            inputs[0].result.source_hash
        );
        for (key, question) in uploaded["questions"].as_object().unwrap() {
            let text = question["instructions"]["question"].as_str().unwrap();
            if key.starts_with("f1_") {
                assert!(text.contains("`functions[1].source`"), "{text}");
            }
        }
        assert_eq!(
            planned.asked.questions.len(),
            uploaded["questions"].as_object().unwrap().len()
        );
    }
    let outline = plan
        .requests
        .iter()
        .find(|p| p.request["jevgate"]["stage"] == "outline")
        .unwrap();
    assert!(outline.request["state"]["members"][0]["signature"].is_string());
    assert!(
        !outline.request.to_string().contains("let mut total"),
        "no bodies"
    );
}

#[test]
fn small_functions_are_too_small_and_never_clear() {
    let project = Project::new();
    project.write("lib.rs", "fn one() -> i32 {\n    1\n}\n");
    let report = run(&project, &args(), &mut Mock::default());
    let dimension = &report.files[0].dimensions["function_simplification"];
    assert_eq!(dimension.units.too_small, 1);
    assert_eq!(dimension.status, Status::NotApplicable);
    assert_eq!(report.files[0].status, Status::NotApplicable);
}

/// Answers every question at `level`, except questions named in `overrides`.
struct Scripted {
    level: usize,
    overrides: Vec<(&'static str, Value)>,
    recheck_level: Option<usize>,
    stages: Vec<String>,
}

impl crate::transport::Evaluator for Scripted {
    fn evaluate(&mut self, request: &Value) -> Result<Value> {
        let recheck = is_recheck(request);
        self.stages
            .push(if recheck { "recheck" } else { "first" }.into());
        let level = if recheck {
            self.recheck_level.unwrap_or(self.level)
        } else {
            self.level
        };
        let mut body = answer(request, level);
        if !recheck {
            self.apply_overrides(&mut body);
        }
        Ok(body)
    }
}

impl Scripted {
    /// Replace every first-pass answer whose key ends with an override's suffix.
    fn apply_overrides(&self, body: &mut Value) {
        for (suffix, value) in &self.overrides {
            for (key, slot) in body["answers"].as_object_mut().unwrap() {
                if key.ends_with(suffix) {
                    *slot = value.clone();
                }
            }
        }
    }
}

/// Rechecks carry more evidence: callees, enclosing functions or file source.
fn is_recheck(request: &Value) -> bool {
    request["jevgate"]["stage"] == "recheck"
        || request["state"]["callees"].is_array()
        || request["state"]["callers"].is_array()
        || request["state"]["site_a"]["function_source"].is_string()
        || request["state"]["file"]["source"].is_string()
}

fn scripted(level: usize) -> Scripted {
    Scripted {
        level,
        overrides: Vec::new(),
        recheck_level: None,
        stages: Vec::new(),
    }
}

/// Run one rule with an undecided first pass and a recheck answered at `level`.
fn run_rechecked(project: &Project, rule: &str, level: usize) -> (CheckArgs, Report) {
    let mut options = args();
    only(&mut options, rule);
    let mut eval = scripted(3);
    eval.recheck_level = Some(level);
    let report = run(project, &options, &mut eval);
    (options, report)
}

fn only(options: &mut CheckArgs, rule: &str) {
    options.rules = vec![rule.into()];
}

/// A project whose `lib.rs` holds `source`, checked for one rule only.
fn rule_project(source: &str, rule: &str) -> (Project, CheckArgs) {
    let project = Project::new();
    project.write("lib.rs", source);
    let mut options = args();
    only(&mut options, rule);
    (project, options)
}

fn function_rule_project(source: &str) -> (Project, CheckArgs) {
    rule_project(source, catalog::FUNCTION_SIMPLIFICATION)
}

#[test]
fn a_review_function_carries_a_located_finding() {
    let (project, options) = function_rule_project(&function("busy"));
    let report = run(&project, &options, &mut scripted(2));
    let file = &report.files[0];
    assert_eq!(file.status, Status::Review);
    assert_eq!(file.findings.len(), 1);
    let finding = &file.findings[0];
    assert_eq!(finding.strength, Strength::Review);
    assert_eq!(finding.rule, "maintainability/function-simplification");
    assert_eq!(finding.locations[0].symbol.as_deref(), Some("busy"));
    assert!(finding.message.contains("mixes separate jobs"));
    assert!((finding.rank - 1.0 * (1.0 + 8.0f64).ln()).abs() < 1e-9);
    assert_eq!(crate::gate::exit_code(&report), 1);
}

const NESTED: &str = "fn nested(rows: &[Vec<i32>]) -> i32 {\n    let mut total = 0;\n    for row in rows {\n        if !row.is_empty() {\n            for value in row {\n                if *value > 0 {\n                    total += value;\n                }\n            }\n        }\n    }\n    total\n}\n";

#[test]
fn flatten_is_asked_only_for_deep_nesting_and_can_raise_a_finding_alone() {
    let (project, mut options) = function_rule_project(&format!("{NESTED}{}", function("flat")));
    let mut eval = scripted(0);
    eval.overrides.push(("flatten", spread(0.0, 0.05, 0.95)));
    let report = run(&project, &options, &mut eval);
    let file = &report.files[0];
    let flatten: Vec<_> = file
        .judgments
        .iter()
        .filter(|j| j.question == "flatten")
        .map(|j| j.unit.as_str())
        .collect();
    assert_eq!(flatten, ["function:nested"]);
    assert_eq!(file.findings.len(), 1);
    assert!(
        file.findings[0]
            .message
            .contains("nested or repeated branches")
    );
    options.refresh = true;
    let report = run(&project, &options, &mut scripted(1));
    assert_eq!(report.files[0].status, Status::Note);
    assert_eq!(report.files[0].findings[0].strength, Strength::Note);
    assert_eq!(crate::gate::exit_code(&report), 0);
}

#[test]
fn a_split_finding_is_located_at_the_chosen_block() {
    let (project, mut options) = function_rule_project(&function("busy"));
    let mut eval = scripted(2);
    eval.overrides.push((
        "block",
        json!({"type":"choice","choice":"B2","confidence":0.9,
            "probabilities":{"B1":0.1,"B2":0.9,"none":0.0}}),
    ));
    let report = run(&project, &options, &mut eval);
    assert_eq!(report.stages["locate"].successful_requests, 1);
    let finding = &report.files[0].findings[0];
    assert_eq!(
        (finding.line, finding.locations[0].end_line),
        (6, 7),
        "the second block, then the function"
    );
    assert_eq!(finding.locations[1].start_line, 1);
    assert!(finding.message.contains("Lines 6–7"), "{}", finding.message);
    assert!(
        (finding.rank - (1.0 + 8.0f64).ln()).abs() < 1e-9,
        "rank ignores the block"
    );
    options.refresh = true;
    let report = run(&project, &options, &mut scripted(1));
    assert!(
        !report.stages.contains_key("locate"),
        "a note is not located"
    );
}

#[test]
fn uncertain_units_get_one_recheck_that_replaces_them_only_when_decisive() {
    let project = Project::new();
    project.write(
        "lib.rs",
        &format!(
            "{}{}",
            function("helper"),
            function("caller").replace(
                "let doubled = total * 2;",
                "let doubled = helper(&[total]);"
            )
        ),
    );
    let (mut options, report) = run_rechecked(&project, catalog::FUNCTION_SIMPLIFICATION, 2);
    assert_eq!(
        report.stages["recheck"].successful_requests, 1,
        "only the caller has callees"
    );
    let file = &report.files[0];
    let dimension = &file.dimensions["function_simplification"];
    assert_eq!((dimension.units.review, dimension.units.uncertain), (1, 1));
    let reviewed: Vec<_> = file.findings.iter().map(|f| f.symbol.as_deref()).collect();
    assert_eq!(
        reviewed,
        [Some("caller")],
        "the decisive recheck replaced it"
    );
    options.refresh = true;
    let mut still = scripted(3);
    let report = run(&project, &options, &mut still);
    assert_eq!(report.files[0].status, Status::Uncertain);
    assert!(report.files[0].findings.is_empty());
}

fn two_concerns() -> String {
    let mut source = String::from("struct Cache { entries: Vec<u8> }\n");
    for i in 0..7 {
        source.push_str(&function(&format!("warm{i}")));
    }
    source.push_str("struct Page { body: String }\n");
    for i in 0..7 {
        source.push_str(&function(&format!("render{i}")));
    }
    source
}

#[test]
fn file_organization_review_without_a_module_choice_is_a_file_wide_finding() {
    let project = Project::new();
    project.write("lib.rs", &two_concerns());
    let mut options = args();
    only(&mut options, catalog::FILE_ORGANIZATION);
    let report = run(&project, &options, &mut scripted(2));
    let file = &report.files[0];
    assert_eq!(file.status, Status::Review);
    let finding = &file.findings[0];
    assert!(finding.message.contains("unrelated responsibilities"));
    assert_eq!(finding.locations[0].start_line, 1);
    assert!(finding.symbol.is_none());
}

#[test]
fn an_uncertain_outline_is_rechecked_once_with_the_application_source() {
    let project = Project::new();
    let source = format!(
        "{}#[cfg(test)]\nmod tests {{\n    #[test]\n    fn hidden_check() {{\n        assert_eq!(1, 1);\n    }}\n}}\n",
        two_concerns()
    );
    project.write("lib.rs", &source);
    let (options, report) = run_rechecked(&project, catalog::FILE_ORGANIZATION, 0);
    assert_eq!(report.stages["outline"].successful_requests, 1);
    assert_eq!(report.stages["recheck"].successful_requests, 1);
    assert_eq!(
        report.files[0].dimensions["file_organization"].status,
        Status::Clear
    );
    let (_, plan) = planned(&project, &options);
    let (request, _) = plan.files.values().next().unwrap().units[0]
        .recheck
        .as_ref()
        .unwrap();
    let sent = request["state"]["file"]["source"].as_str().unwrap();
    assert!(sent.contains("fn warm0") && !sent.contains("hidden_check"));
}

#[test]
fn short_files_are_too_small_to_split_and_never_clear() {
    let project = Project::new();
    project.write(
        "lib.rs",
        &format!("{}{}", function("warm"), function("render")),
    );
    let mut options = args();
    only(&mut options, catalog::FILE_ORGANIZATION);
    let mut mock = Mock::default();
    let report = run(&project, &options, &mut mock);
    let dimension = &report.files[0].dimensions["file_organization"];
    assert_eq!((dimension.units.too_small, mock.calls), (1, 0));
    assert_eq!(dimension.status, Status::NotApplicable);
}

const LOAD: &str = "fn load_user(path: &str) -> Result<User> {\n    let text = std::fs::read_to_string(path)?;\n    let value: Value = serde_json::from_str(&text)?;\n    let name = value[\"name\"].as_str().unwrap_or(\"anonymous\").trim().to_string();\n    Ok(User { name })\n}\n";

#[test]
fn copies_inside_one_test_raise_at_most_a_consider() {
    let project = Project::new();
    let block = "    let text = std::fs::read_to_string(path).unwrap();\n    let value: Value = serde_json::from_str(&text).unwrap();\n    let name = value[\"name\"].as_str().unwrap_or(\"anonymous\").trim().to_string();\n    assert_eq!(name, expected);\n";
    let second = block.replace("text", "body").replace("value", "parsed");
    project.write(
        "tests/cases.rs",
        &format!("#[test]\nfn reads_names() {{\n    let path = \"a.json\";\n    let expected = \"a\";\n{block}    let path = \"b.json\";\n{second}}}\n"),
    );
    let mut options = args();
    options.include_tests = true;
    only(&mut options, catalog::SHARED_LOGIC);
    let mut same = scripted(2);
    same.overrides
        .push(("required", json!({"type":"noul","noul":0.05})));
    let report = run(&project, &options, &mut same);
    let finding = &report.files[0].findings[0];
    assert_eq!(finding.strength, Strength::Consider);
    assert!(
        finding.message.contains("inside one test"),
        "{}",
        finding.message
    );
    options.refresh = true;
    let mut related = scripted(1);
    related
        .overrides
        .push(("required", json!({"type":"noul","noul":0.05})));
    let report = run(&project, &options, &mut related);
    assert_eq!(report.files[0].findings[0].strength, Strength::Note);
}

#[test]
fn copies_across_test_cases_are_one_level_lower_than_copies_in_support_code() {
    let block = "    let text = std::fs::read_to_string(path).unwrap();\n    let value: Value = serde_json::from_str(&text).unwrap();\n    let name = value[\"name\"].as_str().unwrap_or(\"anonymous\").trim().to_string();\n";
    let second = block.replace("text", "body").replace("value", "parsed");
    let cases = format!(
        "#[test]\nfn reads_a() {{\n    let path = \"a.json\";\n{block}    assert_eq!(name, \"a\");\n}}\n\n#[test]\nfn reads_b() {{\n    let path = \"b.json\";\n{second}    assert_eq!(name, \"b\");\n}}\n"
    );
    let support = format!(
        "#[test]\nfn loads() {{\n    assert_eq!(load_a(\"a.json\"), load_b(\"b.json\"));\n}}\n\nfn load_a(path: &str) -> String {{\n{block}    name\n}}\n\nfn load_b(path: &str) -> String {{\n{second}    name\n}}\n"
    );
    let mut options = args();
    options.include_tests = true;
    only(&mut options, catalog::SHARED_LOGIC);
    let strength = |source: &str| {
        let project = Project::new();
        project.write("tests/cases.rs", source);
        let mut same = scripted(2);
        same.overrides
            .push(("required", json!({"type":"noul","noul":0.05})));
        let report = run(&project, &options, &mut same);
        let finding = report.files[0].findings[0].clone();
        (finding.strength, finding.message)
    };
    let (in_cases, message) = strength(&cases);
    assert_eq!(in_cases, Strength::Consider);
    assert!(message.contains("across test cases"), "{message}");
    assert_eq!(strength(&support).0, Strength::Review);
}

fn group(id: &str, users: &[&str]) -> GroupInfo {
    GroupInfo {
        id: id.into(),
        names: vec![format!("{id}_member")],
        locations: Vec::new(),
        users: users.iter().map(PathBuf::from).collect(),
    }
}

#[test]
fn a_split_needs_a_group_with_users_of_its_own_when_users_are_known() {
    let chosen = |id: &str| crate::schema::Answer::Choice {
        choice: id.into(),
        confidence: 1.0,
        probabilities: BTreeMap::from([(id.to_string(), 1.0)]),
    };
    let unknown = [group("G1", &[]), group("G2", &[])];
    assert!(
        outcome::split_has_users(&unknown, None),
        "missing evidence stands"
    );
    let shared = [group("G1", &["a.rs"]), group("G2", &["a.rs"])];
    assert!(!outcome::split_has_users(&shared, None));
    let own = [group("G1", &["a.rs", "b.rs"]), group("G2", &["a.rs"])];
    assert!(outcome::split_has_users(&own, None));
    assert!(outcome::split_has_users(&own, Some(&chosen("G1"))));
    assert!(!outcome::split_has_users(&own, Some(&chosen("G2"))));
}

#[test]
fn duplicate_pairs_across_files_quote_both_sites_and_respect_required_repetition() {
    let project = Project::new();
    project.write("a.rs", LOAD);
    project.write(
        "b.rs",
        &LOAD
            .replace("load_user", "load_team")
            .replace("\"name\"", "\"title\""),
    );
    let mut options = args();
    only(&mut options, catalog::SHARED_LOGIC);
    let mut same = scripted(2);
    same.overrides
        .push(("required", json!({"type":"noul","noul":0.05})));
    let report = run(&project, &options, &mut same);
    assert_eq!(report.stages["duplicate-pair"].successful_requests, 1);
    let a = &report.files[0];
    let finding = &a.findings[0];
    assert_eq!(finding.rule, "maintainability/shared-logic");
    assert_eq!(finding.locations.len(), 2);
    assert_eq!(finding.locations[1].path, std::path::Path::new("b.rs"));
    assert!(finding.quote.as_ref().unwrap().starts_with("let text"));
    assert!(
        finding.message.contains("`\"name\"`→`\"title\"`")
            || finding.message.contains("`name`→`title`"),
        "{}",
        finding.message
    );
    assert!(
        report.files[1].findings.is_empty(),
        "the pair is reported once"
    );
    assert_eq!(
        report.files[1].dimensions["shared_logic"].status,
        Status::NotApplicable
    );
    options.refresh = true;
    let mut required = scripted(2);
    required
        .overrides
        .push(("required", json!({"type":"noul","noul":0.9})));
    let report = run(&project, &options, &mut required);
    assert_eq!(
        report.files[0].dimensions["shared_logic"].status,
        Status::Clear
    );
}

const TESTS: &str = "fn total(values: &[i32]) -> i32 {\n    values.iter().sum()\n}\n\n#[cfg(test)]\nmod tests {\n    use super::*;\n\n    #[test]\n    fn adds_two() {\n        let values = vec![1, 2];\n        assert_eq!(total(&values), 3);\n    }\n\n    #[test]\n    fn adds_three() {\n        let values = vec![1, 2, 3];\n        assert_eq!(total(&values), 6);\n    }\n\n    #[test]\n    fn adds_four() {\n        let values = vec![1, 2, 3, 4];\n        assert_eq!(total(&values), 10);\n    }\n}\n";

#[test]
fn test_rules_need_include_tests_and_summarize_over_tested_subjects() {
    let project = Project::new();
    project.write("lib.rs", TESTS);
    let options = args();
    let (_, plan) = planned(&project, &options);
    assert!(!plan.files[&0].rules.contains_key(catalog::TEST_VALUE));
    let mut options = args();
    options.include_tests = true;
    options.rules = vec![catalog::TEST_VALUE.into(), catalog::TEST_REDUNDANCY.into()];
    let report = run(&project, &options, &mut scripted(1));
    let file = &report.files[0];
    let values = &file.dimensions["test_value"];
    assert_eq!(values.units.judged, 3);
    assert_eq!(
        values.status,
        Status::Uncertain,
        "Noul answers at 0.5 stay uncertain"
    );
    let redundancy = &file.dimensions["test_redundancy"];
    assert_eq!(redundancy.units.judged, 3);
    assert_eq!(redundancy.status, Status::Consider);
    let over = file
        .findings
        .iter()
        .find(|f| f.message.contains("3 tests of `total` overlap"))
        .unwrap();
    assert_eq!(over.strength, Strength::Consider);
    assert_eq!(over.locations.len(), 3);
}

#[test]
fn composition_is_pure_and_repeatable_from_saved_judgments() {
    let project = Project::new();
    project.write("lib.rs", &function("busy"));
    let options = args();
    let report: Report = run(&project, &options, &mut scripted(2));
    let (_, plan) = planned(&project, &options);
    let again = compose::compose(&plan.files[&0], &report.files[0].judgments);
    assert_eq!(again.status, report.files[0].status);
    assert_eq!(
        again
            .findings
            .iter()
            .map(|f| &f.fingerprint)
            .collect::<Vec<_>>(),
        report.files[0]
            .findings
            .iter()
            .map(|f| &f.fingerprint)
            .collect::<Vec<_>>()
    );
}

#[test]
fn packing_and_cache_identity_do_not_depend_on_token_calibration() {
    let project = functions_project(12);
    let options = args();
    let inputs = crate::inventory::collect(&options, &project.context(), &[]).unwrap();
    let keys = |bytes_per_token: f64| {
        let budget = TokenBudget { bytes_per_token };
        let views = BTreeMap::from([(
            0,
            match crate::file_kind::plan(&inputs[0], &options, &budget).unwrap() {
                crate::file_kind::Plan::Ready(view) => view,
                _ => unreachable!(),
            },
        )]);
        plan(&inputs, &views, &options, &budget)
            .requests
            .iter()
            .map(|p| crate::requests::judgment_key(&p.request))
            .collect::<Vec<_>>()
    };
    assert_eq!(keys(2.0), keys(6.0));
}

fn spread(p0: f64, p1: f64, p2: f64) -> Value {
    json!({"type":"score","score":p1 + 2.0 * p2,"confidence":0.3,
        "probabilities":{"0":p0,"1":p1,"2":p2}})
}

#[test]
fn a_function_clears_when_the_split_level_is_ruled_out() {
    let project = Project::new();
    project.write("lib.rs", &function("borderline"));
    let mut options = args();
    options.refresh = true;
    only(&mut options, catalog::FUNCTION_SIMPLIFICATION);
    let status = |split: Value| {
        let mut eval = scripted(0);
        eval.overrides.push(("split", split));
        run(&project, &options, &mut eval).files[0].status.clone()
    };
    assert_eq!(status(spread(0.5, 0.35, 0.15)), Status::Clear);
    assert_eq!(status(spread(0.1, 0.3, 0.6)), Status::Consider);
    assert_eq!(
        status(spread(0.1, 0.5, 0.4)),
        Status::Note,
        "the middle level says the function reads well as it is"
    );
    assert_eq!(status(spread(0.35, 0.05, 0.6)), Status::Uncertain);
}

#[test]
fn undecided_weak_test_signals_do_not_block_a_clear_test() {
    let project = Project::new();
    project.write("lib.rs", TESTS);
    let mut options = args();
    options.include_tests = true;
    options.rules = vec![catalog::TEST_VALUE.into()];
    let mut eval = scripted(0);
    eval.overrides
        .push(("internal", json!({"type":"noul","noul":0.45})));
    eval.overrides
        .push(("several", json!({"type":"noul","noul":0.4})));
    let report = run(&project, &options, &mut eval);
    assert_eq!(
        report.files[0].dimensions["test_value"].status,
        Status::Clear
    );
    options.refresh = true;
    let mut hollow = scripted(0);
    hollow
        .overrides
        .push(("own_logic", json!({"type":"noul","noul":0.5})));
    let report = run(&project, &options, &mut hollow);
    assert_eq!(
        report.files[0].dimensions["test_value"].status,
        Status::Uncertain
    );
}

const HARDCODED: &str = "const REGION: &str = \"eu-west-1\";\n\nfn connect() -> Client {\n    Client::new(\"db.internal:5432\", 30_000)\n}\n\nfn total(values: &[i32]) -> i32 {\n    values.iter().sum()\n}\n";

fn hardcoded_project() -> (Project, CheckArgs) {
    rule_project(HARDCODED, catalog::HARDCODED_VALUES)
}

#[test]
fn functions_with_literals_and_module_constants_are_hardcoded_value_units() {
    let (project, options) = hardcoded_project();
    let (_, plan) = planned(&project, &options);
    let stages: Vec<_> = plan
        .requests
        .iter()
        .map(|p| p.request["jevgate"]["stage"].as_str().unwrap())
        .collect();
    assert_eq!(stages, ["values", "constants"]);
    let values = &plan.requests[0].request["state"]["functions"];
    assert_eq!(
        values.as_array().unwrap().len(),
        1,
        "`total` has no literal"
    );
    assert_eq!(
        values[0]["values"],
        json!(["\"db.internal:5432\"", "30_000"])
    );
    let questions = plan.requests[0].request["questions"].as_object().unwrap();
    assert_eq!(questions.len(), 3);
    assert_eq!(
        plan.requests[1].request["state"]["constants"][0]["value"],
        "\"eu-west-1\""
    );
}

#[test]
fn a_local_default_is_a_note_and_a_special_case_is_a_review() {
    let (project, mut options) = hardcoded_project();
    let run_with = |options: &CheckArgs, overrides: Vec<(&'static str, Value)>| {
        let mut eval = scripted(0);
        eval.overrides = overrides;
        run(&project, options, &mut eval)
    };
    let report = run_with(&options, vec![("environment", spread(0.1, 0.8, 0.1))]);
    let strengths: Vec<_> = report.files[0]
        .findings
        .iter()
        .map(|f| f.strength)
        .collect();
    assert_eq!(strengths, [Strength::Note, Strength::Note]);
    options.refresh = true;
    let report = run_with(
        &options,
        vec![("special", json!({"type":"noul","noul":0.9}))],
    );
    let finding = &report.files[0].findings[0];
    assert_eq!(finding.strength, Strength::Review);
    assert_eq!(finding.rule, "maintainability/hardcoded-values");
    assert!(
        finding
            .message
            .contains("special-cases one specific identity")
    );
    assert!(finding.action.contains("data or configuration"));
}

#[test]
fn undecided_units_are_listed_with_the_questions_left_undecided() {
    let (project, mut options) = function_rule_project(&function("borderline"));
    let report = run(&project, &options, &mut scripted(3));
    let dimension = &report.files[0].dimensions["function_simplification"];
    assert_eq!(dimension.status, Status::Uncertain);
    assert_eq!(
        dimension.undecided,
        [crate::schema::Undecided {
            unit: "borderline".into(),
            line: 1,
            questions: vec!["splitting".into()],
            values: Vec::new(),
        }]
    );
    options.refresh = true;
    let report = run(&project, &options, &mut scripted(0));
    assert!(
        report.files[0].dimensions["function_simplification"]
            .undecided
            .is_empty()
    );
}

#[test]
fn undecided_hardcoded_units_name_their_few_candidate_values() {
    let (project, options) = hardcoded_project();
    // Split answers that lean away from the concern, and follow-ups that do
    // not clear them: they stay undecided.
    let mut eval = scripted(0);
    eval.overrides = vec![
        ("environment", spread(0.55, 0.05, 0.4)),
        ("magic", spread(0.55, 0.05, 0.4)),
        ("special", noul_at(0.3)),
    ];
    let report = run(&project, &options, &mut eval);
    let undecided = &report.files[0].dimensions["hardcoded_values"].undecided;
    let connect = undecided.iter().find(|u| u.unit == "connect").unwrap();
    assert_eq!(connect.values, ["\"db.internal:5432\"", "30_000"]);
    assert_eq!(connect.questions.len(), 3);
    let constants = undecided
        .iter()
        .find(|u| u.unit == "module constants")
        .unwrap();
    assert_eq!(constants.values, ["\"eu-west-1\""]);
}

const QUERY: &str = "fn find(conn: &Connection, name: &str) -> Result<Row> {\n    let sql = format!(\"SELECT id FROM users WHERE name = '{name}'\");\n    conn.query_row(&sql, [], Row::from)\n}\n\nfn total(a: i32, b: i32) -> i32 {\n    a + b\n}\n";

fn security_project(source: &str) -> (Project, CheckArgs) {
    let project = Project::new();
    project.write("lib.rs", source);
    let mut options = args();
    options.rules = catalog::SECURITY.iter().map(|r| r.to_string()).collect();
    (project, options)
}

fn noul_at(p: f64) -> Value {
    json!({"type":"noul","noul":p})
}

/// A certain choice of `id` among the two sites of `find` in `QUERY`.
fn site(id: &str) -> Value {
    let probabilities: serde_json::Map<String, Value> = ["S1", "S2", "none"]
        .iter()
        .map(|option| {
            (
                option.to_string(),
                json!(if *option == id { 1.0 } else { 0.0 }),
            )
        })
        .collect();
    json!({"type":"choice","choice":id,"confidence":1.0,"probabilities":probabilities})
}

#[test]
fn only_functions_with_calls_built_text_or_field_assignments_are_sent_for_security() {
    let (project, options) = security_project(QUERY);
    let (_, plan) = planned(&project, &options);
    let sent: Vec<&str> = plan
        .requests
        .iter()
        .flat_map(|p| p.request["state"]["functions"].as_array().unwrap())
        .filter_map(|f| f["name"].as_str())
        .collect();
    assert_eq!(
        sent,
        ["find"],
        "`total` has no call, built text or field assignment"
    );
}

#[test]
fn clear_presence_needs_no_trace_and_clears_every_security_rule() {
    let (project, options) = security_project(QUERY);
    let mut eval = scripted(0);
    let report = run(&project, &options, &mut eval);
    assert_eq!(eval.stages, ["first"]);
    for rule in catalog::SECURITY {
        assert_eq!(report.files[0].dimensions[rule].status, Status::Clear);
    }
}

#[test]
fn an_unhandled_value_from_another_party_is_a_located_injection_review() {
    let (project, options) = security_project(QUERY);
    let mut eval = scripted(0);
    eval.overrides = vec![
        ("interpreted", noul_at(0.95)),
        ("sql", noul_at(0.95)),
        ("origin", spread(0.0, 0.1, 0.9)),
        ("site", site("S1")),
    ];
    let report = run(&project, &options, &mut eval);
    assert_eq!(eval.stages, ["first", "first"], "one trace, no recheck");
    let finding = &report.files[0].findings[0];
    assert_eq!(finding.rule, "security/injection");
    assert_eq!(finding.strength, Strength::Review);
    assert_eq!(finding.category.as_deref(), Some("CWE-89 SQL injection"));
    assert_eq!(finding.line, 2, "located at the chosen site");
    assert!(finding.action.contains("bound query parameters"));
}

#[test]
fn a_parameter_origin_is_a_consider_that_callers_can_settle() {
    let caller = format!(
        "{QUERY}\nfn handler(conn: &Connection) -> Result<Row> {{\n    find(conn, \"admin\")\n}}\n"
    );
    let overrides = || {
        vec![
            ("interpreted", noul_at(0.95)),
            ("sql", noul_at(0.95)),
            ("origin", spread(0.0, 0.9, 0.1)),
        ]
    };
    let (project, options) = security_project(QUERY);
    let mut eval = scripted(0);
    eval.overrides = overrides();
    let report = run(&project, &options, &mut eval);
    assert_eq!(
        report.files[0].dimensions[catalog::INJECTION].status,
        Status::Consider,
        "no caller is known, so the parameter stays a concern"
    );
    let (project, options) = security_project(&caller);
    let mut eval = scripted(0);
    eval.overrides = overrides();
    eval.recheck_level = Some(0);
    let report = run(&project, &options, &mut eval);
    assert_eq!(eval.stages.last().unwrap(), "recheck");
    let find = |report: &Report| {
        report.files[0]
            .findings
            .iter()
            .any(|f| f.rule == "security/injection" && f.symbol.as_deref() == Some("find"))
    };
    assert!(!find(&report), "the caller passes a fixed value");
}

#[test]
fn a_check_left_undecided_is_decided_again_with_callers() {
    let caller = format!(
        "{QUERY}\nfn handler(conn: &Connection, request: &Request) -> Result<Row> {{\n    find(conn, &request.query[\"name\"])\n}}\n"
    );
    let (project, options) = security_project(&caller);
    let mut eval = scripted(0);
    eval.overrides = vec![
        ("resource", noul_at(0.95)),
        ("path", noul_at(0.5)),
        ("origin", spread(0.0, 0.9, 0.1)),
    ];
    eval.recheck_level = Some(2);
    let report = run(&project, &options, &mut eval);
    let finding = report.files[0]
        .findings
        .iter()
        .find(|f| f.symbol.as_deref() == Some("find") && f.rule == "security/injection")
        .expect("the recheck decides the path check and the origin");
    assert_eq!(
        finding.strength,
        Strength::Review,
        "undecided without the recheck"
    );
    assert!(finding.category.is_some());
}

#[test]
fn a_parameter_in_a_path_or_url_is_a_note_until_callers_show_another_party() {
    let (project, options) = security_project(QUERY);
    let mut eval = scripted(0);
    eval.overrides = vec![
        ("resource", noul_at(0.95)),
        ("url", noul_at(0.95)),
        ("origin", spread(0.0, 0.9, 0.1)),
    ];
    let report = run(&project, &options, &mut eval);
    let finding = &report.files[0].findings[0];
    assert_eq!(finding.strength, Strength::Note);
    assert_eq!(
        finding.category.as_deref(),
        Some("CWE-918 server-side request forgery")
    );
}

#[test]
fn checks_that_all_clear_rule_out_an_uncertain_presence() {
    let (project, options) = security_project(QUERY);
    let mut eval = scripted(0);
    eval.overrides = vec![
        ("interpreted", noul_at(0.5)),
        ("origin", spread(0.0, 1.0, 0.0)),
    ];
    let report = run(&project, &options, &mut eval);
    assert_eq!(
        report.files[0].dimensions[catalog::INJECTION].status,
        Status::Clear
    );
}

#[test]
fn development_only_exposure_is_one_level_lower_and_names_its_weakness() {
    let (project, options) = security_project(QUERY);
    let mut eval = scripted(0);
    eval.overrides = vec![("logs_secret", noul_at(0.95)), ("dev_only", noul_at(0.95))];
    let report = run(&project, &options, &mut eval);
    let finding = &report.files[0].findings[0];
    assert_eq!(finding.rule, "security/sensitive-data");
    assert_eq!(finding.strength, Strength::Consider);
    assert_eq!(
        finding.category.as_deref(),
        Some("CWE-532 sensitive data in logs")
    );
    assert!(finding.message.contains("runs only in development"));
}

#[test]
fn top_level_setup_is_one_unit_for_unsafe_settings() {
    let project = Project::new();
    project.write(
        "server.ts",
        "const app = express()\napp.use(cors({ origin: true, credentials: true }))\n",
    );
    let mut options = args();
    options.rules = vec![catalog::UNSAFE_SETTINGS.into()];
    let (_, plan) = planned(&project, &options);
    let request = &plan.requests[0].request;
    assert!(
        request["state"]["module"]["source"]
            .as_str()
            .unwrap()
            .contains("app.use(cors(")
    );
    let mut eval = scripted(0);
    eval.overrides = vec![("weakened", noul_at(0.95)), ("cors", noul_at(0.95))];
    let report = run(&project, &options, &mut eval);
    let finding = &report.files[0].findings[0];
    assert_eq!(finding.category.as_deref(), Some("CWE-942 permissive CORS"));
    assert!(finding.message.starts_with("Module setup"));
}

#[test]
fn an_undecided_value_is_cleared_by_its_kind_or_leans_into_a_note() {
    let (project, mut options) = hardcoded_project();
    let split = || {
        vec![
            ("environment", spread(0.4, 0.1, 0.5)),
            ("magic", spread(0.6, 0.1, 0.3)),
        ]
    };
    let mut eval = scripted(0);
    eval.overrides = split();
    let report = run(&project, &options, &mut eval);
    assert!(eval.stages.contains(&"recheck".to_string()));
    let dimension = &report.files[0].dimensions["hardcoded_values"];
    assert_eq!(
        (dimension.units.note, dimension.units.uncertain),
        (2, 0),
        "not cleared by the kind checks, and leaning toward the concern"
    );
    let leaned = report.files[0]
        .findings
        .iter()
        .find(|f| f.symbol.as_deref() == Some("connect"))
        .expect("the environment answer leaned toward its concern");
    assert_eq!(leaned.strength, Strength::Note);
    assert!(
        leaned.message.contains("the answer was split"),
        "{}",
        leaned.message
    );
    options.refresh = true;
    let mut eval = scripted(0);
    eval.overrides = split();
    eval.recheck_level = Some(2);
    let report = run(&project, &options, &mut eval);
    let dimension = &report.files[0].dimensions["hardcoded_values"];
    assert_eq!(
        dimension.status,
        Status::Clear,
        "every value is of an acceptable kind"
    );
}

#[test]
fn error_details_clear_on_the_programs_own_messages_or_lean_into_a_note() {
    let (project, mut options) = security_project(QUERY);
    let status = |report: &Report| {
        report.files[0].dimensions[catalog::SENSITIVE_DATA]
            .status
            .clone()
    };
    let mut eval = scripted(0);
    eval.overrides = vec![
        ("error_details", noul_at(0.3)),
        ("exception_to_client", noul_at(0.3)),
        ("own_messages", noul_at(0.95)),
    ];
    assert_eq!(status(&run(&project, &options, &mut eval)), Status::Clear);
    options.refresh = true;
    let mut eval = scripted(0);
    eval.overrides = vec![
        ("error_details", noul_at(0.6)),
        ("exception_to_client", noul_at(0.3)),
    ];
    let report = run(&project, &options, &mut eval);
    let note = &report.files[0].findings[0];
    assert_eq!(note.strength, Strength::Note);
    assert!(
        note.message.contains("may send internal error details"),
        "{}",
        note.message
    );
    assert_eq!(
        note.category.as_deref(),
        Some("CWE-209 error details exposed")
    );
}

#[test]
fn instruction_sections_are_at_most_consider_and_name_their_harnesses() {
    let project = Project::new();
    project.write("Cargo.toml", "[package]\nname = \"demo\"\n");
    project.write("src/lib.rs", "");
    project.write("web/app.ts", "");
    project.write(
        "AGENTS.md",
        "# Stack\nThis is a Rust project.\n\n# Web\nUse the design tokens in `web/theme.ts`.\n\n# Release\nTag with `v` then push.\n",
    );
    let mut options = args();
    only(&mut options, catalog::AGENT_CONTEXT);
    let mut eval = scripted(0);
    let top = json!({"type":"score","score":2.0,"confidence":1.0,
        "probabilities":{"0":0.0,"1":0.0,"2":1.0}});
    let web = json!({"type":"choice","choice":"web/","confidence":1.0,
        "probabilities":{"src/":0.0,"web/":1.0,"none":0.0}});
    eval.overrides = vec![("s0_inferable", top), ("s1_scope", web)];
    let report = run(&project, &options, &mut eval);
    let file = report
        .files
        .iter()
        .find(|f| f.path == std::path::Path::new("AGENTS.md"))
        .unwrap();
    let dimension = &file.dimensions[catalog::AGENT_CONTEXT];
    assert_eq!(
        (
            dimension.units.judged,
            dimension.units.consider,
            dimension.units.note
        ),
        (3, 1, 1)
    );
    let stack = &file.findings[0];
    assert_eq!(stack.strength, Strength::Consider, "capped below review");
    assert_eq!(
        (stack.rule.as_str(), stack.line),
        ("documentation/agent-context", 1)
    );
    assert!(
        stack.message.starts_with("Section `Stack` restates what the repository's files show (1.00). Codex, GitHub Copilot, Cursor, Windsurf, Cline and Claude Code load it at the start of every session"),
        "{}",
        stack.message
    );
    assert_eq!(file.findings[1].symbol.as_deref(), Some("Web"));
    assert!(
        file.findings[1]
            .message
            .contains("applies only to work in `web/`"),
        "{}",
        file.findings[1].message
    );
    assert!(file.findings[1].action.starts_with("Optional: move it"));
    let load = report.context_load.as_ref().unwrap();
    assert!(load.harnesses.iter().any(|h| h.harness == "Codex"));
}

fn git(project: &Project, args: &[&str]) {
    let status = std::process::Command::new("git")
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .current_dir(&*project.0)
        .output()
        .unwrap();
    assert!(status.status.success(), "{status:?}");
}

#[test]
fn stale_sections_and_repeated_sections_are_checked_after_the_first_pass() {
    let project = Project::new();
    let shared = "Install the dependencies, start the local database, copy the example environment file and run the development server before opening a pull request";
    project.write(
        "README.md",
        &format!("# Setup\n{shared}.\nRun `scripts/setup.sh` first.\n"),
    );
    project.write("docs/guide.md", &format!("# Getting started\n{shared}.\n"));
    let mut options = args();
    options.rules = vec![
        catalog::DOC_STALENESS.into(),
        catalog::DOC_DUPLICATION.into(),
    ];
    let mut eval = scripted(2);
    let report = run(&project, &options, &mut eval);
    let readme = report
        .files
        .iter()
        .find(|f| f.path == std::path::Path::new("README.md"))
        .unwrap();
    let messages: Vec<&str> = readme.findings.iter().map(|f| f.message.as_str()).collect();
    assert!(
        messages.iter().any(|m| m.contains(
            "tells the reader to use `scripts/setup.sh`, which is not in the repository"
        )),
        "{messages:?}"
    );
    assert!(
        readme
            .findings
            .iter()
            .any(|f| f.rule == "documentation/duplication" && f.locations.len() == 2),
        "{messages:?}"
    );
    assert!(
        readme
            .findings
            .iter()
            .all(|f| f.strength == Strength::Consider)
    );
}

#[test]
fn a_finished_plan_covers_its_section_checks() {
    let project = Project::new();
    project.write("src/old.ts", "export {}\n");
    git(&project, &["init", "-q"]);
    git(&project, &["add", "."]);
    git(&project, &["commit", "-q", "-m", "one"]);
    git(&project, &["tag", "v0.2.0"]);
    std::fs::remove_file(project.0.join("src/old.ts")).unwrap();
    project.write("src/new.ts", "export {}\n");
    project.write(
        "docs/plans/v0.2.0-plan.md",
        "# v0.2.0 plan\n## Task 1\nEdit `src/old.ts` to add the handler.\n",
    );
    git(&project, &["add", "-A"]);
    git(&project, &["commit", "-q", "-m", "two"]);
    let mut options = args();
    options.rules = vec![catalog::DOC_STALENESS.into()];
    let mut eval = scripted(2);
    let report = run(&project, &options, &mut eval);
    let plan = report
        .files
        .iter()
        .find(|f| f.path.ends_with("v0.2.0-plan.md"))
        .unwrap();
    assert_eq!(plan.findings.len(), 1, "{:?}", plan.findings);
    let message = &plan.findings[0].message;
    assert!(
        message.contains("a plan whose work is finished"),
        "{message}"
    );
    assert!(
        message.contains("release tag v0.2.0") && message.contains("`src/old.ts`"),
        "{message}"
    );
    let dimension = &plan.dimensions[catalog::DOC_STALENESS];
    assert_eq!(dimension.units.covered, 1, "{}", dimension.decision_basis);
}
