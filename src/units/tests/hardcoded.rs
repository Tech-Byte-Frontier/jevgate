//! Hardcoded values: value units, benign kinds and repeated literals.
use super::*;

#[test]
fn functions_with_literals_and_module_constants_are_hardcoded_value_units() {
    let (project, options) = hardcoded_project();
    let (_, plan) = planned(&project, &options);
    assert_eq!(stages(&plan), ["values", "constants"]);
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
fn a_finding_on_module_constants_points_at_the_constant_it_is_about() {
    let source =
        "const API_URL: &str = \"https://api.prod.example.com\";\nconst RETRIES: u32 = 3;\n";
    let (project, options) = rule_project(source, catalog::HARDCODED_VALUES);
    let mut eval = scripted(0);
    eval.overrides = vec![
        ("environment", spread(0.0, 0.05, 0.95)),
        ("constant", choice_of("c0", &["c0", "c1", "none"])),
    ];
    let report = run(&project, &options, &mut eval);
    let finding = &report.files[0].findings[0];
    assert_eq!(finding.locations.len(), 1);
    assert_eq!(finding.line, 1);
    assert_eq!(finding.symbol.as_deref(), Some("API_URL"));
    assert!(
        finding.message.ends_with("The constant is `API_URL`."),
        "{}",
        finding.message
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
    // the review is a note.
    let finding = &report.files[0].findings[0];
    assert_eq!(finding.strength, Strength::Note);
    assert_eq!(finding.rule, "maintainability/hardcoded-values");
    assert!(
        finding.message.starts_with(
            "`connect` special-cases one specific identity. No single value stood out, so it is a note"
        ),
        "{}",
        finding.message
    );
    assert!(finding.action.starts_with("Optional"));
    assert!(finding.values.is_empty());
    assert_eq!(report.files[0].dimensions["hardcoded_values"].units.note, 1);
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
fn a_value_that_needs_a_name_but_is_written_once_is_a_note() {
    let findings = |source: &str| {
        let (project, options) = rule_project(source, catalog::HARDCODED_VALUES);
        let mut eval = scripted(0);
        eval.overrides = vec![
            ("magic", spread(0.1, 0.35, 0.55)),
            ("value", choice_of("v1", &["v0", "v1", "none"])),
        ];
        run(&project, &options, &mut eval).files[0].findings.clone()
    };
    // `300_000` holds `30_000` only as part of a longer number.
    let once = findings(&format!("{HARDCODED}\nconst CAP: u64 = 300_000;\n"));
    let connect = once
        .iter()
        .find(|f| f.symbol.as_deref() == Some("connect"))
        .unwrap();
    assert_eq!(connect.strength, Strength::Note);
    assert!(
        connect
            .message
            .ends_with("It is written once in its file, so it is a note."),
        "{}",
        connect.message
    );
    let twice = findings(&format!(
        "{HARDCODED}\nfn backup() -> Client {{\n    Client::new(\"db.backup:5432\", 30_000)\n}}\n"
    ));
    let connect = twice
        .iter()
        .find(|f| f.symbol.as_deref() == Some("connect"))
        .unwrap();
    assert_eq!(connect.strength, Strength::Consider, "{}", connect.message);
    // Naming a value is a cleanup: a review-level answer is at most a consider.
    let (project, options) = rule_project(
        &format!(
            "{HARDCODED}\nfn backup() -> Client {{\n    Client::new(\"db.backup:5432\", 30_000)\n}}\n"
        ),
        catalog::HARDCODED_VALUES,
    );
    let mut eval = scripted(0);
    eval.overrides = vec![
        ("magic", spread(0.0, 0.05, 0.95)),
        ("value", choice_of("v1", &["v0", "v1", "none"])),
    ];
    let report = run(&project, &options, &mut eval);
    let connect = report.files[0]
        .findings
        .iter()
        .find(|f| f.symbol.as_deref() == Some("connect"))
        .unwrap();
    assert_eq!(connect.strength, Strength::Consider, "{}", connect.message);
    assert!(
        connect.message.contains("a reader must guess"),
        "{}",
        connect.message
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

#[test]
fn a_value_added_to_one_run_of_functions_leaves_the_other_runs_alone() {
    // Runs end after `v3` and `v8`, whose names hash to an end; `total`
    // is a unit only once it holds a value.
    let source = |added: bool| -> String {
        let mut source = String::new();
        for i in 0..10 {
            source += &format!(
                "fn v{i}() -> Client {{\n    Client::new(\"db.internal:5432\", 30_000)\n}}\n\n"
            );
            if i == 5 && added {
                source += "fn total() -> Client {\n    Client::new(\"cache.internal:6379\", 5_000)\n}\n\n";
            } else if i == 5 {
                source += "fn total(values: &[i32]) -> i32 {\n    values.iter().sum()\n}\n\n";
            }
        }
        source
    };
    let rules = [catalog::HARDCODED_VALUES];
    let (sizes, before) = packs(&[("lib.rs", &source(false))], &rules, "values");
    assert_eq!(sizes, [4, 5, 1]);
    let (sizes, after) = packs(&[("lib.rs", &source(true))], &rules, "values");
    assert_eq!(sizes, [4, 6, 1]);
    only_changed(&before, &after, 1);
}
