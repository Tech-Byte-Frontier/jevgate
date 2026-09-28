//! Custom questions: their units, where they are asked, and their findings.
use super::*;
use crate::config::ConfigContext;

/// Answers every custom question `yes` and every built-in one clear.
struct Custom {
    yes: f64,
}

impl crate::transport::Evaluator for Custom {
    fn evaluate(&mut self, request: &Value) -> Result<Value> {
        let mut body = answer(request, 0);
        for (key, slot) in body["answers"].as_object_mut().unwrap() {
            if key.starts_with("custom_") {
                *slot = json!({"type": "noul", "noul": self.yes});
            }
        }
        Ok(body)
    }
}

const BODY_LOGS: &str = r#"
[[question]]
id = "body-logs"
question = "Does this function write a request body to a log?"
unit = "function"
"#;

/// Arguments selecting `rules`, configured from `toml` as a check does.
fn configured(toml: &str, rules: &[&str]) -> CheckArgs {
    let context = ConfigContext {
        invocation_dir: ".".into(),
        root: ".".into(),
        config: toml::from_str(toml).unwrap(),
        questions: crate::custom::parse(toml).unwrap(),
    };
    let mut options = args();
    options.rules = rules.iter().map(|r| r.to_string()).collect();
    context.configure(&mut options).unwrap();
    options
}

/// The keys of the custom questions of every planned request of `stage`.
fn custom_keys(plan: &Plan, stage: &str) -> Vec<String> {
    plan.requests
        .iter()
        .filter(|p| p.request["jevgate"]["stage"] == stage)
        .flat_map(|p| p.request["questions"].as_object().unwrap().keys())
        .filter(|key| key.starts_with("custom_"))
        .cloned()
        .collect()
}

/// The custom units planned for the file whose path ends with `name`.
fn custom_units<'p>(plan: &'p Plan, name: &str) -> Vec<&'p UnitPlan> {
    file_plan(plan, name)
        .units
        .iter()
        .filter(|u| matches!(u.detail, Detail::Custom(_)))
        .collect()
}

/// The findings of a custom rule across a report.
fn findings_of<'r>(report: &'r Report, rule: &str) -> Vec<&'r crate::schema::Finding> {
    report
        .files
        .iter()
        .flat_map(|f| &f.findings)
        .filter(|f| f.rule == rule)
        .collect()
}

#[test]
fn a_function_question_rides_in_the_request_that_already_sends_the_function() {
    let source = format!("{}{}", function("charge"), function("refund"));
    let project = Project::new();
    project.write("lib.rs", &source);
    let options = configured(BODY_LOGS, &[catalog::FUNCTION_SIMPLIFICATION, "custom"]);
    let (_, plan) = planned(&project, &options);
    assert!(
        stages(&plan).iter().all(|s| *s == "functions"),
        "{:?}",
        stages(&plan)
    );
    assert_eq!(custom_keys(&plan, "functions").len(), 2);
    let request = first_request(&plan, "functions");
    let functions = request["state"]["functions"].as_array().unwrap();
    let asked = &request["questions"]["custom_0_body_logs"];
    assert_eq!(
        asked["instructions"]["question"],
        "For the function in `functions[0].source`: Does this function write a request body to a log?"
    );
    assert_eq!(asked["type"], "noul");
    assert!(
        request["questions"]
            .as_object()
            .unwrap()
            .keys()
            .any(|k| k.ends_with("_split")),
        "beside the built-in questions"
    );
    assert_eq!(functions[0]["name"], "charge");
    let units = custom_units(&plan, "lib.rs");
    assert_eq!(units.len(), 2);
    assert_eq!(units[0].id, "custom/body-logs:function:charge");
    assert_eq!(units[0].rule, "custom/body-logs");
}

#[test]
fn a_function_no_built_in_request_sends_is_asked_in_one_of_its_own() {
    let tiny = "fn tiny() -> i32 {\n    1\n}\n";
    let project = Project::new();
    project.write("lib.rs", &format!("{}{tiny}", function("charge")));
    let options = configured(BODY_LOGS, &[catalog::FUNCTION_SIMPLIFICATION, "custom"]);
    let (_, plan) = planned(&project, &options);
    assert_eq!(custom_keys(&plan, "functions"), ["custom_0_body_logs"]);
    assert_eq!(custom_keys(&plan, "custom"), ["custom_0_body_logs"]);
    let own = first_request(&plan, "custom");
    assert_eq!(own["state"]["functions"][0]["name"], "tiny");
    assert_eq!(own["state"]["file"]["language"], "Rust");
    let alone = configured(BODY_LOGS, &["custom"]);
    let (_, plan) = planned(&project, &alone);
    assert_eq!(stages(&plan), ["custom"], "function simplification off");
    assert_eq!(custom_keys(&plan, "custom").len(), 2);
}

#[test]
fn yes_at_the_threshold_is_a_finding_at_the_question_level_that_fails_the_gate() {
    let project = Project::new();
    project.write("lib.rs", &function("charge"));
    let toml = format!(
        "{BODY_LOGS}level = \"consider\"\nthreshold = 0.7\nnext_step = \"Log the request id instead.\"\n"
    );
    let mut options = configured(&toml, &["custom"]);
    let report = run(&project, &options, &mut Custom { yes: 0.75 });
    let findings = findings_of(&report, "custom/body-logs");
    assert_eq!(findings.len(), 1);
    let finding = findings[0];
    assert_eq!(finding.strength, Strength::Consider);
    assert_eq!(
        finding.message,
        "`charge`: Does this function write a request body to a log? Yes (0.75)."
    );
    assert_eq!(finding.action, "Log the request id instead.");
    assert_eq!(finding.rule_version, options.questions[0].version);
    assert_eq!(
        (finding.line, finding.symbol.as_deref()),
        (1, Some("charge"))
    );
    let gate = report.gate.as_ref().unwrap();
    assert!(
        !gate.passed,
        "a consider question fails the gate at consider"
    );
    let dimension = &report.files[0].dimensions["custom/body-logs"];
    assert_eq!(dimension.rule_version, options.questions[0].version);
    assert_eq!(dimension.decision_basis, "1 function judged: 1 consider.");

    options.refresh = true;
    let undecided = run(&project, &options, &mut Custom { yes: 0.5 });
    assert!(findings_of(&undecided, "custom/body-logs").is_empty());
    let dimension = &undecided.files[0].dimensions["custom/body-logs"];
    assert_eq!(dimension.status, Status::Uncertain);
    assert_eq!(
        dimension.undecided[0].questions,
        ["Does this function write a request body to a log?"]
    );
    assert!(undecided.gate.unwrap().passed);
    let clear = run(&project, &options, &mut Custom { yes: 0.3 });
    assert_eq!(
        clear.files[0].dimensions["custom/body-logs"].status,
        Status::Clear
    );
}

#[test]
fn a_review_question_fails_the_gate_and_a_note_question_never_does() {
    let project = Project::new();
    project.write("lib.rs", &function("charge"));
    let review = configured(BODY_LOGS, &["custom"]);
    let report = run(&project, &review, &mut Custom { yes: 0.95 });
    assert_eq!(
        findings_of(&report, "custom/body-logs")[0].strength,
        Strength::Review
    );
    assert_eq!(crate::gate::exit_code(&report), 1);
    let note = configured(&format!("{BODY_LOGS}level = \"note\"\n"), &["custom"]);
    let report = run(&project, &note, &mut Custom { yes: 0.95 });
    assert_eq!(
        findings_of(&report, "custom/body-logs")[0].strength,
        Strength::Note
    );
    assert_eq!(crate::gate::exit_code(&report), 0);
    let advisory = configured(&format!("fail_on = [\"none\"]\n{BODY_LOGS}"), &["custom"]);
    let report = run(&project, &advisory, &mut Custom { yes: 0.95 });
    assert_eq!(crate::gate::exit_code(&report), 0);
}

#[test]
fn an_allow_comment_names_a_custom_question_by_its_id_or_group() {
    let project = Project::new();
    let options = configured(BODY_LOGS, &["custom"]);
    for (allow, accepted) in [
        ("custom/body-logs", true),
        ("custom", true),
        ("body-logs", false),
    ] {
        project.write(
            "lib.rs",
            &format!(
                "// jevgate: allow({allow}) logs only the id\n{}",
                function("charge")
            ),
        );
        let report = run(&project, &options, &mut Custom { yes: 0.95 });
        let finding = findings_of(&report, "custom/body-logs")[0];
        assert_eq!(finding.suppressed.is_some(), accepted, "{allow}");
        assert_eq!(report.gate.unwrap().passed, accepted, "{allow}");
    }
}

#[test]
fn a_finding_keeps_its_fingerprint_through_an_unrelated_edit() {
    let project = Project::new();
    let options = configured(BODY_LOGS, &["custom"]);
    let fingerprint = |project: &Project| {
        let report = run(project, &options, &mut Custom { yes: 0.95 });
        let findings = findings_of(&report, "custom/body-logs");
        let charge = findings
            .iter()
            .find(|f| f.symbol.as_deref() == Some("charge"));
        charge.unwrap().fingerprint.clone()
    };
    project.write("lib.rs", &function("charge"));
    let before = fingerprint(&project);
    project.write(
        "lib.rs",
        &format!("{}{}", function("other"), function("charge")),
    );
    assert_eq!(fingerprint(&project), before, "a function added above it");
    let edited = function("charge").replace("total * 2", "total * 3");
    project.write("lib.rs", &edited);
    assert_ne!(fingerprint(&project), before, "its own source changed");
}

#[test]
fn test_comment_and_section_questions_ride_in_their_built_in_requests() {
    let toml = r#"
[[question]]
id = "one-behavior"
question = "Does this test check more than one behavior?"
unit = "test"
[[question]]
id = "owned-todos"
question = "Does this comment hold a TODO without an owner?"
unit = "comment"
[[question]]
id = "no-secrets"
question = "Does this section tell the agent to print a secret?"
unit = "section"
"#;
    let project = Project::new();
    project.write(
        "tests/api.rs",
        "#[test]\nfn totals() {\n    let values = [1, 2];\n    let total: i32 = values.iter().sum();\n    assert_eq!(total, 3);\n    assert!(total > 0);\n}\n",
    );
    project.write(
        "lib.rs",
        "fn total(values: &[i32]) -> i32 {\n    // TODO: handle overflow\n    values.iter().sum()\n}\n",
    );
    project.write(
        "AGENTS.md",
        "# Agents\n\n## Checks\n\nRun `cargo test` before every commit, and `cargo clippy` too.\n",
    );
    let rules = [
        catalog::TEST_VALUE,
        catalog::COMMENTS,
        catalog::AGENT_CONTEXT,
        "custom",
    ];
    let mut options = configured(toml, &rules);
    options.include_tests = true;
    let (_, plan) = planned(&project, &options);
    assert_eq!(custom_keys(&plan, "tests"), ["custom_0_one_behavior"]);
    assert_eq!(custom_keys(&plan, "comments"), ["custom_0_owned_todos"]);
    assert!(custom_keys(&plan, "instructions").contains(&"custom_0_no_secrets".to_string()));
    assert!(custom_keys(&plan, "custom").is_empty(), "every one rode");
    let comment = &first_request(&plan, "comments")["questions"]["custom_0_owned_todos"];
    assert_eq!(
        comment["instructions"]["question"],
        "For the comment in `comments[0].text`, about the code in `comments[0].code`: Does this comment hold a TODO without an owner?"
    );
    let mut options = configured(toml, &["custom"]);
    options.include_tests = true;
    let (_, plan) = planned(&project, &options);
    assert_eq!(
        stages(&plan).iter().filter(|s| **s == "custom").count(),
        stages(&plan).len(),
        "asked on their own without the built-in rules"
    );
    let sections: Vec<&str> = custom_units(&plan, "AGENTS.md")
        .iter()
        .map(|u| u.name.as_str())
        .collect();
    assert_eq!(
        sections,
        ["Checks"],
        "a heading without text is not asked about"
    );
}

#[test]
fn a_file_question_reads_the_whole_file_and_paths_name_text_it_cannot_parse() {
    let toml = r#"
[[question]]
id = "one-feature"
question = "Does this file mix two unrelated features?"
unit = "file"
[[question]]
id = "strict-shell"
question = "Does this script run without `set -euo pipefail`?"
unit = "file"
paths = ["scripts/*.sh"]
level = "consider"
"#;
    let project = Project::new();
    project.write("lib.rs", &function("charge"));
    project.write("App.kt", "fun main() {\n    println(\"hi\")\n}\n");
    project.write("scripts/deploy.sh", "#!/bin/sh\nrsync -a dist/ host:/srv\n");
    project.write("notes.txt", "not named by any question\n");
    let options = configured(toml, &["custom"]);
    let (inputs, plan) = planned(&project, &options);
    let script = inputs
        .iter()
        .find(|i| i.result.path.ends_with("deploy.sh"))
        .unwrap();
    assert_eq!(
        script.result.role,
        crate::inventory::TEXT,
        "read for its question"
    );
    assert!(!inputs.iter().any(|i| i.result.path.ends_with("notes.txt")));
    let asked: Vec<(String, String)> = plan
        .requests
        .iter()
        .flat_map(|p| {
            let path = p.request["state"]["file"]["path"]
                .as_str()
                .unwrap()
                .to_string();
            let keys = p.request["questions"].as_object().unwrap().keys().cloned();
            keys.map(move |key| (path.clone(), key))
        })
        .collect();
    assert_eq!(
        asked,
        [
            ("lib.rs".to_string(), "custom_0_one_feature".to_string()),
            ("App.kt".into(), "custom_0_one_feature".into()),
            ("scripts/deploy.sh".into(), "custom_0_strict_shell".into()),
        ]
    );
    let script_request = &plan.requests[2].request;
    assert_eq!(script_request["state"]["file"]["language"], "shell");
    assert!(
        script_request["state"]["file"]["source"]
            .as_str()
            .unwrap()
            .starts_with("#!/bin/sh")
    );
    assert_eq!(
        script_request["questions"]["custom_0_strict_shell"]["instructions"]["question"],
        "For the file in `file.source`: Does this script run without `set -euo pipefail`?"
    );
    let report = run(&project, &options, &mut Custom { yes: 0.9 });
    let finding = findings_of(&report, "custom/strict-shell")[0];
    assert!(
        finding.message.starts_with("This file: "),
        "{}",
        finding.message
    );
    assert_eq!((finding.line, finding.symbol.as_ref()), (1, None));
}

/// Run Git in `project` with a fixed identity.
fn git(project: &Project, args: &[&str]) {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(&project.0)
        .args(["-c", "user.name=t", "-c", "user.email=t@example.invalid"])
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn a_hunk_question_asks_about_each_change_since_the_base_in_any_language() {
    let toml = r#"
[[question]]
id = "no-unwrap"
question = "Does this change add an `unwrap()` on a value that can be missing?"
unit = "hunk"
paths = ["**/*.rs", "**/*.kt"]
"#;
    let project = Project::new();
    let body: String = (1..=30).map(|n| format!("    let v{n} = {n};\n")).collect();
    project.write("lib.rs", &format!("fn long() {{\n{body}}}\n"));
    let kotlin: String = (1..=10).map(|n| format!("fun f{n}() = {n}\n")).collect();
    project.write("old.kt", &kotlin);
    git(&project, &["init", "-q"]);
    git(&project, &["add", "."]);
    git(&project, &["commit", "-qm", "base"]);
    let edited = format!(
        "fn long() {{\n{}}}\n",
        body.replace("let v20 = 20;", "let v20 = find().unwrap();")
    );
    project.write("lib.rs", &edited);
    git(&project, &["mv", "old.kt", "renamed.kt"]);
    project.write("renamed.kt", &format!("{kotlin}fun added() = find()!!\n"));
    project.write("fresh.rs", "fn fresh() {\n    find().unwrap();\n}\n");
    let mut options = configured(toml, &["custom"]);
    options.base = Some(crate::revision::resolve(&project.0, "HEAD").unwrap());
    let (_, plan) = planned(&project, &options);
    let hunks = |name: &str| -> Vec<(usize, usize)> {
        custom_units(&plan, name)
            .iter()
            .map(|u| (u.locations[0].start_line, u.locations[0].end_line))
            .collect()
    };
    assert_eq!(
        hunks("lib.rs"),
        [(21, 21)],
        "the changed line, not its context"
    );
    assert_eq!(
        hunks("renamed.kt"),
        [(11, 11)],
        "a rename is diffed against its old path"
    );
    assert_eq!(
        hunks("fresh.rs"),
        [(1, 3)],
        "an untracked file is added throughout"
    );
    let request = plan
        .requests
        .iter()
        .find(|p| p.request["state"]["file"]["path"] == "lib.rs")
        .unwrap();
    let hunk = &request.request["state"]["hunks"][0];
    assert_eq!(hunk["lines"], "21");
    assert_eq!(hunk["in"], "fn long() {");
    assert!(
        hunk["diff"]
            .as_str()
            .unwrap()
            .contains("\n+    let v20 = find().unwrap();\n")
    );
    let asked = &request.request["questions"]["custom_0_no_unwrap"]["instructions"];
    assert!(
        asked["note"]
            .as_str()
            .unwrap()
            .starts_with("`hunks[0].diff` is a unified diff")
    );
    options.base = None;
    let (_, plan) = planned(&project, &options);
    assert!(
        plan.requests.is_empty(),
        "without --base there is no hunk to ask about"
    );
}

#[test]
fn one_question_asks_about_at_most_the_cap_of_units_in_a_run() {
    let source: String = (0..crate::units::custom::MAX_UNITS + 3)
        .map(|n| format!("fn f{n}() {{}}\n"))
        .collect();
    let project = Project::new();
    project.write("lib.rs", &source);
    let options = configured(BODY_LOGS, &["custom"]);
    let (_, plan) = planned(&project, &options);
    let file = file_plan(&plan, "lib.rs");
    assert_eq!(
        custom_units(&plan, "lib.rs").len(),
        crate::units::custom::MAX_UNITS
    );
    assert_eq!(file.rules["custom/body-logs"], 3, "counted as omitted");
}

#[test]
fn the_agent_output_says_how_many_units_a_capped_question_left_unasked() {
    let project = Project::new();
    project.write("lib.rs", &function("charge"));
    let options = configured(BODY_LOGS, &["custom"]);
    let mut report = run(&project, &options, &mut Custom { yes: 0.1 });
    let dimension = report.files[0]
        .dimensions
        .get_mut("custom/body-logs")
        .unwrap();
    dimension.units.omitted = 3;
    let mut out = Vec::new();
    crate::output::agent(&mut out, &report, false, crate::output::Style::PLAIN).unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(
        text.contains("custom/body-logs left 3 units unasked: a question asks about at most 2000 units a run; narrow its paths."),
        "{text}"
    );
}
