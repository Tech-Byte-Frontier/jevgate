//! Hardcoded values: value units, benign kinds and repeated literals.
use super::*;

#[test]
fn functions_with_literals_and_module_constants_are_hardcoded_value_units() {
    let (project, options) = hardcoded_project();
    let (_, plan) = planned(&project, &options);
    let stages: Vec<_> = plan
        .requests
        .iter()
        .map(|p| p.request["jevgate"]["stage"].as_str().unwrap())
        .collect();
    assert_eq!(stages, ["values", "constants"]);
    let values = &plan.requests[0].request["state"]["functions"];
    assert_eq!(
        values.as_array().unwrap().len(),
        1,
        "`total` has no literal"
    );
    assert_eq!(
        values[0]["values"],
        json!(["\"db.internal:5432\"", "30_000"])
    );
    let questions = plan.requests[0].request["questions"].as_object().unwrap();
    assert_eq!(questions.len(), 3);
    assert_eq!(
        plan.requests[1].request["state"]["constants"][0]["value"],
        "\"eu-west-1\""
    );
}

#[test]
fn a_local_default_is_a_note_and_a_special_case_is_a_review() {
    let (project, mut options) = hardcoded_project();
    let run_with = |options: &CheckArgs, overrides: Vec<(&'static str, Value)>| {
        let mut eval = scripted(0);
        eval.overrides = overrides;
        run(&project, options, &mut eval)
    };
    let report = run_with(&options, vec![("environment", spread(0.1, 0.8, 0.1))]);
    let strengths: Vec<_> = report.files[0]
        .findings
        .iter()
        .map(|f| f.strength)
        .collect();
    assert_eq!(strengths, [Strength::Note, Strength::Note]);
    options.refresh = true;
    let report = run_with(
        &options,
        vec![("special", json!({"type":"noul","noul":0.9}))],
    );
    // The locate follow-up offered none clearly, so no value is named and
    // the review is one level lower.
    let finding = &report.files[0].findings[0];
    assert_eq!(finding.strength, Strength::Consider);
    assert_eq!(finding.rule, "maintainability/hardcoded-values");
    assert!(
        finding.message.starts_with(
            "`connect` special-cases one specific identity (0.90). Which value it means was not found"
        ),
        "{}",
        finding.message
    );
    assert!(finding.action.contains("data or configuration"));
    assert!(finding.values.is_empty());
    assert_eq!(
        report.files[0].dimensions["hardcoded_values"]
            .units
            .consider,
        1
    );
    options.refresh = true;
    let chosen = json!({"type":"choice","choice":"v0","confidence":1.0,
        "probabilities":{"v0":1.0,"v1":0.0,"none":0.0}});
    let report = run_with(
        &options,
        vec![
            ("special", json!({"type":"noul","noul":0.9})),
            ("value", chosen),
        ],
    );
    let finding = &report.files[0].findings[0];
    assert_eq!(finding.strength, Strength::Review);
    assert_eq!(finding.values, ["\"db.internal:5432\""]);
    assert!(
        finding
            .message
            .ends_with("The value is \"db.internal:5432\"."),
        "{}",
        finding.message
    );
}

#[test]
fn undecided_units_are_listed_with_the_questions_left_undecided() {
    let (project, mut options) = function_rule_project(&function("borderline"));
    let report = run(&project, &options, &mut scripted(3));
    let dimension = &report.files[0].dimensions["function_simplification"];
    assert_eq!(dimension.status, Status::Uncertain);
    assert_eq!(
        dimension.undecided,
        [crate::schema::Undecided {
            unit: "borderline".into(),
            line: 1,
            questions: vec!["splitting".into()],
            values: Vec::new(),
        }]
    );
    options.refresh = true;
    let report = run(&project, &options, &mut scripted(0));
    assert!(
        report.files[0].dimensions["function_simplification"]
            .undecided
            .is_empty()
    );
}

#[test]
fn undecided_hardcoded_units_name_their_few_candidate_values() {
    let (project, options) = hardcoded_project();
    // Split answers that lean away from the concern, and follow-ups that do
    // not clear them: they stay undecided.
    let mut eval = scripted(0);
    eval.overrides = vec![
        ("environment", spread(0.55, 0.05, 0.4)),
        ("magic", spread(0.55, 0.05, 0.4)),
        ("special", noul_at(0.3)),
    ];
    let report = run(&project, &options, &mut eval);
    let undecided = &report.files[0].dimensions["hardcoded_values"].undecided;
    let connect = undecided.iter().find(|u| u.unit == "connect").unwrap();
    assert_eq!(connect.values, ["\"db.internal:5432\"", "30_000"]);
    assert_eq!(connect.questions.len(), 3);
    let constants = undecided
        .iter()
        .find(|u| u.unit == "module constants")
        .unwrap();
    assert_eq!(constants.values, ["\"eu-west-1\""]);
}

#[test]
fn a_literal_repeated_across_files_is_one_finding_and_its_repeats_are_notes() {
    use crate::schema::{Status, Strength};
    let mut files = vec![
        hardcoded_file("a.ts", "consider", &["'acme-corp'", "0"]),
        hardcoded_file("b.ts", "review", &["'acme-corp'"]),
        hardcoded_file("c.ts", "consider", &["0"]),
        hardcoded_file("d.ts", "consider", &["'acme-corp'", "1"]),
    ];
    let primary = grouped_primary(&mut files);
    assert_eq!(primary.strength, Strength::Review);
    assert_eq!(primary.locations.len(), 3);
    assert!(
        primary.message.contains("also flagged at a.ts:3, d.ts:3"),
        "{}",
        primary.message
    );
    for repeat in [&files[0], &files[3]] {
        assert_eq!(repeat.findings[0].strength, Strength::Note);
        assert!(
            repeat.findings[0]
                .message
                .contains("Same value 'acme-corp' as b.ts:3")
        );
        assert_eq!(repeat.status, Status::Note);
        assert_eq!(repeat.dimensions["hardcoded_values"].status, Status::Note);
    }
    // A short shared value such as `0` never links findings.
    assert_eq!(files[2].findings[0].strength, Strength::Consider);
}
