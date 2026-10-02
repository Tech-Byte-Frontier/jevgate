//! Across rules: what a request uploads, how units are packed (every rule's
//! questions about a function in one pack), too-small units and composition
//! from saved judgments.
use super::*;

#[test]
fn requests_use_literal_paths_and_upload_no_numbers_hashes_or_local_metadata() {
    // Fourteen functions in four runs, and enough lines for an outline.
    let project = functions_project(14);
    let options = args();
    let (inputs, plan) = planned(&project, &options);
    let functions: Vec<_> = plan
        .requests
        .iter()
        .filter(|p| p.request["jevgate"]["stage"] == "functions")
        .collect();
    let sizes: Vec<usize> = functions
        .iter()
        .map(|p| p.request["state"]["functions"].as_array().unwrap().len())
        .collect();
    assert_eq!(sizes, [3, 2, 4, 5], "runs end after `f2`, `f4` and `f8`");
    let budget = TokenBudget::default();
    for planned in &plan.requests {
        let request = &planned.request;
        assert!(budget.fits(request));
        let uploaded = crate::requests::provider_request(request);
        assert!(uploaded.get("jevgate").is_none());
        assert!(!numbers_in(&uploaded["state"]), "{}", uploaded["state"]);
        let text = uploaded.to_string();
        assert!(!text.contains(&inputs[0].result.source_hash));
        assert_eq!(
            request["jevgate"]["sources"][0]["source_hash"],
            inputs[0].result.source_hash
        );
        for (key, question) in uploaded["questions"].as_object().unwrap() {
            let text = question["instructions"]["question"].as_str().unwrap();
            if key.starts_with("f1_") {
                assert!(text.contains("`functions[1].source`"), "{text}");
            }
        }
        assert_eq!(
            planned.asked.questions.len(),
            uploaded["questions"].as_object().unwrap().len()
        );
    }
    let outline = plan
        .requests
        .iter()
        .find(|p| p.request["jevgate"]["stage"] == "outline")
        .unwrap();
    assert!(outline.request["state"]["members"][0]["signature"].is_string());
    assert!(
        !outline.request.to_string().contains("let mut total"),
        "no bodies"
    );
}

#[test]
fn small_functions_are_too_small_and_never_clear() {
    let project = Project::new();
    project.write("lib.rs", "fn one() -> i32 {\n    1\n}\n");
    let report = run(&project, &args(), &mut Mock::default());
    let dimension = &report.files[0].dimensions["function_simplification"];
    assert_eq!(dimension.units.too_small, 1);
    assert_eq!(dimension.status, Status::NotApplicable);
    assert_eq!(report.files[0].status, Status::NotApplicable);
}

#[test]
fn a_run_ends_after_the_last_item_of_a_key_that_ends_runs() {
    // `count` ends a run; `total` and `other` do not.
    let state = json!({});
    let items = vec!["total", "count", "count", "other", "total"];
    let packs = pack_runs(items.clone(), |key| *key, |_| &state, |_| true);
    assert_eq!(
        packs,
        [vec!["total", "count", "count"], vec!["other", "total"]]
    );
    // Items left out still end runs where they end for the whole file.
    let packs = pack_runs(items, |key| *key, |_| &state, |key| *key != "count");
    assert_eq!(packs, [vec!["total"], vec!["other", "total"]]);
    let long: Vec<&str> = (0..10).map(|_| "total").collect();
    let packs = pack_runs(long, |key| *key, |_| &state, |_| true);
    assert_eq!(
        packs.iter().map(Vec::len).collect::<Vec<_>>(),
        [PACK_ITEMS, 2]
    );
}

#[test]
fn composition_is_pure_and_repeatable_from_saved_judgments() {
    let project = Project::new();
    project.write("lib.rs", &function("busy"));
    let options = args();
    let report: Report = run(&project, &options, &mut scripted(2));
    let (_, plan) = planned(&project, &options);
    let first: Vec<_> = plan.requests.iter().collect();
    let again = compose::compose(&plan.files[&0], &report.files[0].judgments, &first);
    assert_eq!(again.status, report.files[0].status);
    assert_eq!(
        again
            .findings
            .iter()
            .map(|f| &f.fingerprint)
            .collect::<Vec<_>>(),
        report.files[0]
            .findings
            .iter()
            .map(|f| &f.fingerprint)
            .collect::<Vec<_>>()
    );
}

#[test]
fn every_undecided_unit_quotes_each_question_it_left_open() {
    let source = format!(
        "const REGION: &str = \"eu-west-1\";\n\nfn connect() -> Client {{\n    Client::new(\"db.internal:5432\", 30_000)\n}}\n\nfn find(conn: &Connection, name: &str) -> Result<Row> {{\n    let sql = format!(\"SELECT id FROM users WHERE name = '{{name}}'\");\n    conn.query_row(&sql, [], Row::from)\n}}\n\n/// Totals the values.\npub fn total(values: &[i32]) -> i32 {{\n    // Start at zero\n    let mut sum = 0;\n    for value in values {{\n        // Add the value\n        sum += value;\n    }}\n    sum\n}}\n\n{}",
        long_function("busy")
    );
    let (project, mut options) = project_with(
        &[
            ("Cargo.toml", "[package]\nname = \"demo\"\n"),
            ("src/lib.rs", &source),
            (
                "tests/total.rs",
                "#[test]\nfn totals() {\n    let sum = demo::total(&[1, 2]);\n    assert_eq!(sum, 3);\n    assert!(sum > 0);\n}\n",
            ),
            (
                "AGENTS.md",
                "# Testing\nTests live beside the code and use the fixtures in `testdata/`.\n\n# Release\nRun `scripts/release.sh` and tag with `v`.\n",
            ),
        ],
        &[],
    );
    options.rules = crate::catalog::keys().into_iter().map(Into::into).collect();
    options.include_tests = true;
    let report = run(&project, &options, &mut scripted(3));
    let undecided: Vec<_> = report
        .files
        .iter()
        .flat_map(|f| f.dimensions.values())
        .flat_map(|d| &d.undecided)
        .collect();
    let rules: std::collections::BTreeSet<&str> = report
        .files
        .iter()
        .flat_map(|f| &f.dimensions)
        .filter(|(_, d)| !d.undecided.is_empty())
        .map(|(rule, _)| rule.as_str())
        .collect();
    // Look-here questions never leave a unit undecided; literal checks do.
    assert!(
        rules.contains("injection") && rules.contains("test_value"),
        "{rules:?}"
    );
    for unit in &undecided {
        assert_eq!(unit.open.len(), unit.questions.len(), "{unit:#?}");
        assert_eq!(unit.fingerprint.len(), 64);
        for open in &unit.open {
            assert!(!open.evidence.is_empty(), "{open:#?}");
            assert!(open.options.len() >= 2, "{open:#?}");
        }
    }
    assert!(
        undecided
            .iter()
            .flat_map(|u| &u.open)
            .any(|open| open.pass != crate::schema::Pass::First),
        "answers given after the first pass are quoted from their follow-ups"
    );
}

#[test]
fn packing_and_cache_identity_do_not_depend_on_token_calibration() {
    let project = functions_project(12);
    let options = args();
    let inputs = crate::inventory::collect(&options, &project.context(), &[]).unwrap();
    let keys = |bytes_per_token: f64| {
        let budget = TokenBudget { bytes_per_token };
        let views = BTreeMap::from([(
            0,
            match crate::file_kind::plan(&inputs[0], &options, budget.uncached()).unwrap() {
                crate::file_kind::Plan::Ready(view) => view,
                _ => unreachable!(),
            },
        )]);
        plan(&inputs, &views, &options, &budget, &project.0)
            .requests
            .iter()
            .map(|p| crate::requests::provider_request(&p.request).into_owned())
            .collect::<Vec<_>>()
    };
    assert_eq!(keys(2.0), keys(6.0));
}

/// Answers every request at `level`, except that the provider refuses the
/// requests of one stage as beyond the model's context.
struct Refusing {
    stage: &'static str,
    level: usize,
}

impl crate::transport::Evaluator for Refusing {
    fn evaluate(&mut self, request: &Value) -> Result<Value> {
        if request["jevgate"]["stage"] == self.stage {
            let refusal = crate::provider_error::Failure {
                status: 400,
                body: Some(r#"{"detail":{"error_type":"max_tokens_exceeded"}}"#),
                ..Default::default()
            };
            let error = crate::provider_error::provider_error(&crate::provider::TYPESAFE, refusal);
            return Err(error.into());
        }
        Ok(answer(request, self.level))
    }
}

#[test]
fn a_request_refused_as_beyond_the_context_leaves_its_units_unsent() {
    let (project, options) = function_rule_project(&function("total"));
    let mut refusing = Refusing {
        stage: "functions",
        level: 0,
    };
    let report = run(&project, &options, &mut refusing);
    let file = &report.files[0];
    assert_ne!(file.status, Status::Error, "{:?}", file.error);
    let units = &file.dimensions["function_simplification"].units;
    assert_eq!((units.judged, units.needs_context), (0, 1));
    assert_eq!(file.status, Status::NeedsContext);
    // A refused recheck leaves the unit with its undecided first answer.
    let mut options = options;
    options.refresh = true;
    let mut refusing = Refusing {
        stage: "recheck",
        level: 3,
    };
    let report = run(&project, &options, &mut refusing);
    let file = &report.files[0];
    assert_ne!(file.status, Status::Error, "{:?}", file.error);
    // Its split stays undecided, and its look-here answer clears it.
    let units = &file.dimensions["function_simplification"].units;
    assert_eq!((units.judged, units.uncertain, units.clear), (1, 0, 1));
}

/// The rules whose first pass asks about functions.
pub(super) const FUNCTION_RULES: [&str; 5] = [
    catalog::FUNCTION_SIMPLIFICATION,
    catalog::HARDCODED_VALUES,
    catalog::INJECTION,
    catalog::SENSITIVE_DATA,
    catalog::UNSAFE_SETTINGS,
];

/// A function every rule of `FUNCTION_RULES` judges: five body lines, a
/// literal value and a query built from its parameter.
pub(super) fn queried(name: &str) -> String {
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
    assert!(
        functions[0].get("values").is_none(),
        "the look-here questions read the source"
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
            (
                "f0_split",
                catalog::FUNCTION_SIMPLIFICATION,
                "function:find"
            ),
            ("f0_look", catalog::FUNCTION_SIMPLIFICATION, "function:find"),
            ("f0_values", catalog::HARDCODED_VALUES, "values:find"),
            ("f0_interpreted", catalog::INJECTION, "injection:find"),
            ("f0_resource", catalog::INJECTION, "injection:find"),
            ("f0_logs_secret", catalog::SENSITIVE_DATA, "data:find"),
            ("f0_error_details", catalog::SENSITIVE_DATA, "data:find"),
            ("f0_weakened", catalog::UNSAFE_SETTINGS, "settings:find"),
        ]
    );
}

#[test]
fn a_preview_language_s_function_pack_asks_only_function_simplification() {
    // A Kotlin function with a literal and a query built from its
    // parameter: `queried` in Kotlin. Hardcoded values and the security
    // rules know no Kotlin sites, sources or sinks, so of the five rules
    // only function simplification asks, in the pack it sends alone.
    let kotlin = "fun find(db: Database, table: String): Int {\n    var total = 0\n    for (row in db.query(\"SELECT id FROM $table\")) {\n        total += row.getInt(0)\n    }\n    val floor = maxOf(total, 40)\n    return floor\n}\n";
    let packs = |rules: &[&str]| -> Vec<Value> {
        let (project, options) = project_with(&[("Shop.kt", kotlin)], rules);
        let (_, plan) = planned(&project, &options);
        assert_eq!(stages(&plan), ["functions"], "{rules:?}");
        plan.requests.into_iter().map(|p| p.request).collect()
    };
    let every = packs(&FUNCTION_RULES);
    let keys: Vec<&String> = every[0]["questions"].as_object().unwrap().keys().collect();
    assert_eq!(keys, ["f0_look", "f0_split"]);
    assert!(every[0]["state"]["functions"][0].get("values").is_none());
    assert_eq!(every, packs(&[catalog::FUNCTION_SIMPLIFICATION]));
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
fn a_packs_questions_are_cached_one_by_one_so_a_reworded_one_is_asked_alone() {
    let (project, options) = project_with(&[("lib.rs", &queried("load"))], &FUNCTION_RULES);
    let (_, plan) = planned(&project, &options);
    let pack = plan.requests[0].request.clone();
    let context = project.context();
    let store = crate::storage::Store::open(&project.0).unwrap();
    let mut mock = crate::tests::Mock::default();
    let mut ask = |request: &Value| {
        let mut session = crate::tests::session(&options, &context, &store, &mut mock);
        session.queries(&[request]).remove(0)
    };
    ask(&pack);
    let mut reworded = pack.clone();
    reworded["questions"]["f0_interpreted"]["instructions"]["question"] =
        json!("Does `functions[0].source` build a query from outside input?");
    let receipt = ask(&reworded);
    let (body, _, cached) = receipt.result.unwrap();
    assert!(!cached);
    assert_eq!(body["answers"].as_object().unwrap().len(), 8);
    assert_eq!(
        (
            receipt.metrics.asked_questions,
            receipt.metrics.cached_questions
        ),
        (1, 7),
        "the other rules' answers about the pack come from the cache"
    );
    let sent: Vec<&String> = mock.requests[1]["questions"]
        .as_object()
        .unwrap()
        .keys()
        .collect();
    assert_eq!(sent, ["f0_interpreted"]);
    assert_eq!(mock.requests[1]["state"], pack["state"]);
}

#[test]
fn a_change_asks_every_rule_about_only_the_functions_it_touched_in_one_pack() {
    // `f0` to `f2` form one run, whose end is `f2`; the change edits `f1`.
    let source = |edited: &str| -> String {
        ["f0", "f1", "f2"]
            .iter()
            .map(|name| match *name == edited {
                true => queried(name).replace("total.max(40)", "total.max(41)"),
                false => queried(name),
            })
            .collect()
    };
    let (project, mut options) = project_with(&[("lib.rs", &source(""))], &FUNCTION_RULES);
    project.commit_all();
    project.write("lib.rs", &source("f1"));
    options.base = Some("HEAD".into());
    let (_, plan) = planned(&project, &options);
    assert_eq!(stages(&plan), ["functions"]);
    let pack = &plan.requests[0];
    assert_eq!(pack.request["state"]["functions"][0]["name"], "f1");
    assert_eq!(
        pack.request["state"]["functions"].as_array().unwrap().len(),
        1
    );
    let mut rules: Vec<&str> = pack.asked.questions.iter().map(|q| q.rule).collect();
    rules.dedup();
    assert_eq!(rules, FUNCTION_RULES);
    let units = &file_plan(&plan, "lib.rs").units;
    assert_eq!(units.len(), FUNCTION_RULES.len());
    assert!(units.iter().all(|u| u.name == "f1"));
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
