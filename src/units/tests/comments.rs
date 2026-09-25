//! Code comments: their units, questions and findings.
use super::*;

/// A function with two comments above code, one that reads like code, and
/// a directive; documented by a comment of more than twenty words.
const COMMENTED: &str = "/// Totals the values it is given, skipping none of them, and returns the sum\n/// of every value in the order the caller passed them to this function.\nfn total(values: &[i32]) -> i32 {\n    // Start at zero\n    let mut sum = 0;\n    // let old = legacy_total(values);\n    for value in values {\n        // Add the value\n        sum += value;\n    }\n    // eslint-disable-next-line\n    sum\n}\n";

fn comments_project() -> (Project, CheckArgs) {
    rule_project(COMMENTED, catalog::COMMENTS)
}

fn top() -> Value {
    spread(0.0, 0.0, 1.0)
}

#[test]
fn each_comment_is_asked_about_with_the_code_it_is_about() {
    let (project, options) = comments_project();
    let (_, plan) = planned(&project, &options);
    assert_eq!(stages(&plan), ["comments"]);
    let request = &plan.requests[0].request;
    let comments = request["state"]["comments"].as_array().unwrap();
    assert_eq!(comments.len(), 4, "the directive is left out");
    assert_eq!(
        comments[0]["placement"],
        "documentation directly above the declaration in `code`"
    );
    assert!(
        comments[0]["code"]
            .as_str()
            .unwrap()
            .starts_with("fn total")
    );
    assert_eq!(comments[1]["code"], "    let mut sum = 0;");
    assert_eq!(comments[1]["in"], "fn total(values: &[i32]) -> i32");
    let questions: Vec<&String> = request["questions"].as_object().unwrap().keys().collect();
    // Wordiness only for the long documentation, code turned off only for
    // the comment that reads like code.
    assert_eq!(
        questions,
        [
            "c0_history",
            "c0_restates",
            "c0_verbose",
            "c1_history",
            "c1_restates",
            "c2_disabled",
            "c2_history",
            "c2_restates",
            "c3_history",
            "c3_restates",
        ]
    );
}

#[test]
fn repeated_code_in_one_function_is_one_consider_and_documentation_a_note() {
    let (project, options) = comments_project();
    let mut eval = scripted(0);
    eval.overrides = vec![("_restates", top())];
    let report = run(&project, &options, &mut eval);
    let file = &report.files[0];
    let dimension = &file.dimensions[catalog::COMMENTS];
    assert_eq!(
        (dimension.units.judged, dimension.units.consider),
        (4, 3),
        "comments are never reviews"
    );
    let findings: Vec<_> = file
        .findings
        .iter()
        .map(|f| (f.strength, f.line, f.locations.len()))
        .collect();
    assert_eq!(
        findings,
        [(Strength::Consider, 4, 3), (Strength::Note, 1, 1)],
        "the three comments inside `total` are one finding"
    );
    let consider = &file.findings[0];
    assert_eq!(consider.rule, "documentation/comments");
    assert_eq!(consider.symbol.as_deref(), Some("total"));
    assert_eq!(
        consider.message,
        "`total` has 3 comments to clean up (1.00): at lines 4, 6 and 8 they repeat the code."
    );
    assert_eq!(consider.action, "Delete these comments");
}

#[test]
fn a_single_short_comment_is_a_note_and_a_narrated_edit_is_rewritten() {
    let source = "fn total(values: &[i32]) -> i32 {\n    let mut sum = 0;\n    // Now sums with a loop instead of fold, as requested.\n    for value in values {\n        sum += value;\n    }\n    sum\n}\n";
    let (project, options) = rule_project(source, catalog::COMMENTS);
    let mut eval = scripted(0);
    eval.overrides = vec![("_history", json!({"type":"noul","noul":0.95}))];
    let report = run(&project, &options, &mut eval);
    let finding = &report.files[0].findings[0];
    assert_eq!(finding.strength, Strength::Note, "one line costs little");
    assert!(
        finding.message.contains("narrates an edit"),
        "{}",
        finding.message
    );
    let long = source.replace(
        "    // Now sums with a loop instead of fold, as requested.\n",
        "    // Now sums with a loop instead of fold, as requested\n    // in review, since fold was slower\n    // on large inputs.\n",
    );
    let (project, mut options) = rule_project(&long, catalog::COMMENTS);
    options.refresh = true;
    let report = run(&project, &options, &mut eval);
    let finding = &report.files[0].findings[0];
    assert_eq!(finding.strength, Strength::Consider);
    assert!(
        finding
            .action
            .starts_with("Rewrite the comment to describe the code as it is"),
        "{}",
        finding.action
    );
}

#[test]
fn an_undecided_comment_is_rechecked_then_settled_by_its_kind() {
    let source = "fn total(values: &[i32]) -> i32 {\n    let mut sum = 0;\n    // Walk the values\n    // one by one.\n    for value in values {\n        sum += value;\n    }\n    sum\n}\n";
    let (project, options) = rule_project(source, catalog::COMMENTS);
    let mut eval = scripted(3);
    let report = run(&project, &options, &mut eval);
    assert_eq!(eval.stages, ["first", "recheck", "first"], "then the kind");
    // The scripted kind picks the first option, a kind that tells the
    // reader something, which clears it.
    let dimension = &report.files[0].dimensions[catalog::COMMENTS];
    assert_eq!((dimension.units.clear, dimension.units.uncertain), (1, 0));
    let mut options = options;
    options.refresh = true;
    let kinds = questions::comment_kind(false)["criteria"].clone();
    let kinds: Vec<&str> = kinds
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    eval.overrides = vec![("kind", choice_of("restates", &kinds))];
    let report = run(&project, &options, &mut eval);
    let finding = &report.files[0].findings[0];
    // Two lines in all: a note.
    assert_eq!(finding.strength, Strength::Note);
    assert!(
        finding.message.contains("at lines 3–4 it repeats the code"),
        "{}",
        finding.message
    );
}
