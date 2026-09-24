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
    recheck_overrides: Vec<(&'static str, Value)>,
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
        if recheck {
            apply(&self.recheck_overrides, &mut body);
        } else {
            apply(&self.overrides, &mut body);
        }
        Ok(body)
    }
}

/// Replace every answer whose key ends with an override's suffix.
fn apply(overrides: &[(&'static str, Value)], body: &mut Value) {
    for (suffix, value) in overrides {
        for (key, slot) in body["answers"].as_object_mut().unwrap() {
            if key.ends_with(suffix) {
                *slot = value.clone();
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
        recheck_overrides: Vec::new(),
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

#[test]
fn a_torn_note_gets_the_recheck_and_takes_its_decisive_answer() {
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
    let mut options = args();
    only(&mut options, catalog::FUNCTION_SIMPLIFICATION);
    let mut eval = scripted(0);
    eval.overrides.push(("split", spread(0.1, 0.45, 0.45)));
    eval.recheck_level = Some(2);
    let report = run(&project, &options, &mut eval);
    let file = &report.files[0];
    let strengths: Vec<_> = file
        .findings
        .iter()
        .map(|f| (f.symbol.as_deref(), f.strength))
        .collect();
    assert!(strengths.contains(&(Some("caller"), Strength::Review)));
    assert!(
        strengths.contains(&(Some("helper"), Strength::Note)),
        "no callees, so no recheck"
    );
    options.refresh = true;
    let mut eval = scripted(0);
    eval.overrides.push(("split", spread(0.1, 0.6, 0.3)));
    eval.recheck_level = Some(2);
    run(&project, &options, &mut eval);
    assert!(
        !eval.stages.iter().any(|s| s == "recheck"),
        "a note whose middle level leads is settled"
    );
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
    hollow
        .recheck_overrides
        .push(("own_logic", json!({"type":"noul","noul":0.5})));
    let report = run(&project, &options, &mut hollow);
    assert!(hollow.stages.contains(&"recheck".to_string()));
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
    // The locate follow-up offered none clearly, so no value is named and
    // the review is one level lower.
    let finding = &report.files[0].findings[0];
    assert_eq!(finding.strength, Strength::Consider);
    assert_eq!(finding.rule, "maintainability/hardcoded-values");
    assert!(
        finding.message.starts_with(
            "`connect` special-cases one specific identity (0.90). Which value it means was not found"
        ),
        "{}",
        finding.message
    );
    assert!(finding.action.contains("data or configuration"));
    assert!(finding.values.is_empty());
    assert_eq!(
        report.files[0].dimensions["hardcoded_values"]
            .units
            .consider,
        1
    );
    options.refresh = true;
    let chosen = json!({"type":"choice","choice":"v0","confidence":1.0,
        "probabilities":{"v0":1.0,"v1":0.0,"none":0.0}});
    let report = run_with(
        &options,
        vec![
            ("special", json!({"type":"noul","noul":0.9})),
            ("value", chosen),
        ],
    );
    let finding = &report.files[0].findings[0];
    assert_eq!(finding.strength, Strength::Review);
    assert_eq!(finding.values, ["\"db.internal:5432\""]);
    assert!(
        finding
            .message
            .ends_with("The value is \"db.internal:5432\"."),
        "{}",
        finding.message
    );
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
        ("own_messages", noul_at(0.5)),
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
fn a_foreign_error_message_confirms_an_error_detail_lean_as_a_consider() {
    let (project, options) = security_project(QUERY);
    let mut eval = scripted(0);
    eval.overrides = vec![
        ("error_details", noul_at(0.3)),
        ("exception_to_client", noul_at(0.6)),
        ("own_messages", noul_at(0.1)),
    ];
    let report = run(&project, &options, &mut eval);
    let finding = &report.files[0].findings[0];
    assert_eq!(finding.strength, Strength::Consider);
    assert!(
        finding
            .message
            .contains("text of a library or database error"),
        "{}",
        finding.message
    );
    assert_eq!(
        finding.category.as_deref(),
        Some("CWE-209 error details exposed")
    );
}

#[test]
fn an_undecided_caller_recheck_replaces_an_undecided_traced_lean() {
    let caller = format!(
        "{QUERY}\nfn handler(conn: &Connection, dir: &Path) -> Result<Row> {{\n    find(conn, &dir.join(\"cache\"))\n}}\n"
    );
    let (project, options) = security_project(&caller);
    let mut eval = scripted(0);
    eval.overrides = vec![
        ("resource", noul_at(0.95)),
        ("path", noul_at(0.6)),
        ("origin", spread(0.0, 0.9, 0.1)),
    ];
    eval.recheck_level = Some(0);
    eval.recheck_overrides = vec![("path", noul_at(0.3)), ("origin", spread(0.1, 0.9, 0.0))];
    let report = run(&project, &options, &mut eval);
    assert_eq!(eval.stages.last().unwrap(), "recheck");
    let file = &report.files[0];
    assert!(
        !file
            .findings
            .iter()
            .any(|f| f.rule == "security/injection" && f.symbol.as_deref() == Some("find")),
        "the callers' answer leans away, so no note"
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

/// A special-case finding at line 3 of `path`, naming `values`.
fn hardcoded_finding(path: &str, strength: &str, values: &[&str]) -> Value {
    json!({
        "rule": "maintainability/hardcoded-values", "strength": strength, "line": 3,
        "message": "`f` special-cases one specific identity (0.90).", "action": "Move it",
        "symbol": "f", "rule_version": "1", "concern_probability": 0.9,
        "locations": [{"path": path, "start_line": 3, "end_line": 5, "symbol": "f"}],
        "values": values, "fingerprint": path, "rank": 1.0
    })
}

fn hardcoded_file(path: &str, strength: &str, values: &[&str]) -> crate::schema::FileResult {
    let finding = hardcoded_finding(path, strength, values);
    let units = json!({"judged": 1, "review": usize::from(strength == "review"), "consider": usize::from(strength == "consider"), "note": 0, "clear": 0, "uncertain": 0, "needs_context": 0, "too_small": 0, "omitted": 0});
    serde_json::from_value(json!({
        "path": path, "role": "source", "contains_tests": false, "source_hash": "", "context_files": [],
        "syntax_checked": true, "context_complete": true, "context_limitations": [], "context_requests": [],
        "content_identity": "", "symbols": [], "semantic_size": 1, "input_tokens": 0, "output_tokens": 0,
        "status": strength, "cached": false, "evaluated_at": null, "model": null, "elapsed_ms": 0,
        "dimensions": {"hardcoded_values": {"status": strength, "concern_probability": 0.9,
            "decision_basis": "", "rule_version": "1", "units": units}},
        "findings": [finding], "error": null
    }))
    .unwrap()
}

#[test]
fn a_literal_repeated_across_files_is_one_finding_and_its_repeats_are_notes() {
    use crate::schema::{Status, Strength};
    let mut files = vec![
        hardcoded_file("a.ts", "consider", &["'acme-corp'", "0"]),
        hardcoded_file("b.ts", "review", &["'acme-corp'"]),
        hardcoded_file("c.ts", "consider", &["0"]),
        hardcoded_file("d.ts", "consider", &["'acme-corp'", "1"]),
    ];
    super::grouping::group_repeats(&mut files);
    let primary = &files[1].findings[0];
    assert_eq!(primary.strength, Strength::Review);
    assert_eq!(primary.locations.len(), 3);
    assert!(
        primary.message.contains("also flagged at a.ts:3, d.ts:3"),
        "{}",
        primary.message
    );
    for repeat in [&files[0], &files[3]] {
        assert_eq!(repeat.findings[0].strength, Strength::Note);
        assert!(
            repeat.findings[0]
                .message
                .contains("Same value 'acme-corp' as b.ts:3")
        );
        assert_eq!(repeat.status, Status::Note);
        assert_eq!(repeat.dimensions["hardcoded_values"].status, Status::Note);
    }
    // A short shared value such as `0` never links findings.
    assert_eq!(files[2].findings[0].strength, Strength::Consider);
}

fn plan_file(path: &str) -> crate::schema::FileResult {
    let mut file = hardcoded_file(path, "consider", &[]);
    let dimension = file.dimensions.remove("hardcoded_values").unwrap();
    file.dimensions.insert("doc_staleness".into(), dimension);
    let finding = &mut file.findings[0];
    finding.rule = "documentation/staleness".into();
    finding.category = Some(super::grouping::FINISHED_PLAN.into());
    finding.message = format!("`{path}` is a plan whose work is finished: tag v1 (0.95).");
    file
}

#[test]
fn finished_plans_in_one_directory_are_one_finding_named_by_the_directory() {
    use crate::schema::{Status, Strength};
    let mut files = vec![
        plan_file("docs/plans/b.md"),
        plan_file("docs/plans/a.md"),
        plan_file("docs/plans/c.md"),
        plan_file("docs/other/d.md"),
    ];
    super::grouping::group_repeats(&mut files);
    let primary = &files[1].findings[0];
    assert_eq!(primary.strength, Strength::Consider);
    assert!(
        primary.message.ends_with(
            "The other 2 plans in `docs/plans` are finished too: `docs/plans/b.md`, `docs/plans/c.md`."
        ),
        "{}",
        primary.message
    );
    assert_eq!(primary.locations.len(), 3);
    for member in [&files[0], &files[2]] {
        assert_eq!(member.findings[0].strength, Strength::Note);
        assert_eq!(member.dimensions["doc_staleness"].units.note, 1);
        assert_eq!(member.status, Status::Note);
    }
    assert_eq!(
        files[3].findings[0].strength,
        Strength::Consider,
        "a plan alone in its directory"
    );
    // The group keeps its identity when a plan is added or removed.
    let fingerprint = primary.fingerprint.clone();
    let mut fewer = vec![plan_file("docs/plans/c.md"), plan_file("docs/plans/b.md")];
    super::grouping::group_repeats(&mut fewer);
    assert_eq!(fewer[1].findings[0].fingerprint, fingerprint);
}

const FIRST_MIGRATION: &str = "create table public.notes (id uuid primary key, owner_id uuid not null, body text);\nalter table public.notes enable row level security;\ncreate policy \"read notes\" on public.notes for select using (true);\n";
const SECOND_MIGRATION: &str = "drop policy \"read notes\" on public.notes;\ncreate policy \"read own notes\" on public.notes for select using (owner_id = auth.uid());\ncreate function public.note_count(uid uuid) returns bigint language sql security definer as $$ select count(*) from public.notes where owner_id = uid $$;\ngrant select on public.notes to authenticated;\n";
const WORKFLOW_FILE: &str = "on:\n  pull_request_target:\njobs:\n  greet:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo \"${{ github.event.pull_request.title }}\"\n  build:\n    runs-on: ubuntu-latest\n    steps:\n      - run: make\n";

/// A project holding `files`, checked for `rules` only.
fn project_with(files: &[(&str, &str)], rules: &[&str]) -> (Project, CheckArgs) {
    let project = Project::new();
    for (path, text) in files {
        project.write(path, text);
    }
    let mut options = args();
    options.rules = rules.iter().map(|r| r.to_string()).collect();
    (project, options)
}

fn configuration_project() -> (Project, CheckArgs) {
    project_with(
        &[
            ("supabase/migrations/1_init.sql", FIRST_MIGRATION),
            ("supabase/migrations/2_own.sql", SECOND_MIGRATION),
            (".github/workflows/greet.yml", WORKFLOW_FILE),
            ("lib.rs", &function("unrelated")),
        ],
        &[catalog::ACCESS_CONTROL, catalog::WORKFLOWS],
    )
}

#[test]
fn access_units_follow_the_final_state_across_migrations() {
    let (project, options) = configuration_project();
    let (inputs, plan) = planned(&project, &options);
    assert!(
        inputs
            .iter()
            .all(|i| i.result.path.extension().is_some_and(|e| e != "rs")),
        "no application source is collected for these rules"
    );
    let access: Vec<(&str, &str)> = plan
        .requests
        .iter()
        .filter(|p| p.request["jevgate"]["stage"] == "access")
        .map(|p| {
            let state = &p.request["state"];
            let kind = ["policy", "function", "statement"]
                .into_iter()
                .find(|k| state.get(k).is_some())
                .unwrap();
            (kind, state["file"]["path"].as_str().unwrap())
        })
        .collect();
    assert_eq!(
        access,
        [
            ("policy", "supabase/migrations/2_own.sql"),
            ("function", "supabase/migrations/2_own.sql"),
            ("statement", "supabase/migrations/2_own.sql"),
        ],
        "the dropped policy is not judged"
    );
    let policy = plan
        .requests
        .iter()
        .find(|p| p.request["state"]["policy"].is_object())
        .unwrap();
    assert!(
        policy.request["state"]["table"]["source"]
            .as_str()
            .unwrap()
            .starts_with("create table public.notes"),
        "the table from the earlier migration is evidence"
    );
    let grant = plan
        .requests
        .iter()
        .find(|p| p.request["state"]["statement"].is_object())
        .unwrap();
    assert_eq!(grant.request["state"]["table"]["row_level_security"], true);
    let jobs: Vec<&str> = plan
        .requests
        .iter()
        .filter(|p| p.request["jevgate"]["stage"] == "workflows")
        .map(|p| p.request["state"]["job"]["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        jobs,
        ["greet", "build"],
        "both jobs run on pull_request_target"
    );
    let greet = &plan
        .requests
        .iter()
        .find(|p| p.request["state"]["job"]["name"] == "greet")
        .unwrap()
        .request;
    assert_eq!(
        greet["state"]["expressions"],
        json!(["github.event.pull_request.title"])
    );
    assert_eq!(greet["questions"].as_object().unwrap().len(), 2);
}

#[test]
fn access_control_judges_migrations_beside_the_code_rules() {
    let (project, mut options) = configuration_project();
    options.rules.push(catalog::INJECTION.into());
    let (inputs, plan) = planned(&project, &options);
    let migration = inputs
        .iter()
        .find(|i| i.result.path.ends_with("2_own.sql"))
        .unwrap();
    assert_eq!(migration.result.role, crate::inventory::SQL);
    assert_eq!(
        plan.requests
            .iter()
            .filter(|p| p.request["jevgate"]["stage"] == "access")
            .count(),
        3,
        "the code rules' walk does not hide migrations from access control"
    );
}

#[test]
fn an_unchecked_definer_is_a_review_and_an_open_search_path_a_consider() {
    let (project, options) = configuration_project();
    let mut eval = scripted(0);
    eval.overrides = vec![("search_path", noul_at(0.9)), ("outside", noul_at(0.95))];
    let report = run(&project, &options, &mut eval);
    let file = |name: &str| {
        report
            .files
            .iter()
            .find(|f| f.path.ends_with(name))
            .unwrap()
    };
    let definer = &file("2_own.sql").findings[0];
    assert_eq!(definer.strength, Strength::Consider);
    assert_eq!(definer.rule, "security/access-control");
    assert_eq!(
        definer.category.as_deref(),
        Some("CWE-426 untrusted search path")
    );
    assert!(
        definer
            .message
            .starts_with("SECURITY DEFINER function `note_count`")
    );
    assert_eq!(file("1_init.sql").status, Status::NotApplicable);
    let job = &file("greet.yml").findings[0];
    assert_eq!(job.strength, Strength::Review);
    assert_eq!(job.category.as_deref(), Some("CWE-78 command injection"));
    assert!(
        job.message
            .contains("`${{ github.event.pull_request.title }}`"),
        "{}",
        job.message
    );
    assert_eq!(job.locations[0].start_line, 4);
}

const VITEST: &str = "import { vi } from 'vitest'\nimport { repo } from './repo'\nimport { getProfile } from './profile'\nvi.mock('./repo')\n\ndescribe('profile', () => {\n  beforeEach(() => {\n    repo.findProfile.mockReset()\n  })\n\n  it('returns the stored profile', async () => {\n    repo.findProfile.mockResolvedValue({ id: 'u1', name: 'Ana' })\n    const result = await getProfile('u1')\n    expect(result).toEqual({ id: 'u1', name: 'Ana' })\n  })\n})\n";
const PROFILE: &str = "export async function getProfile(id: string) {\n  if (!id) {\n    throw new Error('missing id')\n  }\n  const profile = await repo.findProfile(id)\n  return profile\n}\n";

#[test]
fn an_undecided_test_is_asked_again_with_its_subjects_and_setup() {
    let project = Project::new();
    project.write("src/profile.ts", PROFILE);
    project.write("src/profile.test.ts", VITEST);
    let mut options = args();
    options.include_tests = true;
    only(&mut options, catalog::TEST_VALUE);
    let (_, plan) = planned(&project, &options);
    let file = plan
        .files
        .values()
        .find(|f| f.path.ends_with("profile.test.ts"))
        .unwrap();
    let (request, _) = file.units[0].recheck.as_ref().expect("a recheck");
    let state = &request["state"];
    assert!(
        state["subjects"][0]["source"]
            .as_str()
            .unwrap()
            .contains("repo.findProfile(id)")
    );
    let setup = state["setup"].as_str().unwrap();
    assert!(
        setup.starts_with("import { vi } from 'vitest'") && setup.contains("vi.mock('./repo')"),
        "{setup}"
    );
    assert!(setup.contains("beforeEach(() => {\n    repo.findProfile.mockReset()\n  })"));
    assert!(!setup.contains("describe("), "{setup}");
    assert_eq!(
        request["jevgate"]["sources"].as_array().unwrap().len(),
        2,
        "the subject's file is checked for freshness"
    );
    let first = plan
        .requests
        .iter()
        .find(|p| p.request["jevgate"]["stage"] == "tests")
        .unwrap();
    assert!(
        first.request["state"]["subjects"][0]["source"].is_null(),
        "the first pass sends signatures only"
    );
    let test_value = |report: &Report| {
        report
            .files
            .iter()
            .find(|f| f.path.ends_with("profile.test.ts"))
            .unwrap()
            .clone()
    };
    let mut eval = scripted(0);
    eval.overrides = vec![("own_logic", noul_at(0.5))];
    let report = run(&project, &options, &mut eval);
    assert!(eval.stages.contains(&"recheck".to_string()));
    assert_eq!(
        test_value(&report).dimensions["test_value"].status,
        Status::Clear
    );
    options.refresh = true;
    let mut eval = scripted(0);
    eval.overrides = vec![("own_logic", noul_at(0.5))];
    eval.recheck_overrides = vec![("mock_only", noul_at(0.95))];
    let file = test_value(&run(&project, &options, &mut eval));
    assert_eq!(file.findings[0].strength, Strength::Review);
    assert!(
        file.findings[0]
            .message
            .contains("only checks values its mocks")
    );
}

#[test]
fn a_test_files_setup_is_its_head_and_hooks_and_long_parts_are_left_out() {
    let python = "import pytest\nfrom app import total\n\nclass TotalTest(TestCase):\n    def setUp(self):\n        self.rows = [1, 2]\n\n    def test_total(self):\n        self.assertEqual(total(self.rows), 3)\n";
    let setup = test_units::file_setup(python, 1, 8);
    assert_eq!(
        setup,
        "import pytest\nfrom app import total\n\ndef setUp(self):\n        self.rows = [1, 2]"
    );
    let long = format!("{}it('x', () => {{}})\n", "// padding\n".repeat(500));
    assert_eq!(test_units::file_setup(&long, 1, 501), "");
}

const ROUTE: &str = "export async function loadThing(c: Context) {\n  const { data, error } = await db.from('things').select('*').eq('id', c.req.param('id'))\n  if (error) throw new InternalError(`Query failed: ${error.message}`, error)\n  if (!data) throw new NotFoundError('Thing not found')\n  return c.json(data)\n}\n";

fn choice_of(chosen: &str, options: &[&str]) -> Value {
    let probabilities: serde_json::Map<String, Value> = options
        .iter()
        .map(|o| {
            (
                o.to_string(),
                json!(if *o == chosen {
                    0.9
                } else {
                    0.1 / (options.len() - 1) as f64
                }),
            )
        })
        .collect();
    json!({"type":"choice","choice":chosen,"confidence":0.9,"probabilities":probabilities})
}

#[test]
fn each_created_error_message_is_asked_about_and_names_the_foreign_one() {
    let project = Project::new();
    project.write("routes.ts", ROUTE);
    let mut options = args();
    options.rules = vec![catalog::SENSITIVE_DATA.into()];
    let (_, plan) = planned(&project, &options);
    let Detail::Security {
        trace: Some((trace, _)),
        messages,
        ..
    } = &plan.files[&0].units[0].detail
    else {
        panic!("a traced security unit");
    };
    assert_eq!(
        messages,
        &["`Query failed: ${error.message}`", "'Thing not found'"]
    );
    assert_eq!(trace["state"]["messages"][1]["id"], "m1");
    assert!(trace["questions"].get("messages").is_some());
    assert!(
        trace["questions"].get("own_messages").is_none(),
        "the Choice replaces the Noul"
    );
    let run_with = |options: &CheckArgs, chosen: &str| {
        let mut eval = scripted(0);
        eval.overrides = vec![
            ("error_details", noul_at(0.3)),
            ("exception_to_client", noul_at(0.6)),
            ("messages", choice_of(chosen, &["m0", "m1", "none"])),
        ];
        run(&project, options, &mut eval)
    };
    let report = run_with(&options, "none");
    assert_eq!(
        report.files[0].dimensions[catalog::SENSITIVE_DATA].status,
        Status::Clear,
        "every message is the program's own"
    );
    options.refresh = true;
    let report = run_with(&options, "m0");
    let finding = &report.files[0].findings[0];
    assert_eq!(finding.strength, Strength::Consider);
    assert!(
        finding.message.contains("into an error message (0.90)")
            && finding
                .message
                .ends_with("The message is `Query failed: ${error.message}`."),
        "{}",
        finding.message
    );
}

#[test]
fn a_registered_error_handler_is_one_unit_judged_with_the_error_classes() {
    let project = Project::new();
    project.write(
        "src/app.ts",
        "import { errorHandler } from './middleware/error-handler'\nconst app = new Hono()\napp.onError(errorHandler)\nexport default app\n",
    );
    project.write(
        "src/middleware/error-handler.ts",
        "export const errorHandler = (err, c) => {\n  logger.error(err)\n  if (err instanceof AppError) {\n    return c.json({ error: { code: err.code, message: err.message } }, err.status)\n  }\n  return c.json({ error: { message: err.message, stack: err.stack } }, 500)\n}\n",
    );
    project.write(
        "src/lib/errors.ts",
        "export class AppError extends Error {\n  constructor(public status: number, public code: string, message: string) {\n    super(message)\n  }\n}\n",
    );
    project.write(
        "src/app.test.ts",
        "import { errorHandler } from './middleware/error-handler'\ntest('x', () => {\n  app.onError(errorHandler)\n})\n",
    );
    let mut options = args();
    options.rules = vec![catalog::SENSITIVE_DATA.into()];
    let (inputs, plan) = planned(&project, &options);
    let handlers: Vec<(&std::path::Path, &UnitPlan)> = plan
        .files
        .values()
        .flat_map(|f| f.units.iter().map(move |u| (f.path.as_path(), u)))
        .filter(|(_, u)| matches!(u.detail, Detail::Handler { .. }))
        .collect();
    assert_eq!(handlers.len(), 1, "registered once outside tests");
    let (path, unit) = handlers[0];
    assert_eq!(
        path,
        std::path::Path::new("src/middleware/error-handler.ts")
    );
    assert_eq!(unit.name, "errorHandler");
    let request = &plan
        .requests
        .iter()
        .find(|p| p.request["state"]["error_handler"].is_object())
        .unwrap()
        .request;
    assert_eq!(
        request["state"]["error_handler"]["registered"],
        "`app.onError(errorHandler)` (src/app.ts:3)"
    );
    assert!(
        request["state"]["error_classes"]
            .as_str()
            .unwrap()
            .starts_with("export class AppError")
    );
    assert_eq!(request["jevgate"]["sources"].as_array().unwrap().len(), 2);
    assert!(inputs.len() >= 3);
    let mut eval = scripted(0);
    eval.overrides = vec![("handler_leaks", noul_at(0.95))];
    let report = run(&project, &options, &mut eval);
    let file = report
        .files
        .iter()
        .find(|f| f.path.ends_with("error-handler.ts"))
        .unwrap();
    let finding = file
        .findings
        .iter()
        .find(|f| f.symbol.as_deref() == Some("errorHandler") && f.strength == Strength::Review)
        .unwrap();
    assert!(
        finding
            .message
            .starts_with("`errorHandler`, the error handler registered by `app.onError(errorHandler)` (src/app.ts:3), sends clients"),
        "{}",
        finding.message
    );
    assert_eq!(
        finding.category.as_deref(),
        Some("CWE-209 error details exposed")
    );
}

#[test]
fn framework_error_handlers_are_found_where_they_are_implemented_or_used() {
    let project = Project::new();
    project.write(
        "src/error.rs",
        "use axum::response::{IntoResponse, Response};\n\n#[derive(thiserror::Error, Debug)]\npub enum Error {\n    #[error(\"request path not found\")]\n    NotFound,\n    #[error(\"an internal server error occurred\")]\n    Anyhow(#[from] anyhow::Error),\n}\n\n#[derive(Debug)]\npub struct TimeoutError;\n\nimpl IntoResponse for Error {\n    fn into_response(self) -> Response {\n        (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()).into_response()\n    }\n}\n\nimpl IntoResponse for Page {\n    fn into_response(self) -> Response {\n        Html(self.0).into_response()\n    }\n}\n",
    );
    project.write(
        "src/server.ts",
        "const app = express()\napp.use(express.json())\napp.use(cors({ origin: true }))\napp.use((err: Error, req: Request<{}, any>, res: Response, next: NextFunction) => {\n  res.status(500).json({ message: err.message })\n})\n",
    );
    project.write(
        "src/filter.ts",
        "@Catch(HttpException)\nexport class HttpErrorFilter implements ExceptionFilter {\n  catch(exception: HttpException, host: ArgumentsHost) {\n    host.switchToHttp().getResponse().status(500).json(exception.getResponse())\n  }\n}\n",
    );
    let mut options = args();
    options.rules = vec![catalog::SENSITIVE_DATA.into()];
    let (_, plan) = planned(&project, &options);
    let registered: Vec<String> = plan
        .files
        .values()
        .flat_map(|f| &f.units)
        .filter_map(|u| match &u.detail {
            Detail::Handler { registered } => Some(registered.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(registered.len(), 3, "{registered:?}");
    assert!(
        registered.contains(&"`impl IntoResponse for Error` (src/error.rs:15)".to_string()),
        "{registered:?}"
    );
    assert!(
        registered
            .iter()
            .any(|r| r.starts_with("`app.use((err: Error"))
    );
    assert!(
        registered.contains(&"`@Catch(…) class HttpErrorFilter` (src/filter.ts:3)".to_string())
    );
    let classes = plan
        .requests
        .iter()
        .find(|p| {
            p.request["state"]["error_handler"]["registered"]
                .as_str()
                .is_some_and(|r| r.contains("IntoResponse"))
        })
        .unwrap()
        .request["state"]["error_classes"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        classes.starts_with("#[derive(thiserror::Error, Debug)]\npub enum Error {"),
        "{classes}"
    );
    assert!(classes.contains("an internal server error occurred"));
    assert!(classes.ends_with("pub struct TimeoutError;"), "{classes}");
}

#[test]
fn an_injection_trace_shows_the_enums_its_sites_name() {
    let project = Project::new();
    project.write(
        "server/store.ts",
        "import { ConfigKey } from '../shared/config'\n\nexport async function setAITagConfig(DB: D1Database, config: AITagConfig): Promise<boolean> {\n  const insertSql = `INSERT INTO stores (key, value) VALUES ('${ConfigKey.aiTag}', ?) ON CONFLICT(key) DO UPDATE SET value = ?`\n  const bindValue = JSON.stringify(config)\n  const result = await DB.prepare(insertSql).bind(bindValue, bindValue).run()\n  return result.success\n}\n",
    );
    project.write(
        "shared/config.ts",
        "enum ConfigKey {\n  shouldShowRecent = 'config/should_show_recent',\n  aiTag = 'config/ai_tag',\n}\n\nexport { ConfigKey }\n",
    );
    let mut options = args();
    options.rules = vec![catalog::INJECTION.into()];
    let (_, plan) = planned(&project, &options);
    let traces: Vec<&Value> = plan
        .files
        .values()
        .flat_map(|f| &f.units)
        .filter_map(|u| match &u.detail {
            Detail::Security {
                trace: Some((request, _)),
                ..
            } => Some(request),
            _ => None,
        })
        .collect();
    assert_eq!(traces.len(), 1);
    assert!(
        traces[0]["state"]["enums_named_in_sites"][0]
            .as_str()
            .is_some_and(|e| e.starts_with("enum ConfigKey {")),
        "{}",
        traces[0]["state"]
    );
}

const STDB_TABLES: &str = "import { table, t } from 'spacetimedb/server'\n\nexport const character = table(\n  { public: true },\n  {\n    id: t.u64().primaryKey(),\n    owner: t.identity(),\n    name: t.string(),\n  },\n)\n\nexport const account = table({ public: false }, { owner: t.identity().primaryKey() })\n";
const STDB_COMMANDS: &str = "import { t } from 'spacetimedb/server'\nimport { database } from './schema'\nimport { ownedCharacter } from './owned'\n\nexport const renameCharacter = database.reducer({ characterId: t.u64(), name: t.string() }, (ctx, { characterId, name }) => {\n  const row = ownedCharacter(ctx, characterId)\n  ctx.db.character.id.update({ ...row, name })\n})\n\nexport const myCharacters = database.view({ public: true }, t.array(character.rowType), (ctx) => {\n  return [...ctx.db.character.owner.filter(ctx.sender)]\n})\n";
const STDB_OWNED: &str = "export function ownedCharacter(ctx, id) {\n  const row = ctx.db.character.id.find(id)\n  if (!row || !row.owner.isEqual(ctx.sender)) throw new Error('NOT_OWNER')\n  return row\n}\n";

fn spacetimedb_project() -> (Project, CheckArgs) {
    project_with(
        &[
            (
                "server/package.json",
                "{\"dependencies\": {\"spacetimedb\": \"^2.10.0\"}}",
            ),
            ("server/src/tables.ts", STDB_TABLES),
            ("server/src/commands.ts", STDB_COMMANDS),
            ("server/src/owned.ts", STDB_OWNED),
            (
                "web/src/owned.ts",
                "export function ownedCharacter(id) {\n  return fetch(`/characters/${id}`)\n}\n",
            ),
        ],
        &[catalog::ACCESS_CONTROL],
    )
}

#[test]
fn spacetimedb_tables_views_and_reducers_are_judged_with_helpers_and_the_version() {
    let (project, options) = spacetimedb_project();
    let (_, plan) = planned(&project, &options);
    let units: Vec<(&str, &Detail)> = plan
        .files
        .values()
        .flat_map(|f| f.units.iter().map(|u| (u.name.as_str(), &u.detail)))
        .collect();
    assert_eq!(units.len(), 3, "the private table is not asked about");
    let reducer = plan
        .requests
        .iter()
        .find(|p| p.request["state"]["reducer"].is_object())
        .unwrap();
    let state = &reducer.request["state"];
    assert_eq!(state["helpers"][0]["name"], "ownedCharacter");
    assert!(
        state["helpers"][0]["source"]
            .as_str()
            .unwrap()
            .contains("ctx.sender"),
        "the helper nearest the module, not the web client's"
    );
    let note = reducer.request["questions"]["reach"]["instructions"]["note"]
        .as_str()
        .unwrap();
    assert!(
        note.contains("SpacetimeDB 2.10.0")
            && note.contains("Scheduled reducers (named by a table's `scheduled` option, shown as `scheduled_by`) are private")
    );
    assert_eq!(
        reducer.request["jevgate"]["sources"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let table = plan
        .requests
        .iter()
        .find(|p| p.request["state"]["table"].is_object())
        .unwrap();
    assert_eq!(
        table.request["state"]["table"]["columns_naming_users"],
        json!(["owner"])
    );
    // Acceptable levels clear; a literal check at review raises the reducer,
    // while a public table is at most a consider.
    let report = run(&project, &options, &mut scripted(0));
    assert!(
        report
            .files
            .iter()
            .filter(|f| f.path.starts_with("server/src"))
            .all(|f| f.status == Status::Clear || f.status == Status::NotApplicable),
        "{:?}",
        report
            .files
            .iter()
            .map(|f| (&f.path, &f.status))
            .collect::<Vec<_>>()
    );
    let mut options = options;
    options.refresh = true;
    let mut eval = scripted(0);
    eval.overrides = vec![("argument_rows", noul_at(0.9)), ("exposed", noul_at(0.95))];
    let report = run(&project, &options, &mut eval);
    let findings: Vec<&crate::schema::Finding> =
        report.files.iter().flat_map(|f| &f.findings).collect();
    let reducer = findings
        .iter()
        .find(|f| f.symbol.as_deref() == Some("renameCharacter"))
        .unwrap();
    assert_eq!(reducer.strength, Strength::Review);
    assert!(
        reducer
            .message
            .starts_with("Reducer `renameCharacter` reads or changes a row its arguments choose"),
        "{}",
        reducer.message
    );
    assert_eq!(
        reducer.category.as_deref(),
        Some("CWE-639 authorization through a user-controlled key")
    );
    let table = findings
        .iter()
        .find(|f| f.symbol.as_deref() == Some("character"))
        .unwrap();
    assert_eq!(table.strength, Strength::Consider);
    assert!(
        table
            .message
            .starts_with("Public table `character` likely lets every client read")
    );
}
