//! The test harness (projects, scripted answers, runs) and whole-run tests;
//! the gate and baseline tests are in `gating`, and what a run judges
//! (unsupported or oversized input, roles, context) in `scope`.
use super::*;
use crate::{config::ConfigContext, options::CheckArgs};
use clap::Parser;
use serde_json::{Value, json};
use std::path::PathBuf;
mod gating;
mod scope;
#[path = "../../tests/support/temp_dir.rs"]
mod temp_dir;

pub(super) struct Project(pub(super) temp_dir::TempDir);
impl Project {
    pub(super) fn new() -> Self {
        Self(temp_dir::TempDir::new("jev-unit"))
    }
    pub(super) fn write(&self, name: &str, text: &str) {
        std::fs::create_dir_all(self.0.join(name).parent().unwrap()).unwrap();
        std::fs::write(self.0.join(name), text).unwrap();
    }
    pub(super) fn context(&self) -> ConfigContext {
        ConfigContext {
            invocation_dir: self.0.to_path_buf(),
            root: self.0.to_path_buf(),
            config: Default::default(),
        }
    }
}

#[derive(Parser)]
struct TestCli {
    #[command(flatten)]
    args: CheckArgs,
}
pub(super) fn args() -> CheckArgs {
    let mut a = TestCli::parse_from(["test"]).args;
    a.rules = crate::catalog::keys().into_iter().map(Into::into).collect();
    a.fail_on = vec![options::FailOn::Review];
    a
}

/// A function large enough to judge (five body lines).
/// A shared-logic finding at `src/a,b.rs:12`, with text the output formats escape.
pub(super) fn finding(strength: crate::schema::Strength) -> crate::schema::Finding {
    crate::schema::Finding {
        rule: "maintainability/shared-logic".into(),
        strength,
        line: 12,
        message: "Copies: 50% alike,\nsee `b`".into(),
        action: "Share one | implementation".into(),
        symbol: None,
        rule_version: String::new(),
        concern_probability: 0.9,
        locations: vec![crate::schema::Location {
            path: "src/a,b.rs".into(),
            start_line: 12,
            end_line: 20,
            symbol: None,
        }],
        quote: None,
        category: None,
        values: Vec::new(),
        fingerprint: String::new(),
        rank: 1.0,
        baselined: false,
        suppressed: None,
    }
}

pub(super) fn function(name: &str) -> String {
    format!(
        "fn {name}(values: &[i32]) -> i32 {{\n    let mut total = 0;\n    for value in values {{\n        total += value;\n    }}\n    let doubled = total * 2;\n    doubled + 1\n}}\n"
    )
}

/// Levels: 0 answers the bottom of every scale (clear), 1 the middle (consider,
/// or a note where the middle says the code is fine), 2 the top (review),
/// 3 spreads probability (uncertain), 4 leans to the top without reaching review.
pub(super) fn answer(request: &Value, level: usize) -> Value {
    let answers = request["questions"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(name, q)| (name.clone(), typed_answer(q, level)))
        .collect::<serde_json::Map<_, _>>();
    json!({"model":request["model"],"answers":answers,"usage":{"input_tokens":10,"output_tokens":0}})
}

/// A valid answer of the question's type at `level` (see [`answer`]).
fn typed_answer(question: &Value, level: usize) -> Value {
    match question["type"].as_str().unwrap() {
        "noul" => {
            let noul = [0.05, 0.5, 0.95, 0.5, 0.5][level];
            json!({"type":"noul","noul":noul})
        }
        "score" => {
            let p = [
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.0, 0.0, 1.0],
                [0.4, 0.2, 0.4],
                [0.1, 0.3, 0.6],
            ][level];
            json!({"type":"score","score":p[1] + 2.0 * p[2],"confidence":1.0,
                "probabilities":{"0":p[0],"1":p[1],"2":p[2]}})
        }
        _ => choice_answer(question["criteria"].as_object().unwrap()),
    }
}

/// A certain Choice of `none` when offered, else the first option.
fn choice_answer(options: &serde_json::Map<String, Value>) -> Value {
    let chosen = if options.contains_key("none") {
        "none"
    } else {
        options.keys().next().unwrap()
    };
    let probabilities: serde_json::Map<_, _> = options
        .keys()
        .map(|k| (k.clone(), json!(if k == chosen { 1.0 } else { 0.0 })))
        .collect();
    json!({"type":"choice","choice":chosen,"confidence":1.0,"probabilities":probabilities})
}

#[derive(Default)]
pub(super) struct Mock {
    pub(super) calls: usize,
    pub(super) level: usize,
    pub(super) malformed: bool,
    pub(super) edit: Option<PathBuf>,
    pub(super) requests: Vec<Value>,
}
impl transport::Evaluator for Mock {
    fn evaluate(&mut self, request: &Value) -> anyhow::Result<Value> {
        self.calls += 1;
        self.requests
            .push(requests::provider_request(request).into_owned());
        if let Some(path) = &self.edit {
            std::fs::write(path, "fn changed() {}")?;
        }
        if self.malformed {
            return Ok(json!({"answers":{}}));
        }
        Ok(answer(request, self.level))
    }
}

pub(super) fn session<'a>(
    options: &'a CheckArgs,
    context: &'a ConfigContext,
    store: &'a storage::Store,
    evaluator: &'a mut dyn transport::Evaluator,
) -> evaluate::Session<'a> {
    evaluate::Session {
        args: options,
        context,
        store,
        evaluator,
        requests: 0,
        paid_input_tokens: 0,
        paid_output_tokens: 0,
        budget: token_budget::TokenBudget::default(),
        observed: (0, 0),
    }
}

/// The selected inputs and the first snapshot of a check, before evaluation.
pub(super) fn snapshot(
    project: &Project,
    options: &CheckArgs,
) -> (Vec<inventory::Input>, schema::Report) {
    let context = project.context();
    let scope = inventory::scope(options, &context).unwrap();
    let inputs = inventory::collect(options, &context, &scope).unwrap();
    let report = evaluate::snapshot(
        &inputs,
        &Default::default(),
        options,
        evaluate::SnapshotContext {
            root: &project.0,
            generation: 1,
            requests: 0,
        },
    );
    (inputs, report)
}

pub(super) fn run(
    project: &Project,
    options: &CheckArgs,
    mock: &mut impl transport::Evaluator,
) -> schema::Report {
    let context = project.context();
    let (inputs, mut report) = snapshot(project, options);
    let store = storage::Store::open(&project.0).unwrap();
    session(options, &context, &store, mock)
        .evaluate(&inputs, &mut report)
        .unwrap();
    gate::settle(&project.0, &mut report, options).unwrap();
    report
}

#[test]
fn unchanged_files_are_answered_from_cache_without_api_calls() {
    let project = Project::new();
    project.write("a.rs", &function("a"));
    project.write("b.rs", &function("b"));
    let options = args();
    let mut mock = Mock::default();
    let first = run(&project, &options, &mut mock);
    assert_eq!(first.api_requests, 2);
    assert_eq!(first.stages["functions"].successful_requests, 2);
    let second = run(&project, &options, &mut mock);
    assert_eq!(second.api_requests, 0);
    assert_eq!(second.paid_input_tokens, 0);
    assert!(second.files.iter().all(|file| file.cached));
    assert!(
        second
            .files
            .iter()
            .all(|f| f.status == schema::Status::Clear)
    );
    project.write("b.rs", &function("b_changed"));
    let third = run(&project, &options, &mut mock);
    assert_eq!(third.api_requests, 1, "only the changed unit is sent again");
}

#[test]
fn a_dry_run_counts_cached_requests_as_free() {
    let project = Project::new();
    project.write("a.rs", &function("a"));
    let preview = |options: &CheckArgs| snapshot(&project, options).1.stages["functions"].clone();
    let mut options = args();
    options.dry_run = true;
    let cold = preview(&options);
    assert_eq!((cold.planned_requests, cold.planned_cached), (1, 0));
    assert!(cold.planned_tokens > 0);
    assert!(
        !project.0.join(".jevgate").exists(),
        "a dry run writes no state"
    );
    options.dry_run = false;
    run(&project, &options, &mut Mock::default());
    options.dry_run = true;
    let warm = preview(&options);
    assert_eq!((warm.planned_requests, warm.planned_cached), (1, 1));
    assert_eq!(warm.planned_tokens, 0, "answered requests cost nothing");
    options.refresh = true;
    assert_eq!(preview(&options).planned_cached, 0);
}

#[test]
fn command_line_definition_is_consistent() {
    use clap::CommandFactory;
    Cli::command().debug_assert();
}

#[test]
fn model_and_refresh_invalidate_cache() {
    let project = Project::new();
    project.write("lib.rs", &function("f"));
    let mut options = args();
    let mut mock = Mock::default();
    run(&project, &options, &mut mock);
    options.model = Some("other-version".into());
    run(&project, &options, &mut mock);
    options.refresh = true;
    run(&project, &options, &mut mock);
    assert_eq!(mock.calls, 3);
}

#[test]
fn malformed_response_and_exhausted_budget_never_pass() {
    let project = two_files();
    let mut options = args();
    options.max_requests = Some(1);
    let mut mock = Mock {
        malformed: true,
        ..Default::default()
    };
    let report = run(&project, &options, &mut mock);
    assert_eq!(mock.calls, 1);
    assert!(!report.complete);
    assert_eq!(gate::exit_code(&report), 2);
    assert!(
        report
            .files
            .iter()
            .all(|f| f.status == schema::Status::Error)
    );
}

#[test]
fn edit_during_request_is_reported_stale() {
    let project = Project::new();
    project.write("lib.rs", &function("initial"));
    let mut mock = Mock {
        edit: Some(project.0.join("lib.rs")),
        ..Default::default()
    };
    let report = run(&project, &args(), &mut mock);
    assert!(!report.complete);
    assert!(report.files[0].error.as_ref().unwrap().contains("stale"));
}

#[test]
fn changed_and_deleted_files_invalidate_snapshot_before_evaluation() {
    let project = Project::new();
    project.write("a.rs", &function("a"));
    project.write("b.rs", &function("b"));
    let options = args();
    let report = run(&project, &options, &mut Mock::default());
    let old = report
        .files
        .into_iter()
        .map(|f| (f.path.clone(), f))
        .collect();
    project.write("a.rs", &function("a_changed"));
    std::fs::remove_file(project.0.join("b.rs")).unwrap();
    let inputs = inventory::collect(&options, &project.context(), &[]).unwrap();
    let next = evaluate::snapshot(
        &inputs,
        &old,
        &options,
        evaluate::SnapshotContext {
            root: &project.0,
            generation: 2,
            requests: 2,
        },
    );
    assert_eq!(next.files.len(), 1);
    assert_eq!(next.files[0].status, schema::Status::Pending);
    assert!(!next.complete);
}

/// A project with two judged functions, `a.rs` and `b.rs`.
fn two_files() -> Project {
    let project = Project::new();
    project.write("a.rs", &function("a"));
    project.write("b.rs", &function("b"));
    project
}
