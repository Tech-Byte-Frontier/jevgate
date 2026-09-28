//! Injection, sensitive data and unsafe settings: traces, callers and settle
//! Choices here; PHP pages, the checked kinds, exposure, settings, server
//! templates and the confirm Choices in their own modules.
use super::*;

mod confirms;
mod exposure;
mod kinds;
mod php;
mod settings;
mod templates;
use confirms::*;
use kinds::*;
use settings::*;

const QUERY: &str = "fn find(conn: &Connection, name: &str) -> Result<Row> {\n    let sql = format!(\"SELECT id FROM users WHERE name = '{name}'\");\n    conn.query_row(&sql, [], Row::from)\n}\n\nfn total(a: i32, b: i32) -> i32 {\n    a + b\n}\n";

fn security_project(source: &str) -> (Project, CheckArgs) {
    project_with(&[("lib.rs", source)], &catalog::SECURITY)
}

/// A certain choice of `id` among the two sites of `find` in `QUERY`.
fn site(id: &str) -> Value {
    let probabilities: serde_json::Map<String, Value> = ["S1", "S2", "none"]
        .iter()
        .map(|option| {
            (
                option.to_string(),
                json!(if *option == id { 1.0 } else { 0.0 }),
            )
        })
        .collect();
    json!({"type":"choice","choice":id,"confidence":1.0,"probabilities":probabilities})
}

#[test]
fn only_functions_with_calls_built_text_or_field_assignments_are_sent_for_security() {
    let (project, options) = security_project(QUERY);
    let (_, plan) = planned(&project, &options);
    let sent: Vec<&str> = plan
        .requests
        .iter()
        .flat_map(|p| p.request["state"]["functions"].as_array().unwrap())
        .filter_map(|f| f["name"].as_str())
        .collect();
    assert_eq!(
        sent,
        ["find"],
        "`total` has no call, built text or field assignment"
    );
}

#[test]
fn clear_presence_needs_no_trace_and_clears_every_security_rule() {
    let (project, options) = security_project(QUERY);
    let mut eval = scripted(0);
    let report = run(&project, &options, &mut eval);
    assert_eq!(eval.stages, ["first"]);
    for rule in catalog::SECURITY {
        assert_eq!(report.files[0].dimensions[rule].status, Status::Clear);
    }
}

#[test]
fn an_unhandled_value_from_another_party_is_a_located_injection_review() {
    let (project, options) = security_project(QUERY);
    let mut eval = scripted(0);
    eval.overrides = standing();
    eval.overrides.extend([
        ("interpreted", noul_at(0.95)),
        ("sql", noul_at(0.95)),
        ("origin", spread(0.0, 0.1, 0.9)),
        ("site", site("S1")),
    ]);
    let report = run(&project, &options, &mut eval);
    assert_eq!(
        eval.stages,
        ["first", "first", "first"],
        "one trace and what the query's values hold, no recheck"
    );
    let finding = &report.files[0].findings[0];
    assert_eq!(finding.rule, "security/injection");
    assert_eq!(finding.strength, Strength::Review);
    assert_eq!(finding.category.as_deref(), Some("CWE-89 SQL injection"));
    assert_eq!(finding.line, 2, "located at the chosen site");
    assert!(finding.action.contains("bound query parameters"));
}

/// The options of the Choice on what an injection consider's values can hold.
const VALUES: [&str; 5] = ["fixed", "local", "outside", "own", "unknown"];

#[test]
fn a_parameter_origin_is_a_consider_that_callers_can_settle() {
    let caller = format!(
        "{QUERY}\nfn handler(conn: &Connection) -> Result<Row> {{\n    find(conn, \"admin\")\n}}\n"
    );
    let overrides = || {
        vec![
            ("interpreted", noul_at(0.95)),
            ("sql", noul_at(0.95)),
            ("origin", spread(0.0, 0.9, 0.1)),
            ("values", choice_of("unknown", &VALUES)),
        ]
    };
    let (project, options) = security_project(QUERY);
    let mut eval = scripted(0);
    eval.overrides = overrides();
    let report = run(&project, &options, &mut eval);
    assert_eq!(
        report.files[0].dimensions[catalog::INJECTION].status,
        Status::Consider,
        "no caller is known, so the parameter stays a concern"
    );
    let (project, options) = security_project(&caller);
    let mut eval = scripted(0);
    eval.overrides = overrides();
    eval.recheck_level = Some(0);
    let report = run(&project, &options, &mut eval);
    assert!(eval.stages.contains(&"recheck".to_string()));
    let find = |report: &Report| {
        report.files[0]
            .findings
            .iter()
            .any(|f| f.rule == "security/injection" && f.symbol.as_deref() == Some("find"))
    };
    assert!(!find(&report), "the caller passes a fixed value");
}

#[test]
fn a_parameter_consider_is_a_note_when_its_values_are_the_programs_own() {
    let caller = format!(
        "{QUERY}\nfn handler(conn: &Connection) -> Result<Row> {{\n    find(conn, \"admin\")\n}}\n"
    );
    let (project, mut options) = security_project(&caller);
    let mut judged = |values: Value| {
        let mut eval = scripted(0);
        eval.overrides = vec![
            ("interpreted", noul_at(0.95)),
            ("sql", noul_at(0.95)),
            ("origin", spread(0.0, 0.9, 0.1)),
        ];
        // The recheck with callers keeps the parameters as the origin.
        eval.recheck_overrides = vec![
            ("sql", noul_at(0.95)),
            ("origin", spread(0.0, 0.9, 0.1)),
            ("values", values),
        ];
        let report = run(&project, &options, &mut eval);
        options.refresh = true;
        report.files[0]
            .findings
            .iter()
            .find(|f| f.rule == "security/injection" && f.symbol.as_deref() == Some("find"))
            .map(|f| f.strength)
    };
    assert_eq!(
        judged(choice_of("fixed", &VALUES)),
        Some(Strength::Note),
        "the caller passes a literal"
    );
    assert_eq!(
        judged(choice_of("outside", &VALUES)),
        Some(Strength::Consider)
    );
    let mut split: serde_json::Map<String, Value> =
        VALUES.iter().map(|k| (k.to_string(), json!(0.0))).collect();
    split.insert("own".into(), json!(0.3));
    split.insert("local".into(), json!(0.25));
    split.insert("unknown".into(), json!(0.45));
    let leaning =
        json!({"type":"choice","choice":"unknown","confidence":0.4,"probabilities":split});
    assert_eq!(
        judged(leaning),
        Some(Strength::Note),
        "the program's own options together lead"
    );
}

#[test]
fn a_check_left_undecided_is_decided_again_with_callers() {
    let caller = format!(
        "{QUERY}\nfn handler(conn: &Connection, request: &Request) -> Result<Row> {{\n    find(conn, &request.query[\"name\"])\n}}\n"
    );
    let (project, options) = security_project(&caller);
    let mut eval = scripted(0);
    eval.overrides = vec![
        ("resource", noul_at(0.95)),
        ("path", noul_at(0.5)),
        ("origin", spread(0.0, 0.9, 0.1)),
    ];
    eval.recheck_level = Some(2);
    let report = run(&project, &options, &mut eval);
    let finding = report.files[0]
        .findings
        .iter()
        .find(|f| f.symbol.as_deref() == Some("find") && f.rule == "security/injection")
        .expect("the recheck decides the path check and the origin");
    assert_eq!(
        finding.strength,
        Strength::Review,
        "undecided without the recheck"
    );
    assert!(finding.category.is_some());
}

#[test]
fn a_parameter_in_a_path_or_url_is_a_note_until_callers_show_another_party() {
    let (project, options) = security_project(QUERY);
    let mut eval = scripted(0);
    eval.overrides = vec![
        ("resource", noul_at(0.95)),
        ("url", noul_at(0.95)),
        ("origin", spread(0.0, 0.9, 0.1)),
        ("url_parts", choice_of("given", &URL_PARTS)),
    ];
    let finding = &first_finding(&project, &options, &mut eval);
    assert_eq!(finding.strength, Strength::Note);
    assert_eq!(
        finding.category.as_deref(),
        Some("CWE-918 server-side request forgery")
    );
}

#[test]
fn a_decided_url_finding_on_the_programs_own_host_is_clear() {
    let (project, mut options) = security_project(FETCH_QUOTE);
    let mut status = |parts: &str| {
        let mut eval = scripted(0);
        eval.overrides = vec![
            ("resource", noul_at(0.95)),
            ("url", noul_at(0.95)),
            ("origin", spread(0.0, 0.0, 1.0)),
            ("url_parts", choice_of(parts, &URL_PARTS)),
        ];
        let report = run(&project, &options, &mut eval);
        options.refresh = true;
        report.files[0].dimensions[catalog::INJECTION]
            .status
            .clone()
    };
    assert_eq!(status("outside"), Status::Review);
    assert_eq!(
        status("own"),
        Status::Clear,
        "a configured base URL with an id in its path"
    );
}

const FETCH_QUOTE: &str = "fn quote(client: &Client, base: &Url, symbol: &str) -> String {\n    let url = base.join(&format!(\"quotes/{symbol}\")).unwrap();\n    client.get(url).send().unwrap().text().unwrap()\n}\n";

const URL_PARTS: [&str; 5] = ["own", "forwards", "given", "outside", "none"];

const PATH_SOURCE: [&str; 5] = ["own", "local", "given", "outside", "none"];

const RUNS_IN: [&str; 3] = ["browser", "server", "either"];

/// Injection status and settle requests with the URL (or path) check at
/// `check` undecided and the settle Choice answering `parts`.
fn settled_injection(
    project: &Project,
    options: &CheckArgs,
    check: &'static str,
    parts: &str,
) -> (Status, u64) {
    let mut eval = scripted(0);
    eval.overrides = vec![
        ("resource", noul_at(0.95)),
        (check, noul_at(0.4)),
        ("origin", spread(0.0, 0.9, 0.1)),
        ("url_parts", choice_of(parts, &URL_PARTS)),
        ("path_source", choice_of(parts, &PATH_SOURCE)),
        ("runs_in", choice_of("server", &RUNS_IN)),
    ];
    let report = run(project, options, &mut eval);
    (
        report.files[0].dimensions[catalog::INJECTION]
            .status
            .clone(),
        report
            .stages
            .get("settle")
            .map_or(0, |stage| stage.successful_requests),
    )
}

#[test]
fn an_undecided_url_is_settled_only_by_a_host_of_the_programs_own() {
    let (project, mut options) = security_project(FETCH_QUOTE);
    assert_eq!(
        settled_injection(&project, &options, "url", "own"),
        (Status::Clear, 2),
        "where its URLs come from and where it runs"
    );
    // A URL its caller gives or forwards is a note, as a found one would be;
    // one from another party stays open.
    for (parts, status) in [
        ("forwards", Status::Note),
        ("given", Status::Note),
        ("outside", Status::Uncertain),
    ] {
        options.refresh = true;
        assert_eq!(
            settled_injection(&project, &options, "url", parts).0,
            status,
            "{parts}"
        );
    }
}

#[test]
fn an_undecided_path_is_settled_by_the_programs_own_or_its_local_users_paths() {
    let (project, mut options) = security_project(FETCH_QUOTE);
    for (parts, status) in [
        ("own", Status::Clear),
        ("local", Status::Clear),
        ("given", Status::Note),
        ("outside", Status::Uncertain),
    ] {
        assert_eq!(
            settled_injection(&project, &options, "path", parts),
            (status, 1),
            "{parts}"
        );
        options.refresh = true;
    }
    // The note names the path it left undecided.
    let mut eval = scripted(0);
    eval.overrides = vec![
        ("resource", noul_at(0.95)),
        ("path", noul_at(0.4)),
        ("origin", spread(0.0, 0.9, 0.1)),
        ("path_source", choice_of("given", &PATH_SOURCE)),
    ];
    let report = run(&project, &options, &mut eval);
    let note = &report.files[0].findings[0];
    assert!(
        note.message.contains("places a parameter into a file path"),
        "{}",
        note.message
    );
}

#[test]
fn checks_that_all_clear_rule_out_an_uncertain_presence() {
    let (project, options) = security_project(QUERY);
    let mut eval = scripted(0);
    eval.overrides = vec![
        ("interpreted", noul_at(0.5)),
        ("origin", spread(0.0, 1.0, 0.0)),
    ];
    let report = run(&project, &options, &mut eval);
    assert_eq!(
        report.files[0].dimensions[catalog::INJECTION].status,
        Status::Clear
    );
}

#[test]
fn an_undecided_value_is_cleared_by_its_kind_or_leans_into_a_note() {
    let (project, mut options) = hardcoded_project();
    let split = || {
        vec![
            ("environment", spread(0.4, 0.1, 0.5)),
            ("magic", spread(0.6, 0.1, 0.3)),
        ]
    };
    let mut eval = scripted(0);
    eval.overrides = split();
    let report = run(&project, &options, &mut eval);
    assert!(eval.stages.contains(&"recheck".to_string()));
    let dimension = &report.files[0].dimensions["hardcoded_values"];
    assert_eq!(
        (dimension.units.note, dimension.units.uncertain),
        (2, 0),
        "not cleared by the kind checks, and leaning toward the concern"
    );
    let leaned = report.files[0]
        .findings
        .iter()
        .find(|f| f.symbol.as_deref() == Some("connect"))
        .expect("the environment answer leaned toward its concern");
    assert_eq!(leaned.strength, Strength::Note);
    assert!(
        leaned.message.contains("the answer was split"),
        "{}",
        leaned.message
    );
    options.refresh = true;
    let mut eval = scripted(0);
    eval.overrides = split();
    eval.recheck_level = Some(2);
    let report = run(&project, &options, &mut eval);
    let dimension = &report.files[0].dimensions["hardcoded_values"];
    assert_eq!(
        dimension.status,
        Status::Clear,
        "every value is of an acceptable kind"
    );
}

#[test]
fn an_undecided_caller_recheck_replaces_an_undecided_traced_lean() {
    let caller = format!(
        "{QUERY}\nfn handler(conn: &Connection, dir: &Path) -> Result<Row> {{\n    find(conn, &dir.join(\"cache\"))\n}}\n"
    );
    let (project, options) = security_project(&caller);
    let mut eval = scripted(0);
    eval.overrides = vec![
        ("resource", noul_at(0.95)),
        ("path", noul_at(0.6)),
        ("origin", spread(0.0, 0.9, 0.1)),
    ];
    eval.recheck_level = Some(0);
    eval.recheck_overrides = vec![("path", noul_at(0.3)), ("origin", spread(0.1, 0.9, 0.0))];
    let report = run(&project, &options, &mut eval);
    assert_eq!(eval.stages.last().unwrap(), "recheck");
    let file = &report.files[0];
    assert!(
        !file
            .findings
            .iter()
            .any(|f| f.rule == "security/injection" && f.symbol.as_deref() == Some("find")),
        "the callers' answer leans away, so no note"
    );
}

#[test]
fn a_check_left_undecided_by_its_callers_is_quoted_as_the_recheck_asked_it() {
    let caller = format!(
        "{QUERY}\nfn handler(conn: &Connection, request: &Request) -> Result<Row> {{\n    find(conn, &request.query[\"name\"])\n}}\n"
    );
    let (project, options) = security_project(&caller);
    let report = run(&project, &options, &mut scripted(3));
    let undecided = &report.files[0].dimensions[catalog::INJECTION].undecided;
    let open = |unit: &str| {
        let unit = undecided.iter().find(|u| u.unit == unit).unwrap();
        unit.open.iter().find(|q| q.id == "origin").unwrap().clone()
    };
    // Asked again with its callers, `find`'s undecided origin replaced the
    // traced one; `handler` has no callers, so its trace is quoted.
    let (find, handler) = (open("find"), open("handler"));
    use crate::schema::Pass;
    assert_eq!((find.pass, handler.pass), (Pass::Recheck, Pass::Trace));
    assert_eq!(find.evidence, ["function.source"]);
}

/// The status of `rule` and the number of settle requests, with `nouls`
/// leaving one check undecided and `settle` answering its Choice.
fn settled_status(
    project: &Project,
    options: &CheckArgs,
    rule: &str,
    nouls: &[(&'static str, f64)],
    settle: (&'static str, Value),
) -> (Status, u64) {
    let mut eval = scripted(0);
    eval.overrides = standing();
    eval.overrides
        .extend(nouls.iter().map(|&(q, p)| (q, noul_at(p))));
    eval.overrides.push(("origin", spread(0.0, 0.9, 0.1)));
    eval.overrides
        .push(("values", choice_of("unknown", &VALUES)));
    eval.overrides.push(settle);
    let report = run(project, options, &mut eval);
    (
        report.files[0].dimensions[rule].status.clone(),
        report
            .stages
            .get("settle")
            .map_or(0, |stage| stage.successful_requests),
    )
}

#[test]
fn undecided_markup_cors_cookies_and_logged_objects_are_settled_by_their_choices() {
    let (project, mut options) = security_project(QUERY);
    let markup = ["escaped", "text", "raw", "none"];
    let undecided_markup = [("interpreted", 0.95), ("markup", 0.4)];
    for (chosen, status) in [("escaped", Status::Clear), ("raw", Status::Uncertain)] {
        options.refresh = true;
        let settle = ("markup_output", choice_of(chosen, &markup));
        assert_eq!(
            settled_status(
                &project,
                &options,
                catalog::INJECTION,
                &undecided_markup,
                settle
            ),
            (status, 1),
            "{chosen}"
        );
    }
    let origins = ["unset", "listed", "public", "any"];
    let undecided_cors = [("weakened", 0.4), ("cors", 0.3)];
    for (chosen, status) in [("public", Status::Clear), ("any", Status::Uncertain)] {
        options.refresh = true;
        let settle = ("cors_origins", choice_of(chosen, &origins));
        assert_eq!(
            settled_status(
                &project,
                &options,
                catalog::UNSAFE_SETTINGS,
                &undecided_cors,
                settle
            )
            .0,
            status,
            "{chosen}"
        );
    }
    let cookies = ["unset", "flagged", "missing"];
    let undecided_cookie = [("weakened", 0.4), ("cookie", 0.3)];
    for (chosen, status) in [("flagged", Status::Clear), ("missing", Status::Uncertain)] {
        options.refresh = true;
        let settle = ("cookie_flags", choice_of(chosen, &cookies));
        assert_eq!(
            settled_status(
                &project,
                &options,
                catalog::UNSAFE_SETTINGS,
                &undecided_cookie,
                settle
            )
            .0,
            status,
            "{chosen}"
        );
    }
    let logs = [
        "plain", "identity", "operator", "secret", "personal", "none",
    ];
    let undecided_logs = [("logs_secret", 0.4), ("logs_object_secret", 0.4)];
    for (chosen, status) in [("plain", Status::Clear), ("secret", Status::Uncertain)] {
        options.refresh = true;
        let settle = ("logged", choice_of(chosen, &logs));
        assert_eq!(
            settled_status(
                &project,
                &options,
                catalog::SENSITIVE_DATA,
                &undecided_logs,
                settle
            )
            .0,
            status,
            "{chosen}"
        );
    }
}

#[test]
fn a_decided_check_asks_no_settle_choice() {
    let (project, options) = security_project(QUERY);
    let settle = ("markup_output", choice_of("escaped", &["escaped", "raw"]));
    let (status, settles) = settled_status(
        &project,
        &options,
        catalog::INJECTION,
        &[("interpreted", 0.95), ("markup", 0.95)],
        settle,
    );
    assert_eq!(settles, 0, "a markup check at review is not second-guessed");
    assert_eq!(status, Status::Consider);
}

#[test]
fn a_function_added_to_one_run_is_the_only_security_request_asked_again() {
    // Runs end after `f2`, `f4` and `f8`, whose names hash to an end.
    let query = |name: &str| {
        format!(
            "fn {name}(conn: &Connection, name: &str) -> Result<Row> {{\n    let sql = format!(\"SELECT id FROM users WHERE name = '{{name}}'\");\n    conn.query_row(&sql, [], Row::from)\n}}\n\n"
        )
    };
    let source = |added: bool| -> String {
        (0..14)
            .map(|i| match i {
                3 if added => format!("{}{}", query("f3"), query("g")),
                _ => query(&format!("f{i}")),
            })
            .collect()
    };
    let (sizes, before) = packs(
        &[("lib.rs", &source(false))],
        &catalog::SECURITY,
        "functions",
    );
    assert_eq!(sizes, [3, 2, 4, 5]);
    let (sizes, after) = packs(
        &[("lib.rs", &source(true))],
        &catalog::SECURITY,
        "functions",
    );
    assert_eq!(sizes, [3, 3, 4, 5]);
    only_changed(&before, &after, 1);
}
