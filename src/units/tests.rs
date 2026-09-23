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
    request["state"]["callees"].is_array()
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

/// A project whose `lib.rs` holds `source`, checked for function simplification only.
fn function_rule_project(source: &str) -> (Project, CheckArgs) {
    let project = Project::new();
    project.write("lib.rs", source);
    let mut options = args();
    only(&mut options, catalog::FUNCTION_SIMPLIFICATION);
    (project, options)
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
        compose::split_has_users(&unknown, None),
        "missing evidence stands"
    );
    let shared = [group("G1", &["a.rs"]), group("G2", &["a.rs"])];
    assert!(!compose::split_has_users(&shared, None));
    let own = [group("G1", &["a.rs", "b.rs"]), group("G2", &["a.rs"])];
    assert!(compose::split_has_users(&own, None));
    assert!(compose::split_has_users(&own, Some(&chosen("G1"))));
    assert!(!compose::split_has_users(&own, Some(&chosen("G2"))));
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
