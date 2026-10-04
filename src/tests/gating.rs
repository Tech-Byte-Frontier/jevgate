//! The gate: failure levels per rule and path, and the baseline of accepted findings.
use super::*;

#[test]
fn gate_fails_only_on_the_configured_results() {
    let project = Project::new();
    project.write("lib.rs", &long_function("f"));
    let mut options = args();
    // Mock level, report status, the default gate's exit code, and the gate that fails it.
    let cases = [
        (2, "review", 1, options::FailOn::None, 0),
        (1, "clear", 0, options::FailOn::Review, 0),
    ];
    for (level, status, default, stricter, stricter_code) in cases {
        options.refresh = true;
        options.fail_on = vec![options::FailOn::Mature];
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
    // A function too large to send leaves its file undecided: only
    // `uncertain` fails on it.
    let large = Project::new();
    large.write("huge.rs", &too_large());
    options.max_file_bytes = 1_048_576;
    options.fail_on = vec![options::FailOn::Mature];
    let report = run(&large, &options, &mut Mock::default());
    assert_eq!(
        (report.status.as_str(), gate::exit_code(&report)),
        ("needs-context", 0)
    );
    options.fail_on = vec![options::FailOn::Uncertain];
    let report = run(&large, &options, &mut Mock::default());
    assert_eq!(gate::exit_code(&report), 1);
}

/// One function too large for any request, so its unit needs context.
fn too_large() -> String {
    let mut body = String::from("fn huge() -> usize {\n    let mut total = 0;\n");
    let mut index = 0usize;
    while body.len() < 200_000 {
        body.push_str(&format!("    total += {index} * {index};\n"));
        index += 1;
    }
    body + "    total\n}\n"
}

#[test]
fn a_rule_level_fails_the_gate_only_for_that_rule() {
    let project = Project::new();
    project.write("lib.rs", &long_function("f"));
    let mut options = args();
    options.fail_on = vec![options::FailOn::None];
    let mut mock = Mock {
        level: 2,
        ..Default::default()
    };
    let report = run(&project, &options, &mut mock);
    assert_eq!(
        (report.status.as_str(), gate::exit_code(&report)),
        ("review", 0)
    );
    for (rule, code) in [
        (crate::catalog::SHARED_LOGIC, 0),
        (crate::catalog::FUNCTION_SIMPLIFICATION, 1),
    ] {
        options.rule_fail_on =
            std::collections::BTreeMap::from([(rule.to_string(), vec![options::FailOn::Review])]);
        let report = run(&project, &options, &mut mock);
        assert_eq!(gate::exit_code(&report), code, "review on {rule}");
    }
}

/// A `[[scope]]` that sets `levels` for every rule in the files `glob` matches.
fn every_rule_in(glob: &str, levels: Vec<options::FailOn>) -> options::PathLevels {
    options::PathLevels {
        paths: vec![glob.into()],
        matcher: crate::boundary::globs(&[glob.into()]).unwrap(),
        rules: crate::catalog::keys()
            .into_iter()
            .map(|key| (key.to_string(), levels.clone()))
            .collect(),
    }
}

#[test]
fn a_scope_makes_its_paths_report_only_while_other_files_gate() {
    let project = Project::new();
    project.write("src/lib.rs", &long_function("f"));
    project.write("scripts/tool.rs", &long_function("g"));
    let mut options = args();
    options.rules = vec![crate::catalog::FUNCTION_SIMPLIFICATION.into()];
    options.fail_on = vec![options::FailOn::Review];
    let mut mock = Mock {
        level: 2,
        ..Default::default()
    };
    let scripts = |levels| every_rule_in("scripts/**", levels);
    options.path_fail_on = vec![scripts(vec![options::FailOn::None])];
    let report = run(&project, &options, &mut mock);
    let gate = report.gate.as_ref().unwrap();
    assert_eq!(gate.reasons, ["1 new review finding"], "only src/lib.rs");
    options.paths = vec!["scripts".into()];
    let report = run(&project, &options, &mut mock);
    assert_eq!(gate::exit_code(&report), 0, "tooling findings only report");
    project.write("scripts/tool.rs", &too_large());
    options.max_file_bytes = 1_048_576;
    options.path_fail_on = vec![scripts(vec![options::FailOn::Uncertain])];
    let report = run(&project, &options, &mut mock);
    assert_eq!(
        gate::exit_code(&report),
        1,
        "uncertain fails inside the scope"
    );
}

/// `baseline mark REASON TARGET` without a note, for findings of `rules`.
fn mark(
    project: &Project,
    reason: options::Disposition,
    target: String,
    rules: &[&str],
) -> anyhow::Result<usize> {
    baseline::mark(
        &project.0,
        &baseline::Mark {
            reason,
            note: None,
            targets: &[target],
            rules,
        },
    )
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
    project.write("lib.rs", &long_function("f"));
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
fn a_merged_baseline_after_a_check_of_changed_lines_keeps_the_rest_of_its_files() {
    let project = Project::new();
    let source = format!("{}{}", function("a"), other_function("b"));
    project.write("lib.rs", &source);
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
    project.commit_all();
    // The change touches `a` alone: `b`'s accepted finding was not judged.
    project.write("lib.rs", &source.replacen("doubled + 1", "doubled + 2", 1));
    options.base = Some("HEAD".into());
    let report = run(&project, &options, &mut review);
    assert_eq!(report.scope, schema::Scope::ChangedLines);
    publish(&project, &report);
    let merged = baseline::write(&project.0, true, None).unwrap();
    assert_eq!((merged.accepted, merged.kept), (1, 2));
    options.base = None;
    let report = run(&project, &options, &mut review);
    assert!(report.files[0].findings.iter().all(|f| f.baselined));
    assert_eq!(gate::exit_code(&report), 0);
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
    assert_eq!(mark(&project, Wrong, "b.rs:1".into(), &[]).unwrap(), 1);
    assert!(mark(&project, Wrong, "c.rs".into(), &[]).is_err());
    for empty in ["", " ", "/"] {
        assert!(
            mark(&project, Wrong, empty.into(), &[]).is_err(),
            "{empty:?} names no finding"
        );
    }
    assert!(
        mark(&project, Wrong, "a.rs".into(), &[crate::catalog::INJECTION]).is_err(),
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
fn a_mark_note_survives_rewrites_merges_and_marking_again() {
    use options::Disposition::{Later, Wrong};
    let project = two_files();
    let mut options = args();
    let mut review = Mock {
        level: 2,
        ..Default::default()
    };
    publish(&project, &run(&project, &options, &mut review));
    let noted = |note: Option<Option<String>>, reason| {
        baseline::mark(
            &project.0,
            &baseline::Mark {
                reason,
                note,
                targets: &["b.rs:1".into()],
                rules: &[],
            },
        )
        .unwrap()
    };
    let note_of_b = || {
        let listed = baseline::list(&project.0, &[], &[]).unwrap();
        let b = listed.iter().find(|l| l.path.ends_with("b.rs")).unwrap();
        (b.reason, b.note.clone())
    };
    // Dismissed from the last check with a note, before any baseline.
    assert_eq!(noted(Some(Some("#192".into())), Later), 1);
    assert_eq!(note_of_b(), (Some(Later), Some("#192".into())));
    // Rewriting the baseline from the whole check keeps it.
    baseline::write(&project.0, false, None).unwrap();
    assert_eq!(note_of_b(), (Some(Later), Some("#192".into())));
    // So does a merge after a check of `a.rs` alone, and one of `b.rs`.
    for checked in ["a.rs", "b.rs"] {
        options.paths = vec![checked.into()];
        publish(&project, &run(&project, &options, &mut review));
        baseline::write(&project.0, true, None).unwrap();
        assert_eq!(note_of_b(), (Some(Later), Some("#192".into())), "{checked}");
    }
    // Marked again without a note, it keeps the note; an empty one clears it.
    assert_eq!(noted(None, Wrong), 1);
    assert_eq!(note_of_b(), (Some(Wrong), Some("#192".into())));
    assert_eq!(noted(Some(baseline::note(" ").unwrap()), Wrong), 1);
    assert_eq!(note_of_b(), (Some(Wrong), None));
}

#[test]
fn a_mark_note_is_one_short_line() {
    assert_eq!(
        baseline::note("  see #192 ").unwrap(),
        Some("see #192".into())
    );
    assert_eq!(baseline::note("").unwrap(), None);
    assert!(baseline::note("two\nlines").is_err());
    assert!(baseline::note(&"x".repeat(baseline::NOTE_CHARS)).is_ok());
    assert!(baseline::note(&"x".repeat(baseline::NOTE_CHARS + 1)).is_err());
}

#[test]
fn a_finding_of_the_last_check_is_dismissed_by_its_line_or_fingerprint() {
    use options::Disposition::Wrong;
    let project = two_files();
    let options = args();
    let mut review = Mock {
        level: 2,
        ..Default::default()
    };
    let report = run(&project, &options, &mut review);
    publish(&project, &report);
    assert!(!project.0.join(baseline::BASELINE_FILE).exists());
    assert!(
        mark(&project, Wrong, "b.rs".into(), &[]).is_err(),
        "a path dismisses no finding nobody read"
    );
    assert_eq!(mark(&project, Wrong, "b.rs:1".into(), &[]).unwrap(), 1);
    let rule = "maintainability/function-simplification";
    assert_eq!(baseline::stats(&project.0).unwrap()[rule].wrong, 1);
    let fingerprint = report
        .files
        .iter()
        .find(|f| f.path.ends_with("a.rs"))
        .unwrap()
        .findings[0]
        .fingerprint
        .clone();
    assert_eq!(
        mark(&project, Wrong, fingerprint[..8].to_string(), &[]).unwrap(),
        1
    );
    let again = run(&project, &options, &mut review);
    assert!(
        again
            .files
            .iter()
            .flat_map(|f| &f.findings)
            .all(|f| f.accepted()),
        "both dismissed"
    );
    assert!(again.gate.unwrap().passed);
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

const SIMPLIFICATION: &str = "maintainability/function-simplification";
const SHARED_LOGIC: &str = "maintainability/shared-logic";

/// How the gate counted each finding of a report, in order.
fn gates(report: &schema::Report) -> Vec<Option<schema::Gating>> {
    report.files[0].findings.iter().map(|f| f.gate).collect()
}

#[test]
fn the_default_gate_fails_only_on_mature_rule_levels() {
    use schema::{Gating::*, Strength::*};
    let accepted = schema::Finding {
        baselined: true,
        ..finding_of(SIMPLIFICATION, Review)
    };
    let findings = vec![
        finding_of(SIMPLIFICATION, Review),
        finding_of(SIMPLIFICATION, Consider),
        finding_of(SHARED_LOGIC, Review),
        finding_of(SIMPLIFICATION, Note),
        accepted,
    ];
    let report = gated(findings.clone(), &args());
    assert_eq!(
        gates(&report),
        [Some(Fails), Some(Measuring), Some(Measuring), None, None]
    );
    assert_eq!(
        report.gate.as_ref().unwrap().reasons,
        ["1 new review finding"]
    );
    assert_eq!(gate::exit_code(&report), 1);
    let measured = gated(findings[1..].to_vec(), &args());
    let gate = measured.gate.as_ref().unwrap();
    assert!(gate.passed, "reviews still being measured are reported");
    assert_eq!((gate.new_findings, gate.baselined_findings), (2, 1));
    assert_eq!(gate::exit_code(&measured), 0);
}

#[test]
fn every_finding_but_a_note_carries_its_rule_and_levels_precision() {
    use crate::maturity::Labels;
    use schema::Strength::*;
    let labels = |right, labeled| Some(Labels { right, labeled });
    let accepted = schema::Finding {
        baselined: true,
        ..finding_of(SIMPLIFICATION, Review)
    };
    let report = gated(
        vec![
            finding_of(SIMPLIFICATION, Review),
            reported("documentation/comments", Consider),
            finding_of("tests/laws", Review),
            finding_of(SIMPLIFICATION, Note),
            accepted,
        ],
        &args(),
    );
    let precision: Vec<_> = report.files[0]
        .findings
        .iter()
        .map(|f| f.precision)
        .collect();
    assert_eq!(
        precision,
        [
            labels(20, 23),
            labels(39, 72),
            labels(0, 0),
            None,
            labels(20, 23)
        ]
    );
    let json = serde_json::to_value(&report.files[0].findings).unwrap();
    assert_eq!(json[0]["precision"], json!({"right": 20, "labeled": 23}));
    assert!(
        json[3].get("precision").is_none(),
        "a note is never labeled"
    );
    let mut out = Vec::new();
    output::agent(&mut out, &report, true, output::Style::PLAIN).unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(
        text.contains(
            "(fails the gate) Copies: 50% alike,\nsee `b` Right 87% of the time (23 labels).\n"
        ),
        "{text}"
    );
    assert!(
        text.contains("[tests/laws] Copies: 50% alike,\nsee `b` Not yet measured: labeled only on Bend 2 projects, which the maturity table leaves out.\n"),
        "{text}"
    );
    assert!(!text.contains("0.9"), "no probability: {text}");
}

#[test]
fn an_explicit_level_replaces_the_default_exactly_as_it_says() {
    use schema::{Gating::*, Strength::*};
    let findings = vec![
        finding_of(SIMPLIFICATION, Review),
        finding_of(SHARED_LOGIC, Review),
        reported(SHARED_LOGIC, Consider),
    ];
    // At one level, `review` fails on every finding.
    let mut options = args();
    options.fail_on = vec![options::FailOn::Review];
    let report = gated(findings.clone(), &options);
    assert_eq!(gates(&report), [Some(Fails); 3]);
    assert_eq!(report.gate.unwrap().reasons, ["3 new review findings"]);
    // A level for one rule leaves the others at the default.
    let mut options = args();
    options.rule_fail_on = std::collections::BTreeMap::from([(
        crate::catalog::SHARED_LOGIC.to_string(),
        vec![options::FailOn::None],
    )]);
    let report = gated(findings.clone(), &options);
    assert_eq!(
        gates(&report),
        [Some(Fails), Some(Advisory), Some(Advisory)]
    );
    // A scope's report level covers the mature rule too.
    let mut options = args();
    options.path_fail_on = vec![every_rule_in("src/**", vec![options::FailOn::None])];
    let report = gated(findings, &options);
    assert_eq!(gates(&report), [Some(Advisory); 3]);
    assert_eq!(gate::exit_code(&report), 0);
}

#[test]
fn agent_text_marks_what_fails_and_says_why_the_rest_did_not() {
    use schema::Strength::*;
    let report = gated(
        vec![
            finding_of(SIMPLIFICATION, Review),
            finding_of(SHARED_LOGIC, Review),
            reported(SHARED_LOGIC, Consider),
        ],
        &args(),
    );
    let mut out = Vec::new();
    output::agent(&mut out, &report, false, output::Style::PLAIN).unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(
        text.contains("[maintainability/function-simplification] (fails the gate) Copies"),
        "{text}"
    );
    assert!(
        text.contains("[maintainability/shared-logic] Copies"),
        "{text}"
    );
    assert!(
        text.contains("\n2 reviews did not fail the gate: by default only rules and levels right at least 80% of the time on projects JevGate was never tuned on fail it, and theirs are still being measured."),
        "{text}"
    );
}

#[test]
fn a_preview_language_s_findings_never_fail_the_default_gate() {
    use crate::maturity::Labels;
    use schema::{Gating::*, Strength::*};
    let findings = vec![
        finding_of(SIMPLIFICATION, Review),
        reported("documentation/comments", Consider),
    ];
    let report = gated_at(KOTLIN_FILE, findings.clone(), &args());
    assert_eq!(
        gates(&report),
        [Some(Measuring), Some(Measuring)],
        "a mature rule and level too"
    );
    assert!(report.gate.as_ref().unwrap().passed);
    // Kotlin's own labels, not the ten supported languages' 20 of 23.
    let precision: Vec<_> = report.files[0]
        .findings
        .iter()
        .map(|f| f.precision)
        .collect();
    let labels = |right, labeled| Some(Labels { right, labeled });
    assert_eq!(precision, [labels(1, 1), labels(1, 2)]);
    let mut out = Vec::new();
    output::agent(&mut out, &report, false, output::Style::PLAIN).unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(
        text.contains("[maintainability/function-simplification] Copies: 50% alike,\nsee `b` Not yet measured in Kotlin.\n"),
        "{text}"
    );
    assert!(
        text.contains("\n2 reviews in Kotlin files did not fail the gate: Kotlin is in preview, and by default JevGate's own rules never fail it there. `--fail-on review` makes every review fail the gate.\n"),
        "{text}"
    );
    assert!(!text.contains("theirs are still being measured"), "{text}");
    // An explicit level replaces the default exactly as it says.
    let mut options = args();
    options.fail_on = vec![options::FailOn::Review];
    let report = gated_at(KOTLIN_FILE, findings, &options);
    assert_eq!(gates(&report), [Some(Fails), Some(Fails)]);
    // A language's own labels, from 20 on, give a share: Bash's
    // function-simplification reviews were right 25 times in 28.
    let bash = gated_at(
        ("scripts/deploy.sh", "deploy() {\n  echo done\n}\n"),
        vec![finding_of(SIMPLIFICATION, Review)],
        &args(),
    );
    let mut out = Vec::new();
    output::agent(&mut out, &bash, false, output::Style::PLAIN).unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(
        text.contains("see `b` Right 89% of the time in Bash (28 labels).\n"),
        "{text}"
    );
}

#[test]
fn sarif_says_a_preview_language_s_findings_are_measured_in_it() {
    use schema::Strength::*;
    let report = gated_at(
        KOTLIN_FILE,
        vec![finding_of(SIMPLIFICATION, Review)],
        &args(),
    );
    let mut out = Vec::new();
    crate::sarif::emit(&mut out, &report, &[]).unwrap();
    let log: serde_json::Value = serde_json::from_slice(&out).unwrap();
    let result = &log["runs"][0]["results"][0];
    assert_eq!(result["level"], "warning");
    assert_eq!(result["properties"]["gate"], "measuring");
    assert_eq!(
        result["properties"]["precision"],
        json!({"right": 1, "labeled": 1})
    );
    assert_eq!(result["properties"]["preview"], "Kotlin");
    let message = result["message"]["text"].as_str().unwrap();
    assert!(
        message.contains("Not yet measured in Kotlin.")
            && message.ends_with("Does not fail the gate: Kotlin is in preview, and by default JevGate's own rules never fail it there."),
        "{message}"
    );
}

#[test]
fn a_capped_list_shows_the_findings_that_fail_the_gate_first() {
    use schema::Strength::*;
    let failing = schema::Finding {
        rank: 0.1,
        ..reported("documentation/agent-context", Consider)
    };
    let mut findings = vec![reported(SHARED_LOGIC, Consider); 12];
    findings.push(failing);
    let report = gated(findings, &args());
    let mut out = Vec::new();
    output::agent(&mut out, &report, false, output::Style::PLAIN).unwrap();
    let text = String::from_utf8(out).unwrap();
    let section = text.split("Review (").nth(1).unwrap();
    // Every finding that fails the gate, then the top ten others.
    assert!(
        section.starts_with(
            "13, top 11, those that fail the gate first; --verbose shows all):\n  src/lib.rs:12 [documentation/agent-context] (fails the gate)"
        ),
        "{text}"
    );
}
