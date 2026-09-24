//! Unit planning and composition tests, by rule family; shared helpers here.
mod access;
mod documentation;
mod duplicates;
mod functions;
mod hardcoded;
mod nextjs;
mod organization;
mod security;
mod test_rules;

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

/// Numbers other than the sizes an outline sends as evidence (`lines`).
fn numbers_in(value: &Value) -> bool {
    match value {
        Value::Number(_) => true,
        Value::Array(items) => items.iter().any(numbers_in),
        Value::Object(map) => map
            .iter()
            .any(|(key, value)| key != "lines" && numbers_in(value)),
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

fn noul_at(p: f64) -> Value {
    json!({"type":"noul","noul":p})
}

const HARDCODED: &str = "const REGION: &str = \"eu-west-1\";\n\nfn connect() -> Client {\n    Client::new(\"db.internal:5432\", 30_000)\n}\n\nfn total(values: &[i32]) -> i32 {\n    values.iter().sum()\n}\n";

fn hardcoded_project() -> (Project, CheckArgs) {
    rule_project(HARDCODED, catalog::HARDCODED_VALUES)
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

/// Run with every answer at the bottom level except the named Nouls; text a
/// unit produces goes to a remote client, so the settle Choice clears nothing.
fn run_with_nouls(project: &Project, options: &CheckArgs, nouls: &[(&'static str, f64)]) -> Report {
    let mut eval = scripted(0);
    eval.overrides = nouls.iter().map(|&(q, p)| (q, noul_at(p))).collect();
    eval.overrides.push(to_client());
    run(project, options, &mut eval)
}

/// The settle Choice sending a unit's text to a remote client.
fn to_client() -> (&'static str, Value) {
    let probabilities =
        json!({"client": 0.9, "local": 0.025, "logs": 0.025, "caller": 0.025, "stored": 0.025});
    (
        "destination",
        json!({"type":"choice","choice":"client","confidence":0.9,"probabilities":probabilities}),
    )
}

/// The finding that repeated findings across `files` are grouped into; the
/// second file holds it.
fn grouped_primary(files: &mut [crate::schema::FileResult]) -> crate::schema::Finding {
    super::grouping::group_repeats(files);
    files[1].findings[0].clone()
}

/// The first finding of the first file after a run.
fn first_finding(
    project: &Project,
    options: &CheckArgs,
    evaluator: &mut impl crate::transport::Evaluator,
) -> crate::schema::Finding {
    run(project, options, evaluator).files[0].findings[0].clone()
}
