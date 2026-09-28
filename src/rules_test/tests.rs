use super::*;
use crate::tests::{Project, answer};
use serde_json::json;

/// Answers each custom question `yes` when its unit's evidence holds
/// `marker`, else `no`, and counts the requests it answers.
struct Marked {
    marker: &'static str,
    yes: f64,
    no: f64,
    requests: usize,
}

impl Marked {
    fn new(marker: &'static str) -> Self {
        Self {
            marker,
            yes: 0.95,
            no: 0.05,
            requests: 0,
        }
    }
}

impl Evaluator for Marked {
    fn evaluate(&mut self, request: &Value) -> Result<Value> {
        self.requests += 1;
        let mut body = answer(request, 0);
        let state = &request["state"];
        for (key, slot) in body["answers"].as_object_mut().unwrap() {
            let index: usize = key.split('_').nth(1).unwrap().parse().unwrap();
            let unit = ["functions", "tests", "comments", "sections", "hunks"]
                .iter()
                .find_map(|list| state[list].get(index))
                .unwrap_or(&state["file"]);
            let yes = if unit.to_string().contains(self.marker) {
                self.yes
            } else {
                self.no
            };
            *slot = json!({"type": "noul", "noul": yes});
        }
        Ok(body)
    }
}

const QUESTION: &str = r#"
[[question]]
id = "no-body-logs"
question = "Does this function write a request body to a log?"
unit = "function"
"#;

/// A failing example that logs a body and a passing one that does not.
const EXAMPLES: &str = r#"
[[question.failing]]
path = "src/orders.rs"
code = "fn charge(req: &Request) {\n    log(req.body());\n}\n"

[[question.passing]]
path = "src/orders.rs"
code = "fn charge(req: &Request) {\n    log(req.id());\n}\n"
"#;

/// A project configured by `toml`, read as a check reads it.
fn project(toml: &str) -> (Project, ConfigContext) {
    let project = Project::new();
    let context = configured(&project, toml);
    (project, context)
}

/// `project` with `toml` as its configuration.
fn configured(project: &Project, toml: &str) -> ConfigContext {
    project.write("jevgate.toml", toml);
    let mut context = project.context();
    context.config = toml::from_str(toml).unwrap();
    context.questions = crate::custom::parse(toml).unwrap();
    context
}

/// `rules test` with `flags`, `evaluator` answering.
fn tested(context: &ConfigContext, flags: &[&str], evaluator: &mut Marked) -> Result<Report> {
    #[derive(clap::Parser)]
    struct Cli {
        #[command(flatten)]
        test: RulesTestArgs,
    }
    let test =
        <Cli as clap::Parser>::parse_from(std::iter::once("test").chain(flags.iter().copied()))
            .test;
    let args = arguments(&test, context)?;
    examine(&test.rules, &args, context, evaluator)
}

/// The JSON report, and the examples of its first question.
fn reported(report: &Report) -> (Value, Vec<Value>) {
    let json = serde_json::to_value(report).unwrap();
    let examples = json["questions"][0]["examples"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    (json, examples)
}

fn table(report: &Report) -> String {
    let mut out = Vec::new();
    report.table(&mut out).unwrap();
    String::from_utf8(out).unwrap()
}

#[test]
fn a_question_that_separates_its_examples_passes_and_a_rerun_asks_nothing() {
    let (_project, context) = project(&format!("{QUESTION}{EXAMPLES}"));
    let mut evaluator = Marked::new("req.body");
    let report = tested(&context, &[], &mut evaluator).unwrap();
    assert_eq!(report.exit_code(), 0);
    let (json, examples) = reported(&report);
    assert_eq!(
        (json["complete"].clone(), json["passed"].clone()),
        (json!(true), json!(true))
    );
    assert_eq!(json["api_requests"], 2);
    assert_eq!(json["models"], json!(["jev-1.13.0"]));
    assert_eq!(examples[0]["expected"], "failing");
    assert_eq!(
        (
            examples[0]["result"].clone(),
            examples[0]["yes"].clone(),
            examples[0]["found"].clone()
        ),
        (json!("right"), json!(0.95), json!(true))
    );
    assert_eq!(examples[0]["unit"], "`charge`");
    assert_eq!(
        (examples[1]["result"].clone(), examples[1]["found"].clone()),
        (json!("right"), json!(false))
    );
    assert_eq!(examples[1]["cached"], false);
    let text = table(&report);
    assert!(
        text.starts_with("JevGate: rules test · all 2 examples right · 1 question · 2 API requests · 20 input tokens · ~$0.0000 · answered by jev-1.13.0\n"),
        "{text}"
    );
    assert!(text.contains("\ncustom/no-body-logs (function, review at 0.80): all 2 examples right\n  ok     failing 1  yes 0.95  src/orders.rs: `charge`\n  ok     passing 1  yes 0.05  src/orders.rs: `charge`\n"), "{text}");
    let rerun = tested(&context, &[], &mut evaluator).unwrap();
    assert_eq!(evaluator.requests, 2, "a rerun is answered from the cache");
    let (json, examples) = reported(&rerun);
    assert_eq!(
        (json["api_requests"].clone(), examples[1]["cached"].clone()),
        (json!(0), json!(true))
    );
    assert_eq!(rerun.exit_code(), 0);
}

#[test]
fn an_example_the_question_gets_wrong_fails_and_says_what_a_check_would_do() {
    let (_project, context) = project(&format!("{QUESTION}{EXAMPLES}"));
    let report = tested(&context, &[], &mut Marked::new("req.id")).unwrap();
    assert_eq!(report.exit_code(), 1);
    let (json, examples) = reported(&report);
    assert_eq!(json["passed"], false);
    assert_eq!(
        (examples[0]["result"].clone(), examples[0]["found"].clone()),
        (json!("wrong"), json!(false)),
        "the failing example is missed"
    );
    assert_eq!(
        (examples[1]["result"].clone(), examples[1]["found"].clone()),
        (json!("wrong"), json!(true)),
        "the passing one is found"
    );
    let text = table(&report);
    assert!(text.contains(" · 2 of 2 examples wrong · "), "{text}");
    assert!(
        text.contains(
            "  wrong  failing 1  yes 0.05  src/orders.rs: `charge` (a check misses it below 0.80)\n"
        ),
        "{text}"
    );
    assert!(text.contains("  wrong  passing 1  yes 0.95  src/orders.rs: `charge` (a check reports it at 0.80 or more)\n"), "{text}");
}

#[test]
fn a_new_model_or_a_reworded_question_asks_the_examples_again() {
    let (project, context) = project(&format!("{QUESTION}{EXAMPLES}"));
    let mut evaluator = Marked::new("req.body");
    tested(&context, &[], &mut evaluator).unwrap();
    let moved = tested(&context, &["--model", "jev-latest"], &mut evaluator).unwrap();
    assert_eq!(evaluator.requests, 4, "answers are cached per model");
    assert_eq!(reported(&moved).0["requested_model"], "jev-latest");
    let reworded = format!("{QUESTION}guidance = \"Logging an id is fine.\"\n{EXAMPLES}");
    tested(&configured(&project, &reworded), &[], &mut evaluator).unwrap();
    assert_eq!(
        evaluator.requests, 6,
        "the reworded question is asked again"
    );
}

#[test]
fn an_example_that_cannot_be_asked_leaves_the_run_incomplete() {
    let (_project, context) = project(&format!("{QUESTION}{EXAMPLES}"));
    let mut evaluator = Marked::new("req.body");
    let report = tested(&context, &["--cache-only"], &mut evaluator).unwrap();
    assert_eq!((report.exit_code(), evaluator.requests), (2, 0));
    let (json, examples) = reported(&report);
    assert_eq!(
        (json["complete"].clone(), json["passed"].clone()),
        (json!(false), json!(false))
    );
    assert_eq!(examples[0]["result"], "error");
    assert!(
        examples[0]["error"]
            .as_str()
            .unwrap()
            .contains("No current cached response")
    );
    assert!(
        table(&report)
            .starts_with("JevGate: rules test · incomplete: 2 of 2 examples not answered · ")
    );

    let files = r#"
upload_deny = ["private/**"]

[[question]]
id = "no-body-logs"
question = "Does this function write a request body to a log?"
unit = "function"

[[question.failing]]
file = "examples/missing.rs"

[[question.failing]]
file = "private/charge.rs"

[[question.failing]]
file = "examples/latin1.rs"

[[question.passing]]
file = "examples/charge.rs"
"#;
    let (project, context) = project(files);
    project.write("private/charge.rs", "fn charge() {\n    log(body);\n}\n");
    project.write("examples/charge.rs", "fn charge() {\n    log(id);\n}\n");
    std::fs::write(
        project.0.join("examples/latin1.rs"),
        b"// caf\xe9\nfn f() {}\n",
    )
    .unwrap();
    let report = tested(&context, &[], &mut evaluator).unwrap();
    assert_eq!(report.exit_code(), 2);
    let (_, examples) = reported(&report);
    let errors: Vec<&str> = examples
        .iter()
        .map(|e| e["error"].as_str().unwrap_or(""))
        .collect();
    assert!(
        errors[0].starts_with("Cannot read examples/missing.rs"),
        "{errors:?}"
    );
    assert!(
        errors[1].contains("private/charge.rs is outside upload_allow or inside upload_deny"),
        "{errors:?}"
    );
    assert!(errors[2].contains("not UTF-8"), "{errors:?}");
    assert_eq!(
        (examples[3]["result"].clone(), examples[3]["file"].clone()),
        (json!("right"), json!("examples/charge.rs"))
    );
    assert_eq!(
        evaluator.requests, 1,
        "only the example that can be read is asked"
    );
    #[cfg(unix)]
    {
        project.write("secret.txt", "TYPESAFE_API_KEY=do-not-expose\n");
        std::fs::remove_file(project.0.join("examples/charge.rs")).unwrap();
        std::os::unix::fs::symlink(
            project.0.join("secret.txt"),
            project.0.join("examples/charge.rs"),
        )
        .unwrap();
        let report = tested(&context, &[], &mut evaluator).unwrap();
        let (_, examples) = reported(&report);
        assert!(
            examples[3]["error"].as_str().unwrap().contains("symlinks"),
            "{examples:?}"
        );
        assert_eq!(evaluator.requests, 1, "a linked file is never sent");
    }
}

#[test]
fn a_dry_run_counts_what_the_cache_lacks_and_writes_nothing() {
    let (project, context) = project(&format!("{QUESTION}{EXAMPLES}"));
    let mut evaluator = Marked::new("req.body");
    let dry = tested(&context, &["--dry-run"], &mut evaluator).unwrap();
    let (json, examples) = reported(&dry);
    assert_eq!(
        (
            json["planned"]["requests"].clone(),
            json["planned"]["cached"].clone()
        ),
        (json!(2), json!(0))
    );
    assert!(json["planned"]["new_input_tokens"].as_u64().unwrap() > 0);
    assert_eq!(
        (json["passed"].clone(), examples[0]["result"].clone()),
        (json!(null), json!("ready"))
    );
    assert_eq!((dry.exit_code(), evaluator.requests), (0, 0));
    assert!(
        !project.0.join(".jevgate").exists(),
        "a dry run writes no state"
    );
    tested(&context, &[], &mut evaluator).unwrap();
    let warm = reported(&tested(&context, &["--dry-run"], &mut evaluator).unwrap()).0;
    assert_eq!(
        (
            warm["planned"]["cached"].clone(),
            warm["planned"]["new_input_tokens"].clone()
        ),
        (json!(2), json!(0))
    );
    let broken = format!(
        "{QUESTION}[[question.failing]]\npath = \"src/a.rs\"\ncode = \"const A: u32 = 1;\"\n"
    );
    let report = tested(
        &configured(&project, &broken),
        &["--dry-run"],
        &mut evaluator,
    )
    .unwrap();
    assert_eq!(
        report.exit_code(),
        2,
        "a dry run checks every example can be asked"
    );
    let text = table(&report);
    assert!(
        text.contains("): 1 of 1 example cannot be asked\n  error  failing 1            src/a.rs: it holds no function"),
        "{text}"
    );
}

#[test]
fn rules_select_questions_and_those_without_examples_are_listed() {
    let toml = format!(
        "{QUESTION}{EXAMPLES}\n[[question]]\nid = \"owned-todos\"\nquestion = \"Does this comment hold a TODO without an owner?\"\nunit = \"comment\"\n"
    );
    let (_project, context) = project(&toml);
    let mut evaluator = Marked::new("req.body");
    let report = tested(&context, &[], &mut evaluator).unwrap();
    let (json, _) = reported(&report);
    assert_eq!(json["untested"], json!(["custom/owned-todos"]));
    assert!(table(&report).ends_with("\nWithout examples: custom/owned-todos.\n"));
    let grouped = reported(&tested(&context, &["--rule", "custom"], &mut evaluator).unwrap()).0;
    assert_eq!(grouped["questions"].as_array().unwrap().len(), 1);
    for (rule, problem) in [
        (
            "custom/owned-todos",
            "custom/owned-todos has no examples: add [[question.failing]]",
        ),
        (
            "maintainability",
            "maintainability names no custom question",
        ),
        ("custom/nope", "custom/nope names no custom question"),
    ] {
        let error = tested(&context, &["--rule", rule], &mut evaluator)
            .err()
            .unwrap()
            .to_string();
        assert!(error.starts_with(problem), "{error}");
    }
    let (_, empty) = project("");
    let report = tested(&empty, &[], &mut evaluator).unwrap();
    assert_eq!(report.exit_code(), 0);
    assert_eq!(
        table(&report),
        "JevGate: rules test · no custom question has examples\n"
    );
}

#[test]
fn an_example_is_found_when_any_of_its_units_is_and_undecided_passes() {
    let two = "fn charge(req: &Request) {\n    log(req.body());\n}\n\nfn refund(req: &Request) {\n    log(req.id());\n}\n";
    let toml = format!(
        "{QUESTION}[[question.failing]]\npath = \"src/orders.rs\"\ncode = '''\n{two}'''\n[[question.passing]]\npath = \"src/orders.rs\"\ncode = '''\n{two}'''\n"
    );
    let (_project, context) = project(&toml);
    let report = tested(&context, &[], &mut Marked::new("req.body")).unwrap();
    let (_, examples) = reported(&report);
    assert_eq!(examples[0]["result"], "right");
    assert_eq!(examples[1]["result"], "wrong", "one unit is a finding");
    assert_eq!(
        examples[1]["unit"], "`charge`",
        "the unit that leans most to yes"
    );
    assert_eq!(
        examples[1]["units"],
        json!([{"unit": "`charge`", "yes": 0.95, "found": true}, {"unit": "`refund`", "yes": 0.05, "found": false}])
    );
    let mut undecided = Marked::new("req.body");
    (undecided.yes, undecided.no) = (0.83, 0.5);
    let (_project, context) = project(&format!("{QUESTION}{EXAMPLES}"));
    let report = tested(&context, &[], &mut undecided).unwrap();
    assert_eq!(
        report.exit_code(),
        0,
        "an undecided passing example is not a finding"
    );
    let (_, examples) = reported(&report);
    assert_eq!(
        (examples[0]["close"].clone(), examples[1]["close"].clone()),
        (json!(true), json!(false))
    );
    assert!(
        table(&report).contains(
            "  ok     failing 1  yes 0.83  src/orders.rs: `charge` (within 0.10 of 0.80)\n"
        )
    );
    let mut exactly = Marked::new("req.body");
    exactly.yes = 0.7;
    let (_, examples) = reported(&tested(&context, &["--refresh"], &mut exactly).unwrap());
    assert_eq!(
        (examples[0]["result"].clone(), examples[0]["close"].clone()),
        (json!("wrong"), json!(false)),
        "0.10 away is not within 0.10"
    );
}
