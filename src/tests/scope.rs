//! What a run judges: unsupported and oversized input is skipped, an empty
//! scope is incomplete, and only selected roles and context are uploaded.
use super::*;

#[test]
fn unparseable_binary_and_unsupported_files_are_skipped_without_blocking_the_run() {
    let project = Project::new();
    project.write("large.rs", &function("too_large"));
    project.write("invalid.rs", "fn broken( {");
    project.write("Main.java", "class Main {\n    void run() {}\n}\n");
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
    for name in ["invalid.rs", "Main.java", "latin1.rs"] {
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
