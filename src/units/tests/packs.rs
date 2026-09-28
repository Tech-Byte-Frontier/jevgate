//! Function packs: every rule's first-pass questions about a function, asked
//! in one request that sends its source once.
use super::*;
use crate::catalog::{
    FUNCTION_SIMPLIFICATION, HARDCODED_VALUES, INJECTION, SENSITIVE_DATA, UNSAFE_SETTINGS,
};

/// The rules whose first pass asks about functions.
const FUNCTION_RULES: [&str; 5] = [
    FUNCTION_SIMPLIFICATION,
    HARDCODED_VALUES,
    INJECTION,
    SENSITIVE_DATA,
    UNSAFE_SETTINGS,
];

/// A function every rule of `FUNCTION_RULES` judges: five body lines, a
/// literal value and a query built from its parameter.
fn queried(name: &str) -> String {
    format!(
        "fn {name}(conn: &Connection, table: &str) -> Result<usize> {{\n    let mut total = 0;\n    for row in conn.query(&format!(\"SELECT id FROM {{table}}\"), [])? {{\n        total += row.get::<usize>(0)?;\n    }}\n    let floor = total.max(40);\n    Ok(floor)\n}}\n"
    )
}

#[test]
fn every_rule_asks_about_a_function_in_one_request_that_sends_it_once() {
    // Neither name ends a run, so one pack holds both.
    let source = format!("{}{}", queried("find"), queried("scan"));
    let (project, options) = project_with(&[("lib.rs", &source)], &FUNCTION_RULES);
    let (_, plan) = planned(&project, &options);
    assert_eq!(stages(&plan), ["functions"]);
    let pack = &plan.requests[0];
    let functions = pack.request["state"]["functions"].as_array().unwrap();
    let names: Vec<&str> = functions
        .iter()
        .map(|f| f["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["find", "scan"]);
    assert_eq!(
        functions[0]["values"],
        json!(["\"SELECT id FROM {table}\"", "40"])
    );
    let asked: Vec<(&str, &str, &str)> = pack
        .asked
        .questions
        .iter()
        .filter(|q| q.key.starts_with("f0_"))
        .map(|q| (q.key.as_str(), q.rule, q.unit.as_str()))
        .collect();
    assert_eq!(
        asked,
        [
            ("f0_split", FUNCTION_SIMPLIFICATION, "function:find"),
            ("f0_environment", HARDCODED_VALUES, "values:find"),
            ("f0_magic", HARDCODED_VALUES, "values:find"),
            ("f0_special", HARDCODED_VALUES, "values:find"),
            ("f0_interpreted", INJECTION, "injection:find"),
            ("f0_resource", INJECTION, "injection:find"),
            ("f0_logs_secret", SENSITIVE_DATA, "data:find"),
            ("f0_error_details", SENSITIVE_DATA, "data:find"),
            ("f0_weakened", UNSAFE_SETTINGS, "settings:find"),
        ]
    );
}

#[test]
fn one_request_answers_every_rule_about_its_functions() {
    let (project, options) = project_with(&[("lib.rs", &queried("load"))], &FUNCTION_RULES);
    let report = run(&project, &options, &mut scripted(0));
    assert_eq!(report.api_requests, 1);
    let file = &report.files[0];
    let mut answered: Vec<&str> = file
        .judgments
        .iter()
        .filter(|j| j.pass == crate::schema::Pass::First)
        .map(|j| j.rule.as_str())
        .collect();
    answered.dedup();
    assert_eq!(answered, FUNCTION_RULES);
    for rule in FUNCTION_RULES {
        assert_eq!(file.dimensions[rule].status, Status::Clear, "{rule}");
    }
}

#[test]
fn a_function_whose_questions_do_not_fit_together_is_asked_rule_by_rule() {
    let (project, mut options) =
        project_with(&[("lib.rs", &queried("load"))], &[FUNCTION_SIMPLIFICATION]);
    // The request function simplification sends alone fits once answered,
    // whatever the budget; under a budget nothing else fits.
    let alone = planned(&project, &options).1.requests.remove(0).request;
    let store = crate::storage::Store::open(&project.0).unwrap();
    let whole = (
        crate::schema::RUBRIC,
        crate::requests::provider_request(&alone),
    );
    let key = crate::schema::hash(&serde_json::to_vec(&whole).unwrap());
    store
        .save_request(&key, &answer(&alone, 0), crate::schema::now())
        .unwrap();
    options.rules.push(HARDCODED_VALUES.into());
    let tight = TokenBudget {
        bytes_per_token: 1e-3,
    };
    let (_, plan) = planned_with(&project, &options, &tight);
    let sent: Vec<&Value> = plan.requests.iter().map(|p| &p.request).collect();
    assert_eq!(sent, [&alone], "the split question is sent as it is alone");
    let units = &file_plan(&plan, "lib.rs").units;
    let unit = |rule: &str| units.iter().find(|u| u.rule == rule).unwrap();
    assert_eq!(unit(FUNCTION_SIMPLIFICATION).presence, Presence::Judged);
    let values = unit(HARDCODED_VALUES);
    assert_eq!(values.presence, Presence::NeedsContext);
    assert!(values.recheck.is_none());
    assert!(matches!(values.detail, Detail::Values { locate: None, .. }));
}

#[test]
fn a_function_added_to_one_run_is_the_only_pack_every_rule_asks_again() {
    // Runs end after `f2`, `f4` and `f8`, whose names hash to an end.
    let source = |added: bool| -> String {
        (0..14)
            .map(|i| match i {
                3 if added => format!("{}{}", queried("f3"), queried("g")),
                _ => queried(&format!("f{i}")),
            })
            .collect()
    };
    let rules = FUNCTION_RULES;
    let (sizes, before) = packs(&[("lib.rs", &source(false))], &rules, "functions");
    assert_eq!(sizes, [3, 2, 4, 5]);
    let (sizes, after) = packs(&[("lib.rs", &source(true))], &rules, "functions");
    assert_eq!(sizes, [3, 3, 4, 5]);
    only_changed(&before, &after, 1);
}
