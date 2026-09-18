use super::*;
use clap::Parser;
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT: AtomicUsize = AtomicUsize::new(0);
pub(super) struct Project(pub(super) PathBuf);
impl Project {
    pub(super) fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "jev-unit-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    pub(super) fn write(&self, name: &str, text: &str) {
        std::fs::create_dir_all(self.0.join(name).parent().unwrap()).unwrap();
        std::fs::write(self.0.join(name), text).unwrap();
    }
    pub(super) fn context(&self) -> ConfigContext {
        ConfigContext {
            invocation_dir: self.0.clone(),
            root: self.0.clone(),
            config: Default::default(),
        }
    }
}
impl Drop for Project {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[derive(Parser)]
struct TestCli {
    #[command(flatten)]
    args: CheckArgs,
}
pub(super) fn args() -> CheckArgs {
    {
        let mut a = TestCli::parse_from(["test"]).args;
        a.rules = crate::maintainability::KEYS
            .iter()
            .map(|k| (*k).into())
            .collect();
        a
    }
}

pub(super) fn answer(request: &Value, level: usize, missing: f64) -> Value {
    let answers=request["questions"].as_object().unwrap().iter().map(|(name,q)| {
        let keys=q["criteria"].as_object().unwrap();
        let preferred=if name.ends_with("_location") {"none"} else if missing>=0.5 {"context"} else if level>=2 {"review"} else {"clear"};
        let chosen=if keys.contains_key(preferred) {preferred} else if keys.contains_key("idiom") {"idiom"} else {keys.keys().next().unwrap()};
        let probabilities: serde_json::Map<_,_>=keys.keys().map(|k|(k.clone(),json!(if k==chosen {1.0} else {0.0}))).collect();
        (name.clone(),json!({"type":"choice","choice":chosen,"confidence":1.0,"probabilities":probabilities}))
    }).collect::<serde_json::Map<_,_>>();
    json!({"model":request["model"],"answers":answers,"usage":{"input_tokens":10,"output_tokens":0}})
}

#[derive(Default)]
pub(super) struct Mock {
    pub(super) calls: usize,
    pub(super) level: usize,
    pub(super) missing: f64,
    pub(super) malformed: bool,
    pub(super) edit: Option<PathBuf>,
}
impl transport::Evaluator for Mock {
    fn evaluate(&mut self, request: &Value) -> anyhow::Result<Value> {
        self.calls += 1;
        assert!(request["state"]["file"]["source"].is_string());
        if let Some(path) = &self.edit {
            std::fs::write(path, "fn changed() {}")?;
        }
        if self.malformed {
            return Ok(json!({"answers":{}}));
        }
        Ok(answer(request, self.level, self.missing))
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
    let mut session = evaluate::Session {
        args: options,
        context: &context,
        store: &store,
        evaluator: mock,
        requests: 0,
        paid_input_tokens: 0,
        paid_output_tokens: 0,
    };
    session.evaluate(&inputs, &mut report).unwrap();
    report
}

#[test]
fn model_and_refresh_invalidate_cache() {
    let project = Project::new();
    project.write("lib.rs", "fn f() {}");
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
fn advisory_reviews_and_abstentions_do_not_claim_acceptance() {
    let project = Project::new();
    project.write("lib.rs", "fn f() {}");
    let mut options = args();
    options.refresh = true;
    let mut mock = Mock {
        level: 2,
        ..Default::default()
    };
    let report = run(&project, &options, &mut mock);
    assert_eq!(report.status, "review");
    assert_eq!(outcome(&report), 0);
    assert!(!report.acceptance_evaluated);
    mock.missing = 0.8;
    let report = run(&project, &options, &mut mock);
    assert_eq!(report.files[0].status, schema::Status::NeedsContext);
    assert_eq!(outcome(&report), 0);
}

#[test]
fn malformed_response_and_exhausted_budget_never_pass() {
    let project = Project::new();
    project.write("a.rs", "fn a() {}");
    project.write("b.rs", "fn b() {}");
    let mut options = args();
    options.max_requests = Some(1);
    let mut mock = Mock {
        malformed: true,
        ..Default::default()
    };
    let report = run(&project, &options, &mut mock);
    assert_eq!(mock.calls, 1);
    assert!(!report.complete);
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
    project.write("lib.rs", "fn initial() {}");
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
    project.write("a.rs", "fn a() {}");
    project.write("b.rs", "fn b() {}");
    let options = args();
    let report = run(&project, &options, &mut Mock::default());
    let old = report
        .files
        .into_iter()
        .map(|f| (f.path.clone(), f))
        .collect();
    project.write("a.rs", "fn a_changed() {}");
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
fn oversized_and_invalid_source_never_reach_api() {
    let project = Project::new();
    project.write("large.rs", "fn too_large() {}");
    project.write("invalid.rs", "fn broken( {");
    let mut options = args();
    options.max_file_bytes = 15;
    let mut mock = Mock::default();
    let report = run(&project, &options, &mut mock);
    assert_eq!(mock.calls, 0);
    assert!(!report.complete);
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
    let fixture = inputs
        .iter()
        .find(|i| i.result.path == std::path::Path::new("fixtures/sample.rs"))
        .unwrap();
    assert_eq!(fixture.result.status, schema::Status::Skipped);
    assert!(fixture.source.is_none());
    let generated = inputs
        .iter()
        .find(|i| i.result.path == std::path::Path::new("database.types.ts"))
        .unwrap();
    assert_eq!(generated.result.status, schema::Status::Skipped);
    assert!(generated.source.is_none());
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
