use super::*;
use crate::{
    schema::{Report, Status, Strength},
    tests::{Mock, Project, answer, args, function, run},
};

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
    let project = Project::new();
    let mut source = String::new();
    for i in 0..10 {
        source.push_str(&function(&format!("f{i}")));
    }
    project.write("lib.rs", &source);
    let options = args();
    let (inputs, plan) = planned(&project, &options);
    let functions: Vec<_> = plan
        .requests
        .iter()
        .filter(|p| p.request["jevgate"]["stage"] == "functions")
        .collect();
    assert_eq!(functions.len(), 2, "ten functions pack into two requests");
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
        let recheck = request["state"]["callees"].is_array()
            || request["state"]["site_a"]["function_source"].is_string();
        self.stages
            .push(if recheck { "recheck" } else { "first" }.into());
        let level = if recheck {
            self.recheck_level.unwrap_or(self.level)
        } else {
            self.level
        };
        let mut body = answer(request, level);
        if !recheck {
            for (suffix, value) in &self.overrides {
                for (key, slot) in body["answers"].as_object_mut().unwrap() {
                    if key.ends_with(suffix) {
                        *slot = value.clone();
                    }
                }
            }
        }
        Ok(body)
    }
}

fn scripted(level: usize) -> Scripted {
    Scripted {
        level,
        overrides: Vec::new(),
        recheck_level: None,
        stages: Vec::new(),
    }
}

fn only(options: &mut CheckArgs, rule: &str) {
    options.rules = vec![rule.into()];
}

#[test]
fn a_review_function_always_carries_a_finding_even_without_a_task_kind() {
    // A decisive review whose follow-up location is not decisive must never pass.
    let project = Project::new();
    project.write("lib.rs", &function("busy"));
    let mut options = args();
    only(&mut options, catalog::FUNCTION_SIMPLIFICATION);
    let mut eval = scripted(2);
    eval.overrides.push((
        "task_kind",
        json!({"type":"choice","choice":"none","confidence":0.4,
            "probabilities":{"validation":0.1,"parsing":0.1,"transformation":0.1,"io":0.1,
            "formatting":0.1,"error_handling":0.1,"setup_cleanup":0.1,"coordination":0.1,"none":0.2}}),
    ));
    let report = run(&project, &options, &mut eval);
    let file = &report.files[0];
    assert_eq!(file.status, Status::Review);
    assert_eq!(file.findings.len(), 1);
    let finding = &file.findings[0];
    assert_eq!(finding.strength, Strength::Review);
    assert_eq!(finding.rule, "maintainability/function-simplification");
    assert_eq!(finding.locations[0].symbol.as_deref(), Some("busy"));
    assert!(finding.message.contains("two or more substantial tasks"));
    assert!((finding.rank - 1.0 * (1.0 + 8.0f64).ln()).abs() < 1e-9);
    assert_eq!(crate::gate::exit_code(&report), 1);
}

#[test]
fn flatten_alone_is_a_review_and_the_middle_level_is_consider() {
    let project = Project::new();
    project.write("lib.rs", &function("nested"));
    let mut options = args();
    only(&mut options, catalog::FUNCTION_SIMPLIFICATION);
    let mut eval = scripted(0);
    eval.overrides
        .push(("flatten", json!({"type":"noul","noul":0.9})));
    let report = run(&project, &options, &mut eval);
    assert!(report.files[0].findings[0].message.contains("flattened"));
    options.refresh = true;
    let report = run(&project, &options, &mut scripted(1));
    assert_eq!(report.files[0].status, Status::Consider);
    assert_eq!(report.files[0].findings[0].strength, Strength::Consider);
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
    let mut options = args();
    only(&mut options, catalog::FUNCTION_SIMPLIFICATION);
    let mut eval = scripted(3);
    eval.recheck_level = Some(2);
    let report = run(&project, &options, &mut eval);
    assert_eq!(
        eval.stages,
        ["first", "recheck"],
        "only the caller has callees"
    );
    let file = &report.files[0];
    let dimension = &file.dimensions["function_simplification"];
    assert_eq!((dimension.units.review, dimension.units.uncertain), (1, 1));
    assert!(file.judgments.iter().any(|j| j.pass == Pass::Recheck));
    assert!(
        file.judgments
            .iter()
            .any(|j| j.unit == "function:caller" && j.pass == Pass::First)
    );
    assert_eq!(report.stages["recheck"].successful_requests, 1);
    options.refresh = true;
    let mut still = scripted(3);
    let report = run(&project, &options, &mut still);
    assert_eq!(report.files[0].status, Status::Uncertain);
    assert!(report.files[0].findings.is_empty());
}

#[test]
fn file_organization_review_without_a_module_choice_is_a_file_wide_finding() {
    let project = Project::new();
    project.write(
        "lib.rs",
        &format!(
            "struct Cache {{ entries: Vec<u8> }}\n{}struct Page {{ body: String }}\n{}",
            function("warm"),
            function("render")
        ),
    );
    let mut options = args();
    only(&mut options, catalog::FILE_ORGANIZATION);
    let report = run(&project, &options, &mut scripted(2));
    let file = &report.files[0];
    assert_eq!(file.status, Status::Review);
    let finding = &file.findings[0];
    assert!(finding.message.contains("separate purposes"));
    assert_eq!(finding.locations[0].start_line, 1);
    assert!(finding.symbol.is_none());
}

const LOAD: &str = "fn load_user(path: &str) -> Result<User> {\n    let text = std::fs::read_to_string(path)?;\n    let value: Value = serde_json::from_str(&text)?;\n    let name = value[\"name\"].as_str().unwrap_or(\"anonymous\").trim().to_string();\n    Ok(User { name })\n}\n";

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
    let project = Project::new();
    let mut source = String::new();
    for i in 0..12 {
        source.push_str(&function(&format!("f{i}")));
    }
    project.write("lib.rs", &source);
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
