use super::*;
use clap::Parser;
use serde_json::{Value, json};
use std::path::PathBuf;
#[path = "../tests/support/temp_dir.rs"]
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

pub(super) fn run(
    project: &Project,
    options: &CheckArgs,
    mock: &mut impl transport::Evaluator,
) -> schema::Report {
    let context = project.context();
    let scope = inventory::scope(options, &context).unwrap();
    let inputs = inventory::collect(options, &context, &scope).unwrap();
    let mut report = evaluate::snapshot(
        &inputs,
        &Default::default(),
        options,
        evaluate::SnapshotContext {
            root: &project.0,
            generation: 1,
            requests: 0,
        },
    );
    let store = storage::Store::open(&project.0).unwrap();
    session(options, &context, &store, mock)
        .evaluate(&inputs, &mut report)
        .unwrap();
    gate::settle(&project.0, &mut report, &options.fail_on).unwrap();
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
fn model_and_refresh_invalidate_cache() {
    let project = Project::new();
    project.write("lib.rs", &function("f"));
    let mut options = args();
    let mut mock = Mock::default();
    run(&project, &options, &mut mock);
    options.model = "other-version".into();
    run(&project, &options, &mut mock);
    options.refresh = true;
    run(&project, &options, &mut mock);
    assert_eq!(mock.calls, 3);
}

#[test]
fn gate_fails_only_on_the_configured_results() {
    let project = Project::new();
    project.write("lib.rs", &function("f"));
    let mut options = args();
    // Mock level, report status, the default gate's exit code, and the gate that fails it.
    let cases = [
        (2, "review", 1, options::FailOn::None, 0),
        (4, "consider", 0, options::FailOn::Consider, 1),
        (1, "note", 0, options::FailOn::Consider, 0),
        (3, "uncertain", 0, options::FailOn::Uncertain, 1),
    ];
    for (level, status, default, stricter, stricter_code) in cases {
        options.refresh = true;
        options.fail_on = vec![options::FailOn::Review];
        let mut mock = Mock {
            level,
            ..Default::default()
        };
        let report = run(&project, &options, &mut mock);
        assert_eq!(report.status, status);
        assert_eq!(gate::exit_code(&report), default, "{status} by default");
        assert!(!report.acceptance_evaluated);
        options.refresh = false;
        options.fail_on = vec![stricter];
        let report = run(&project, &options, &mut mock);
        assert_eq!(
            gate::exit_code(&report),
            stricter_code,
            "{status} with {stricter:?}"
        );
    }
}

#[test]
fn baselined_findings_do_not_fail_the_gate_but_new_ones_do() {
    let project = Project::new();
    project.write("lib.rs", &function("f"));
    let options = args();
    let mut review = Mock {
        level: 2,
        ..Default::default()
    };
    let report = run(&project, &options, &mut review);
    storage::Store::open(&project.0)
        .unwrap()
        .publish(&report)
        .unwrap();
    let (path, count) = gate::write_baseline(&project.0).unwrap();
    assert_eq!((path, count), (project.0.join(gate::BASELINE_FILE), 1));
    let report = run(&project, &options, &mut review);
    assert!(report.files[0].findings[0].baselined);
    assert_eq!(gate::exit_code(&report), 0);
    assert_eq!(report.gate.as_ref().unwrap().baselined_findings, 1);
    project.write("other.rs", &function("g"));
    let report = run(&project, &options, &mut review);
    assert_eq!(gate::exit_code(&report), 1, "a new finding still fails");
}

#[test]
fn malformed_response_and_exhausted_budget_never_pass() {
    let project = Project::new();
    project.write("a.rs", &function("a"));
    project.write("b.rs", &function("b"));
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

#[test]
fn unparseable_binary_and_unsupported_files_are_skipped_without_blocking_the_run() {
    let project = Project::new();
    project.write("large.rs", &function("too_large"));
    project.write("invalid.rs", "fn broken( {");
    project.write("main.go", "package main\n\nfunc main() {}\n");
    project.write("ok.rs", &function("ok"));
    std::fs::write(project.0.join("latin1.rs"), b"fn caf\xe9() {}\n").unwrap();
    let mut options = args();
    options.max_file_bytes = 160;
    assert!(function("ok").len() <= 160 && function("too_large").len() > 160);
    let mut mock = Mock::default();
    let report = run(&project, &options, &mut mock);
    assert_eq!(mock.calls, 1);
    assert!(report.complete, "{:?}", report.files);
    let file = |name: &str| {
        report
            .files
            .iter()
            .find(|f| f.path.ends_with(name))
            .unwrap()
    };
    assert_eq!(file("ok.rs").status, schema::Status::Clear);
    for name in ["invalid.rs", "main.go", "latin1.rs"] {
        assert_eq!(file(name).status, schema::Status::Skipped, "{name}");
        assert!(
            file(name).error.as_ref().unwrap().contains("not judged"),
            "{name}"
        );
    }
    let large = file("large.rs");
    assert_eq!(large.status, schema::Status::NeedsContext);
    assert!(large.dimensions.is_empty() && large.findings.is_empty());
    let reason = &large.classification.as_ref().unwrap().reason;
    assert!(
        reason.contains("too_large") && reason.contains("160-byte read cap"),
        "{reason}"
    );
    assert_eq!(report.status, "needs-context");
    assert_eq!(gate::exit_code(&report), 0);
}

#[test]
fn a_function_too_large_for_one_request_is_needs_context_and_not_sent() {
    let project = Project::new();
    let mut body = String::from("fn huge() -> usize {\n    let mut total = 0;\n");
    let mut index = 0usize;
    while body.len() < 200_000 {
        body.push_str(&format!("    total += {index} * {index};\n"));
        index += 1;
    }
    body.push_str("    total\n}\n");
    project.write("huge.rs", &format!("{body}\n{}", function("small")));
    let mut options = args();
    options.max_file_bytes = 1_048_576;
    let mut mock = Mock::default();
    let report = run(&project, &options, &mut mock);
    assert!(report.complete);
    let huge = &report.files[0];
    let dimension = &huge.dimensions["function_simplification"];
    assert_eq!(dimension.units.needs_context, 1);
    assert_eq!(dimension.units.judged, 1);
    assert_eq!(dimension.status, schema::Status::NeedsContext);
    assert!(
        mock.requests
            .iter()
            .all(|r| !r["state"]["functions"].to_string().contains("fn huge"))
    );
    assert_eq!(huge.status, schema::Status::NeedsContext);
}

#[test]
fn empty_scope_is_incomplete() {
    let project = Project::new();
    let report = run(&project, &args(), &mut Mock::default());
    assert!(!report.complete);
    assert!(!report.acceptance_evaluated);
}

#[test]
fn storage_has_single_writer_and_releases_lock() {
    let project = Project::new();
    let store = storage::Store::open(&project.0).unwrap();
    assert!(storage::Store::open(&project.0).is_err());
    drop(store);
    assert!(storage::Store::open(&project.0).is_ok());
}

#[test]
fn credential_parser_does_not_execute_shell() {
    let project = Project::new();
    project.write(
        ".env",
        "export TYPESAFE_API_KEY='literal$(do-not-execute)'\n",
    );
    assert_eq!(
        transport::key_from_file(&project.0.join(".env")).unwrap(),
        "literal$(do-not-execute)"
    );
}

#[test]
fn fixture_and_generated_roles_are_not_uploaded() {
    let project = Project::new();
    std::fs::create_dir(project.0.join("fixtures")).unwrap();
    project.write("fixtures/sample.rs", "fn fixture() {}");
    project.write("database.types.ts", "export type Db = string;");
    let mut context = project.context();
    context.config.generated = vec!["database.types.ts".into()];
    let inputs = inventory::collect(&args(), &context, &[]).unwrap();
    for path in ["fixtures/sample.rs", "database.types.ts"] {
        let input = inputs
            .iter()
            .find(|i| i.result.path == std::path::Path::new(path))
            .unwrap();
        assert_eq!(input.result.status, schema::Status::Skipped, "{path}");
        assert!(input.source.is_none(), "{path}");
    }
}

#[cfg(unix)]
#[test]
fn storage_rejects_symlinked_state_directory() {
    let project = Project::new();
    let outside = Project::new();
    std::os::unix::fs::symlink(&outside.0, project.0.join(".jevgate")).unwrap();
    assert!(storage::Store::open(&project.0).is_err());
    assert!(!outside.0.join("jev").exists());
}

#[test]
fn context_limits_and_visibility_are_enforced_without_api_calls() {
    let project = Project::new();
    project.write("lib.rs", "fn f() {}");
    project.write("contract.md", "a contract");
    project.write(".env", "TYPESAFE_API_KEY=secret");
    let mut options = args();
    options.context.push(".env".into());
    assert!(inventory::collect(&options, &project.context(), &[]).is_err());
    options.context = vec!["contract.md".into()];
    options.max_context_bytes = 2;
    assert!(inventory::collect(&options, &project.context(), &[]).is_err());
    options.max_context_bytes = 100;
    std::fs::remove_file(project.0.join("contract.md")).unwrap();
    assert!(inventory::collect(&options, &project.context(), &[]).is_err());
}
