//! The test harness (projects, scripted answers, runs) and whole-run tests;
//! the gate and baseline tests are in `gating`, what a run judges
//! (unsupported or oversized input, roles, context) in `scope`, and the
//! answer cache in `cache`.
use super::*;
use crate::{config::ConfigContext, options::CheckArgs};
use clap::Parser;
use serde_json::{Value, json};
use std::path::PathBuf;
mod cache;
mod gating;
#[path = "../../tests/support/git.rs"]
pub(super) mod git;
#[path = "../../tests/support/mock_provider.rs"]
pub(super) mod mock_provider;
pub(super) use mock_provider::answer;
mod guards;
mod scope;
#[path = "../../tests/support/temp_dir.rs"]
mod temp_dir;

/// Whether the crate is built from its published package, which holds the
/// source and tests but not the repository's other files (`plugin/`,
/// `npm/`, `site/`, `jevgate.schema.json`): tests that hold those files to
/// the code have nothing to read there. Cargo adds `.cargo_vcs_info.json`
/// only to a package.
pub(super) fn packaged() -> bool {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(".cargo_vcs_info.json")
        .exists()
}

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
    /// Run Git in the project, with a fixed identity and no signing, apart
    /// from the repository running the tests, and return what it printed.
    pub(super) fn git(&self, args: &[&str]) -> String {
        git::run(&self.0, args)
    }
    /// A Git repository holding the project's files as its first commit.
    pub(super) fn commit_all(&self) {
        self.git(&["init", "-q"]);
        self.git(&["add", "-A"]);
        self.git(&["commit", "-qm", "base"]);
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
    a.fail_on = vec![options::FailOn::Mature];
    a
}

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
        gate: None,
    }
}

/// A finding of `rule` (an ID) at `strength`, otherwise as [`finding`].
pub(super) fn finding_of(rule: &str, strength: crate::schema::Strength) -> crate::schema::Finding {
    crate::schema::Finding {
        rule: rule.into(),
        ..finding(strength)
    }
}

/// A complete report of one judged file, `src/lib.rs`, holding `findings`,
/// with the gate applied as `options` set it.
pub(super) fn gated(findings: Vec<crate::schema::Finding>, options: &CheckArgs) -> schema::Report {
    let project = Project::new();
    project.write("src/lib.rs", &function("f"));
    let (_, mut report) = snapshot(&project, options);
    report.files[0].findings = findings;
    report.complete = true;
    gate::evaluate(&mut report, options);
    report
}

/// A function large enough to judge (five body lines).
pub(super) fn function(name: &str) -> String {
    format!(
        "fn {name}(values: &[i32]) -> i32 {{\n    let mut total = 0;\n    for value in values {{\n        total += value;\n    }}\n    let doubled = total * 2;\n    doubled + 1\n}}\n"
    )
}

/// A function longer than twenty lines, whose split can be a consider.
pub(super) fn long_function(name: &str) -> String {
    format!(
        "fn {name}(values: &[i32]) -> i32 {{\n    let mut total = 0;\n    for value in values {{\n        total += value;\n    }}\n    let mut largest = i32::MIN;\n    for value in values {{\n        if *value > largest {{\n            largest = *value;\n        }}\n    }}\n    let mut smallest = i32::MAX;\n    for value in values {{\n        if *value < smallest {{\n            smallest = *value;\n        }}\n    }}\n    let spread = largest - smallest;\n    let doubled = total * 2;\n    doubled + spread + 1\n}}\n"
    )
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
        paid: Default::default(),
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

/// The first snapshot of a project holding one short function, `a`, at
/// `path`, with every rule selected: a report to shape in a test.
pub(super) fn one_function(path: &str) -> schema::Report {
    let project = Project::new();
    project.write(path, &function("a"));
    snapshot(&project, &args()).1
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

/// Answers as a provider that names `model` and reports usage only when `metered`.
struct Answering {
    model: &'static str,
    metered: bool,
}
impl transport::Evaluator for Answering {
    fn evaluate(&mut self, request: &Value) -> anyhow::Result<Value> {
        let mut body = answer(request, 0);
        body["model"] = json!(self.model);
        if !self.metered {
            body.as_object_mut().unwrap().remove("usage");
        }
        Ok(body)
    }
}

#[test]
fn cost_is_priced_by_the_answering_model_and_unknown_without_usage() {
    let project = Project::new();
    project.write("lib.rs", &function("f"));
    let mut options = args();
    options.model = Some("jev-latest".into());
    let metered = Answering {
        model: "jev-1.13.0",
        metered: true,
    };
    let report = run(&project, &options, &mut { metered });
    assert_eq!(report.paid_models, [("jev-1.13.0".to_string(), 10)].into());
    assert!((report.estimated_usd.unwrap() - 10.0 * 0.042 / 1e6).abs() < 1e-15);
    assert!(output::headline(&report).ends_with("· 10 input tokens · ~$0.0000"));
    options.model = Some("typesafe-ai/jev".into());
    let unmetered = Answering {
        model: "typesafe-ai/jev",
        metered: false,
    };
    let report = run(&project, &options, &mut { unmetered });
    assert!(report.complete);
    assert_eq!((report.api_requests, report.unmetered_requests), (1, 1));
    assert_eq!((report.paid_input_tokens, report.estimated_usd), (0, None));
    assert!(output::headline(&report).ends_with("· 0 input tokens · cost unknown"));
    let replay = run(&project, &options, &mut Mock::default());
    assert_eq!(replay.api_requests, 0);
    assert!(
        replay
            .estimated_usd
            .is_some_and(|usd| usd.is_sign_positive() && usd == 0.0)
    );
    assert!(output::headline(&replay).ends_with("· 0 input tokens · ~$0.0000"));
    assert!(replay.files.iter().all(|file| file.cached));
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
