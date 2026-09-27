//! Hardcoded values: value units, benign kinds and repeated literals.
use super::*;

/// A run answering `overrides`, the finding on `symbol` and how many locate
/// requests it asked; later runs with `options` ask again.
fn judged(
    project: &Project,
    options: &mut CheckArgs,
    overrides: Vec<(&'static str, Value)>,
    symbol: &str,
) -> (crate::schema::Finding, u64) {
    let mut eval = scripted(0);
    eval.overrides = overrides;
    let report = run(project, options, &mut eval);
    options.refresh = true;
    let asked = report
        .stages
        .get("locate")
        .map_or(0, |stage| stage.successful_requests);
    let finding = report.files[0]
        .findings
        .iter()
        .find(|f| f.symbol.as_deref() == Some(symbol))
        .cloned()
        .unwrap();
    (finding, asked)
}

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
fn an_environment_finding_whose_constant_stays_the_same_everywhere_is_a_note() {
    let source = "const API_URL: &str = \"https://api.prod.example.com\";\nconst RETRIES: u32 = 3;\n\nfn client() -> Client {\n    Client::new(API_URL)\n}\n";
    let (project, mut options) = rule_project(source, catalog::HARDCODED_VALUES);
    let (_, plan) = planned(&project, &options);
    let locate = plan.files[&0]
        .units
        .iter()
        .find_map(|u| match &u.detail {
            Detail::Constants {
                locate: Some(locate),
                ..
            } => Some(locate.request()),
            _ => None,
        })
        .expect("a locate for the constants");
    assert_eq!(
        locate["state"]["constants"][0]["used_at"],
        json!(["5: Client::new(API_URL)"])
    );
    assert!(locate["state"]["constants"][1].get("used_at").is_none());
    let kinds = ["author", "each", "fallback", "not_run", "same"];
    let overrides = |kind: &str| {
        vec![
            ("environment", spread(0.0, 0.05, 0.95)),
            ("constant", choice_of("c0", &["c0", "c1", "none"])),
            ("environment_kind", choice_of(kind, &kinds)),
        ]
    };
    // A server each installation must set keeps the review.
    let (kept, _) = judged(&project, &mut options, overrides("each"), "API_URL");
    assert_eq!(kept.strength, Strength::Review, "{}", kept.message);
    let (same, _) = judged(&project, &mut options, overrides("same"), "API_URL");
    assert_eq!(same.strength, Strength::Note);
    assert!(
        same.message.ends_with(
            "The constant is `API_URL`. It likely stays the same wherever the program runs, or is only a fallback, so it is a note."
        ),
        "{}",
        same.message
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
fn a_named_value_that_reads_for_itself_is_a_note() {
    let source = format!(
        "{HARDCODED}\nfn backup() -> Client {{\n    Client::new(\"db.backup:5432\", 30_000)\n}}\n"
    );
    let (project, mut options) = rule_project(&source, catalog::HARDCODED_VALUES);
    let kinds = ["copies", "idiom", "named", "tuning", "unexplained"];
    let overrides = |kind: &str| {
        vec![
            ("magic", spread(0.1, 0.35, 0.55)),
            ("value", choice_of("v1", &["v0", "v1", "none"])),
            ("value_kind", choice_of(kind, &kinds)),
        ]
    };
    // Copies that must change together keep the consider.
    let (kept, asked) = judged(&project, &mut options, overrides("copies"), "connect");
    assert_eq!(kept.strength, Strength::Consider, "{}", kept.message);
    // Both functions' values are located, then both are asked their kind.
    assert_eq!(asked, 4);
    let (named, _) = judged(&project, &mut options, overrides("named"), "connect");
    assert_eq!(named.strength, Strength::Note);
    assert!(
        named
            .message
            .ends_with("It reads for itself where it is used, so it is a note."),
        "{}",
        named.message
    );
}

#[test]
fn the_value_locate_lists_the_other_lines_that_write_each_value() {
    let source = format!(
        "{HARDCODED}\nfn backup() -> Client {{\n    Client::new(\"db.backup:5432\", 30_000)\n}}\n\nconst CAP: u64 = 300_000;\n"
    );
    let (project, options) = rule_project(&source, catalog::HARDCODED_VALUES);
    let (_, plan) = planned(&project, &options);
    let locate = plan.files[&0]
        .units
        .iter()
        .find_map(|u| match &u.detail {
            Detail::Values {
                locate: Some(locate),
                ..
            } if u.name == "connect" => Some(locate.request()),
            _ => None,
        })
        .expect("a locate for connect");
    let values = &locate["state"]["function"]["values"];
    assert_eq!(values[0]["value"], "\"db.internal:5432\"");
    assert!(values[0].get("elsewhere").is_none(), "written once");
    // `300_000` holds `30_000` only as part of a longer number.
    assert_eq!(
        values[1]["elsewhere"],
        json!(["12: Client::new(\"db.backup:5432\", 30_000)"])
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

#[test]
fn a_bend_benchmark_s_values_are_its_workload() {
    let source = "import Base\n\ndef rounds(+n: Nat, +acc: U32) -> U32:\n  match n:\n    case 0n:\n      acc\n    case 1n+p:\n      rounds(p, U32.mul(U32.add(acc, 40503), 2654435761))\n\ndef main() -> IO(Unit):\n  IO.print(U32.show(rounds(100000n, 7)))\n";
    let asked = |path: &str| {
        let (project, options) = project_with(&[(path, source)], &[catalog::HARDCODED_VALUES]);
        let (_, plan) = planned(&project, &options);
        !plan.requests.is_empty()
    };
    assert!(!asked("bench/hash/main.bend"));
    assert!(asked("src/hash.bend"), "the same code outside a benchmark");
}
