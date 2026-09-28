//! Across rules: what a request uploads, how units are packed, too-small units
//! and composition from saved judgments.
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
    assert!(rules.len() >= 3, "{rules:?}");
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
    let units = &file.dimensions["function_simplification"].units;
    assert_eq!((units.judged, units.uncertain), (1, 1));
}
