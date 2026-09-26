//! The gate: failure levels per rule and path, and the baseline of accepted findings.
use super::*;

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
fn a_rule_level_fails_the_gate_only_for_that_rule() {
    let project = Project::new();
    project.write("lib.rs", &function("f"));
    let mut options = args();
    let mut mock = Mock {
        level: 4,
        ..Default::default()
    };
    let report = run(&project, &options, &mut mock);
    assert_eq!(
        (report.status.as_str(), gate::exit_code(&report)),
        ("consider", 0)
    );
    for (rule, code) in [
        (crate::catalog::SHARED_LOGIC, 0),
        (crate::catalog::FUNCTION_SIMPLIFICATION, 1),
    ] {
        options.rule_fail_on =
            std::collections::BTreeMap::from([(rule.to_string(), vec![options::FailOn::Consider])]);
        let report = run(&project, &options, &mut mock);
        assert_eq!(gate::exit_code(&report), code, "consider on {rule}");
    }
}

#[test]
fn a_scope_makes_its_paths_report_only_while_other_files_gate() {
    let project = Project::new();
    project.write("src/lib.rs", &function("f"));
    project.write("scripts/tool.rs", &function("g"));
    let mut options = args();
    options.fail_on = vec![options::FailOn::Consider];
    let mut mock = Mock {
        level: 4,
        ..Default::default()
    };
    let scripts = |levels: Vec<options::FailOn>| options::PathLevels {
        paths: vec!["scripts/**".into()],
        matcher: crate::boundary::globs(&["scripts/**".into()]).unwrap(),
        rules: crate::catalog::keys()
            .into_iter()
            .map(|key| (key.to_string(), levels.clone()))
            .collect(),
    };
    options.path_fail_on = vec![scripts(vec![options::FailOn::None])];
    let report = run(&project, &options, &mut mock);
    let gate = report.gate.as_ref().unwrap();
    assert_eq!(gate.reasons, ["1 new consider finding"], "only src/lib.rs");
    options.paths = vec!["scripts".into()];
    let report = run(&project, &options, &mut mock);
    assert_eq!(gate::exit_code(&report), 0, "tooling findings only report");
    options.path_fail_on = vec![scripts(vec![options::FailOn::Uncertain])];
    mock.level = 3;
    options.refresh = true;
    let report = run(&project, &options, &mut mock);
    assert_eq!(
        gate::exit_code(&report),
        1,
        "uncertain fails inside the scope"
    );
}

/// Save a report as the last check, as `jevgate check` does.
fn publish(project: &Project, report: &schema::Report) {
    storage::Store::open(&project.0)
        .unwrap()
        .publish(report)
        .unwrap();
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
    publish(&project, &run(&project, &options, &mut review));
    let written = baseline::write(&project.0, false, None).unwrap();
    assert_eq!(
        (written.path, written.accepted),
        (project.0.join(baseline::BASELINE_FILE), 1)
    );
    let report = run(&project, &options, &mut review);
    assert!(report.files[0].findings[0].baselined);
    assert_eq!(gate::exit_code(&report), 0);
    assert_eq!(report.gate.as_ref().unwrap().baselined_findings, 1);
    project.write("other.rs", &function("g"));
    let report = run(&project, &options, &mut review);
    assert_eq!(gate::exit_code(&report), 1, "a new finding still fails");
}

#[test]
fn a_merged_baseline_keeps_accepted_findings_for_files_the_check_did_not_cover() {
    let project = two_files();
    let mut options = args();
    let mut review = Mock {
        level: 2,
        ..Default::default()
    };
    publish(&project, &run(&project, &options, &mut review));
    assert_eq!(
        baseline::write(&project.0, false, None).unwrap().accepted,
        2
    );
    // A check of `a.rs` alone, as a `--base` run that only changed it.
    project.write("a.rs", &function("a2"));
    options.paths = vec!["a.rs".into()];
    publish(&project, &run(&project, &options, &mut review));
    let merged = baseline::write(&project.0, true, None).unwrap();
    assert_eq!((merged.accepted, merged.kept), (1, 1));
    options.paths.clear();
    let report = run(&project, &options, &mut review);
    assert!(report.files.iter().all(|f| f.findings[0].baselined));
    assert_eq!(gate::exit_code(&report), 0);
    // Without merge the same partial check drops `b.rs`.
    options.paths = vec!["a.rs".into()];
    publish(&project, &run(&project, &options, &mut review));
    assert_eq!(
        baseline::write(&project.0, false, None).unwrap().accepted,
        1
    );
    options.paths.clear();
    assert_eq!(gate::exit_code(&run(&project, &options, &mut review)), 1);
}

#[test]
fn baseline_reasons_are_marked_counted_and_kept_across_rewrites() {
    use options::Disposition::{Later, Wrong};
    let project = two_files();
    let options = args();
    let mut review = Mock {
        level: 2,
        ..Default::default()
    };
    publish(&project, &run(&project, &options, &mut review));
    baseline::write(&project.0, false, Some(Later)).unwrap();
    let rule = "maintainability/function-simplification";
    let counts = baseline::stats(&project.0).unwrap();
    assert_eq!(
        (counts[rule].later, counts[rule].wrong_rate),
        (2, Some(0.0))
    );
    assert_eq!(
        baseline::mark(&project.0, Wrong, &["b.rs:1".into()], &[]).unwrap(),
        1
    );
    assert!(baseline::mark(&project.0, Wrong, &["c.rs".into()], &[]).is_err());
    assert!(
        baseline::mark(
            &project.0,
            Wrong,
            &["a.rs".into()],
            &[crate::catalog::INJECTION]
        )
        .is_err(),
        "the rule filter excludes it"
    );
    let counts = baseline::stats(&project.0).unwrap();
    assert_eq!((counts[rule].later, counts[rule].wrong), (1, 1));
    assert_eq!(counts[rule].wrong_rate, Some(0.5));
    // Rewriting the baseline keeps each accepted finding's reason.
    baseline::write(&project.0, false, None).unwrap();
    assert_eq!(baseline::stats(&project.0).unwrap()[rule].wrong, 1);
    assert!(baseline::stats_table(&counts).contains("50%"));
}

#[test]
fn an_allow_comment_accepts_a_finding_only_with_a_reason() {
    let project = Project::new();
    let options = args();
    let mut mock = Mock {
        level: 2,
        ..Default::default()
    };
    for (comment, accepted) in [
        (
            "// jevgate: allow(maintainability) kept as the protocol spells it\n",
            true,
        ),
        ("// jevgate: allow(maintainability)\n", false),
        (
            "// jevgate: allow(security) kept as the protocol spells it\n",
            false,
        ),
    ] {
        project.write("lib.rs", &format!("{comment}{}", function("f")));
        let report = run(&project, &options, &mut mock);
        let finding = &report.files[0].findings[0];
        assert_eq!(finding.suppressed.is_some(), accepted, "{comment}");
        assert_eq!(
            gate::exit_code(&report),
            if accepted { 0 } else { 1 },
            "{comment}"
        );
        let gate = report.gate.as_ref().unwrap();
        assert_eq!(gate.suppressed_findings, usize::from(accepted));
        assert_eq!(
            finding.message.contains("ignored: it gives no reason"),
            comment.ends_with(")\n"),
            "{comment}"
        );
    }
}
