//! Hardcoded values: value units and the look-here question.
use super::*;

#[test]
fn functions_with_literals_and_module_constants_are_hardcoded_value_units() {
    let (project, options) = hardcoded_project();
    let (_, plan) = planned(&project, &options);
    assert_eq!(stages(&plan), ["constants", "functions"]);
    let functions = first_request(&plan, "functions");
    let sent = &functions["state"]["functions"];
    assert_eq!(sent.as_array().unwrap().len(), 1, "`total` has no literal");
    assert!(
        sent[0].get("values").is_none(),
        "the look-here question reads the source"
    );
    let questions: Vec<&String> = functions["questions"].as_object().unwrap().keys().collect();
    assert_eq!(questions, ["f0_values"]);
    let constants = first_request(&plan, "constants");
    assert_eq!(constants["state"]["constants"][0]["value"], "\"eu-west-1\"");
    let questions: Vec<&String> = constants["questions"].as_object().unwrap().keys().collect();
    assert_eq!(questions, ["look"]);
}

#[test]
fn a_function_the_look_question_flags_is_one_unmeasured_review() {
    let (project, mut options) = hardcoded_project();
    let mut eval = scripted(0);
    eval.overrides = vec![("f0_values", json!({"type":"noul","noul":0.9}))];
    let report = run(&project, &options, &mut eval);
    assert_eq!(
        eval.stages,
        ["first", "first"],
        "no recheck, locate or kind"
    );
    let findings = &report.files[0].findings;
    assert_eq!(findings.len(), 1);
    let finding = &findings[0];
    assert_eq!(finding.symbol.as_deref(), Some("connect"));
    assert_eq!(
        (finding.strength, finding.measured_as),
        (Strength::Review, None)
    );
    assert!(
        finding.message.contains("fixed value worth a look")
            && finding
                .action
                .contains("dismiss this finding with a reason"),
        "{}",
        finding.message
    );
    // Below the look probability the unit is clear, never undecided.
    options.refresh = true;
    let mut eval = scripted(1);
    let report = run(&project, &options, &mut eval);
    let dimension = &report.files[0].dimensions["hardcoded_values"];
    assert_eq!((dimension.units.clear, dimension.units.uncertain), (2, 0));
}

#[test]
fn module_constants_are_asked_whether_one_fixes_a_value_worth_a_look() {
    let source =
        "const API_URL: &str = \"https://api.prod.example.com\";\nconst RETRIES: u32 = 3;\n";
    let (project, options) = rule_project(source, catalog::HARDCODED_VALUES);
    let mut eval = scripted(0);
    eval.overrides = vec![("look", json!({"type":"noul","noul":0.85}))];
    let report = run(&project, &options, &mut eval);
    let finding = &report.files[0].findings[0];
    assert_eq!(finding.locations.len(), 2, "each constant");
    assert!(
        finding
            .message
            .starts_with("A constant of this file may fix a value"),
        "{}",
        finding.message
    );
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
    let (sizes, before) = packs(&[("lib.rs", &source(false))], &rules, "functions");
    assert_eq!(sizes, [4, 5, 1]);
    let (sizes, after) = packs(&[("lib.rs", &source(true))], &rules, "functions");
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
