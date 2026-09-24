//! Function simplification: split and flatten questions, located blocks and rechecks.
use super::*;

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
