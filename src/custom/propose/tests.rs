//! `jevgate rules propose` and `accept`, with scripted answers.
use super::{
    PROPOSALS, Printed, accept,
    files::File,
    lines::{Line, lines},
    proposal,
};
use crate::{
    config::ConfigContext,
    custom::{self, Kind},
    options::{CheckArgs, ProposeArgs, ProposeFormat},
    tests::{Project, answer},
    transport::Evaluator,
    units::questions::{PROPOSAL_CHECKERS, PROPOSAL_UNITS},
};
use anyhow::Result;
use serde_json::{Value, json};
use std::path::PathBuf;

/// An AGENTS.md with rules, a command, a fact, a nested list, a table, code,
/// a comment and an import.
const AGENTS: &str = "\
# Conventions

Prefer `undefined` for absent values.

Handlers follow these rules:

- Never log request bodies.
- Run `cargo test` before pushing.
- **Errors**:
  - Never swallow an error without a log line.
  - Wrap errors with context
    about the call that failed.

| Command | What |
| --- | --- |
| `make` | builds |

```sh
- not an item
```
<!-- - Never read this: a comment -->
@docs/more.md
";

/// The ids `AGENTS` gets: the rules starting with \"Never\".
const IDS: [&str; 2] = [
    "never-log-request-bodies",
    "never-swallow-an-error-without-a-log",
];

/// Answers that a line is a rule when it starts with "Never", that a
/// function shows it, and that a reviewer checks it, unless it is about
/// characters per line, which a formatter checks, or locales, which a test
/// run checks.
#[derive(Default)]
struct Rules {
    calls: usize,
}

impl Evaluator for Rules {
    fn evaluate(&mut self, request: &Value) -> Result<Value> {
        self.calls += 1;
        let mut body = answer(request, 0);
        let candidates = request["state"]["candidates"].as_array().unwrap();
        for (key, slot) in body["answers"].as_object_mut().unwrap() {
            let (position, question) = key[1..].split_once('_').unwrap();
            let text = candidates[position.parse::<usize>().unwrap()]["text"]
                .as_str()
                .unwrap();
            *slot = match question {
                "convention" => {
                    json!({"type": "noul", "noul": if text.starts_with("Never") { 0.95 } else { 0.05 }})
                }
                "unit" => likely("function", &PROPOSAL_UNITS),
                _ if text.contains("characters per line") => likely("tool", &PROPOSAL_CHECKERS),
                _ if text.contains("locales") => likely("run", &PROPOSAL_CHECKERS),
                _ => likely("reviewer", &PROPOSAL_CHECKERS),
            };
        }
        Ok(body)
    }
}

/// A Choice of `chosen` at 0.9, the rest spread over the other options.
fn likely(chosen: &str, options: &[(&str, &str)]) -> Value {
    let rest = 0.1 / (options.len() - 1) as f64;
    let probabilities: serde_json::Map<String, Value> = options
        .iter()
        .map(|(option, _)| {
            (
                option.to_string(),
                json!(if *option == chosen { 0.9 } else { rest }),
            )
        })
        .collect();
    json!({"type": "choice", "choice": chosen, "confidence": 0.9, "probabilities": probabilities})
}

/// A provider that refuses every request.
struct Refused;

impl Evaluator for Refused {
    fn evaluate(&mut self, _: &Value) -> Result<Value> {
        anyhow::bail!("HTTP 402: credits exhausted")
    }
}

fn texts(found: &[Line]) -> Vec<(usize, &str, Option<&str>)> {
    found
        .iter()
        .map(|l| (l.start_line, l.text.as_str(), l.lead_in.as_deref()))
        .collect()
}

#[test]
fn candidates_are_list_items_at_any_depth_and_paragraphs_with_what_introduces_them() {
    let found = lines(AGENTS);
    let rules = Some("Handlers follow these rules:");
    assert_eq!(
        texts(&found),
        [
            (3, "Prefer `undefined` for absent values.", None),
            (7, "Never log request bodies.", rules),
            (8, "Run `cargo test` before pushing.", rules),
            (
                10,
                "Never swallow an error without a log line.",
                Some("**Errors**:")
            ),
            (
                11,
                "Wrap errors with context about the call that failed.",
                Some("**Errors**:")
            ),
        ],
        "introductions, tables, code, comments and imports are no candidates"
    );
    assert!(found.iter().all(|l| l.heading == "Conventions"));
    assert_eq!(
        found[4].end_line, 12,
        "a continuation line belongs to its item"
    );
}

#[test]
fn a_line_ending_in_a_colon_is_a_candidate_when_no_list_follows_it() {
    let found = lines("# A\n\n- Log with context:\n- Next item\n\nRun this:\n\n```sh\nmake\n```\n");
    assert_eq!(
        texts(&found),
        [
            (3, "Log with context:", None),
            (4, "Next item", None),
            (6, "Run this:", None),
        ]
    );
}

#[test]
fn a_paragraph_at_the_margin_ends_the_list_and_its_introduction() {
    let found = lines("Rules:\n- One\n\n  Still one.\n\nAfter the list.\n- Two\n");
    assert_eq!(
        texts(&found),
        [
            (2, "One", Some("Rules:")),
            (4, "Still one.", Some("One")),
            (6, "After the list.", None),
            (7, "Two", None),
        ]
    );
}

#[test]
fn frontmatter_task_boxes_quotes_and_numbered_items_are_read_as_text() {
    let source = "---\nglobs: src/**\n---\n1. First rule\n2. [x] Done rule\n> Quoted rule\n> on two lines\n* * *\nPlain text rule\n";
    assert_eq!(
        texts(&lines(source)),
        [
            (4, "First rule", None),
            (5, "Done rule", None),
            (6, "Quoted rule on two lines", None),
            (9, "Plain text rule", None),
        ]
    );
}

#[test]
fn control_and_bidirectional_characters_are_removed() {
    let found = lines("- Never \u{1b}[31mlog\u{202e} bodies\u{7}\n");
    assert_eq!(found[0].text, "Never [31mlog bodies");
}

#[test]
fn a_comment_keeps_the_line_numbers_below_it() {
    let found = lines("<!--\nhidden\n-->\n- Never log bodies\n");
    assert_eq!(texts(&found), [(4, "Never log bodies", None)]);
}

#[test]
fn a_byte_order_mark_hides_no_heading_or_frontmatter() {
    let found = lines("\u{feff}# Rules\n\n- Never log bodies\n");
    assert_eq!(texts(&found), [(3, "Never log bodies", None)]);
    assert_eq!(found[0].heading, "Rules");
    let found = lines("\u{feff}---\nglobs: src/**\n---\n- Never log bodies\n");
    assert_eq!(texts(&found), [(4, "Never log bodies", None)]);
}

/// A file of `text` as `propose` reads it.
fn file(path: &str, text: &str) -> File {
    File {
        path: PathBuf::from(path),
        source_hash: "hash".into(),
        lines: lines(text),
        scope: Vec::new(),
    }
}

/// The first pass over `files`.
fn first_pass(files: &[File]) -> super::ask::Plan {
    super::ask::plan(files, ("jev-1.13.0", super::ask::Pass::First), |_, _| true)
}

#[test]
fn each_line_is_asked_two_questions_in_a_request_without_paths_or_line_numbers() {
    let plan = first_pass(&[file("AGENTS.md", AGENTS)]);
    assert_eq!(plan.requests.len(), 1);
    let request = &plan.requests[0];
    let candidates = request["state"]["candidates"].as_array().unwrap();
    assert_eq!(candidates.len(), 5);
    assert_eq!(
        candidates[1],
        json!({"heading": "Conventions", "lead_in": "Handlers follow these rules:", "text": "Never log request bodies."})
    );
    let questions = request["questions"].as_object().unwrap();
    assert_eq!(questions.len(), 10);
    assert_eq!(questions["c1_convention"]["type"], "noul");
    assert_eq!(questions["c1_unit"]["type"], "choice");
    let text = request["questions"].to_string();
    assert!(text.contains("`candidates[1].text`"), "{text}");
    assert!(!request["state"].to_string().contains("AGENTS.md"));
    assert_eq!(request["jevgate"]["stage"], "propose");
    assert_eq!(request["jevgate"]["sources"][0]["path"], "AGENTS.md");
}

#[test]
fn a_copy_of_a_file_is_asked_once() {
    let files = [file("AGENTS.md", AGENTS), file("CLAUDE.md", AGENTS)];
    let plan = first_pass(&files);
    assert_eq!(plan.requests.len(), 1);
    assert_eq!(plan.places[0], plan.places[1]);
}

#[test]
fn every_unit_option_names_a_question_unit() {
    let kinds: Vec<Option<Kind>> = crate::units::questions::PROPOSAL_UNITS
        .iter()
        .map(|(option, _)| super::ask::kind(option))
        .collect();
    assert!(kinds.iter().all(Option::is_some), "{kinds:?}");
    assert!(kinds.contains(&Some(Kind::Hunk)));
}

/// `propose`'s arguments in `format`, for a run or a dry run.
fn arguments(format: ProposeFormat, dry_run: bool) -> ProposeArgs {
    ProposeArgs {
        paths: Vec::new(),
        format: Some(format),
        dry_run,
        show_requests: false,
        env_file: None,
    }
}

/// The context of `project`, with the questions of its questions directory.
fn context(project: &Project) -> ConfigContext {
    let root = project.0.to_path_buf();
    let questions = custom::load(
        &root,
        (&root.join("jevgate.toml"), &[]),
        Some(&custom::directory(&root)),
    )
    .unwrap();
    ConfigContext {
        invocation_dir: root.clone(),
        root,
        config: Default::default(),
        questions: Box::leak(questions.into_boxed_slice()),
    }
}

/// `jevgate rules propose` in `project`, answered by `evaluator`.
fn propose(project: &Project, args: &ProposeArgs, evaluator: &mut dyn Evaluator) -> Printed {
    let context = context(project);
    let check = super::settings(args, &context).unwrap();
    super::propose(args, &context, (&check, evaluator)).unwrap()
}

/// The table run of `propose`, answered by [`Rules`].
fn proposed(project: &Project) -> Printed {
    propose(
        project,
        &arguments(ProposeFormat::Table, false),
        &mut Rules::default(),
    )
}

fn json_run(project: &Project, evaluator: &mut dyn Evaluator) -> Value {
    let printed = propose(project, &arguments(ProposeFormat::Json, false), evaluator);
    serde_json::from_str(&printed.stdout).unwrap()
}

/// The path of the proposal `id`.
fn proposal_file(project: &Project, id: &str) -> PathBuf {
    custom::within(&project.0, PROPOSALS).join(format!("{id}.toml"))
}

fn proposals(project: &Project) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(custom::within(&project.0, PROPOSALS))
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

fn ids(ids: &[&str]) -> Vec<String> {
    ids.iter().map(|id| id.to_string()).collect()
}

#[test]
fn rules_become_note_questions_that_quote_their_line_and_load_as_question_files() {
    let project = Project::new();
    project.write("AGENTS.md", AGENTS);
    let mut rules = Rules::default();
    let printed = propose(
        &project,
        &arguments(ProposeFormat::Table, false),
        &mut rules,
    );
    assert_eq!(
        (printed.code, rules.calls),
        (0, 2),
        "the rules are asked what checks them"
    );
    assert_eq!(proposals(&project), IDS.map(|id| format!("{id}.toml")));
    assert!(
        printed
            .stdout
            .contains("Proposed 2 questions in .jevgate/proposals/:"),
        "{}",
        printed.stdout
    );
    let path = proposal_file(&project, IDS[0]);
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(
        text.starts_with("# Proposed by `jevgate rules propose` from AGENTS.md:7.\n"),
        "{text}"
    );
    let question = custom::read_file(&path, "p.toml".into()).unwrap();
    assert_eq!(
        question.question,
        "Does this function break the project rule \"Never log request bodies.\" (AGENTS.md:7)?"
    );
    assert_eq!(
        question.background.as_deref(),
        Some("The rule is from AGENTS.md, section \"Conventions\".")
    );
    assert_eq!(
        question.guidance.as_deref(),
        Some("In AGENTS.md, the rule comes under \"Handlers follow these rules:\".")
    );
    assert_eq!(question.unit, Kind::Function);
    assert_eq!(question.level, crate::schema::Strength::Note);
    assert!(question.paths.is_empty());
}

#[test]
fn a_rerun_asks_nothing_new_and_keeps_edited_and_accepted_proposals() {
    let project = Project::new();
    project.write("AGENTS.md", AGENTS);
    proposed(&project);
    let kept = proposal_file(&project, IDS[1]);
    let edited = std::fs::read_to_string(&kept)
        .unwrap()
        .replace("level = \"note\"", "level = \"review\"");
    std::fs::write(&kept, &edited).unwrap();
    accept(&ids(&IDS[..1]), &context(&project)).unwrap();
    project.write(
        "AGENTS.md",
        &AGENTS.replace("# Conventions", "# Conventions\n\nIntro."),
    );
    let mut rules = Rules::default();
    let printed = propose(
        &project,
        &arguments(ProposeFormat::Table, false),
        &mut rules,
    );
    assert_eq!(rules.calls, 1, "only the pack with the new line is asked");
    assert!(
        printed.stdout.contains("No new proposals."),
        "{}",
        printed.stdout
    );
    assert!(
        printed.stdout.contains(
            "Not written: 1 line already proposed and kept as it is; 1 line already a question."
        ),
        "{}",
        printed.stdout
    );
    assert_eq!(std::fs::read_to_string(&kept).unwrap(), edited);
    assert_eq!(proposals(&project), [format!("{}.toml", IDS[1])]);
    let mut again = Rules::default();
    propose(
        &project,
        &arguments(ProposeFormat::Table, false),
        &mut again,
    );
    assert_eq!(
        again.calls, 0,
        "unchanged lines are answered from the cache"
    );
}

#[test]
fn a_rule_a_tool_checks_is_not_proposed_and_one_a_test_run_shows_is() {
    let project = Project::new();
    project.write(
        "AGENTS.md",
        "# Style\n\n- Never write more than 100 characters per line.\n- Never log request bodies.\n- Never let the locales' keys differ.\n",
    );
    let printed = proposed(&project);
    assert_eq!(
        proposals(&project),
        [
            "never-let-the-locales-keys-differ.toml",
            "never-log-request-bodies.toml"
        ]
    );
    assert!(
        printed.stdout.contains(
            "A rule a formatter, linter, compiler or measuring script already checks at 0.80: 1 line."
        ),
        "{}",
        printed.stdout
    );
    let report = json_run(&project, &mut Rules::default());
    assert_eq!(report["thresholds"], json!({"rule": 0.8, "tool": 0.8}));
    let line = &report["candidates"][0];
    assert_eq!(line["checkers"]["tool"], 0.9);
    assert!(line["proposal"].is_null());
    assert_eq!(report["candidates"][1]["checkers"]["reviewer"], 0.9);
    assert_eq!(
        report["candidates"][2]["checkers"]["run"], 0.9,
        "a rule a test run shows is still a reviewer's to check"
    );
}

#[test]
fn a_copy_of_a_rule_in_another_file_is_proposed_once() {
    let project = Project::new();
    project.write("AGENTS.md", AGENTS);
    project.write("CLAUDE.md", AGENTS);
    let report = json_run(&project, &mut Rules::default());
    let statuses: Vec<(&str, &str)> = report["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| !c["proposal"].is_null())
        .map(|c| {
            let status = c["proposal"]["status"].as_str().unwrap();
            (c["path"].as_str().unwrap(), status)
        })
        .collect();
    assert_eq!(
        statuses,
        [
            ("AGENTS.md", "new"),
            ("AGENTS.md", "new"),
            ("CLAUDE.md", "repeated"),
            ("CLAUDE.md", "repeated"),
        ]
    );
    assert!(proposals(&project).is_empty(), "JSON writes nothing");
}

#[test]
fn json_shows_every_line_with_its_answers() {
    let project = Project::new();
    project.write("AGENTS.md", AGENTS);
    let report = json_run(&project, &mut Rules::default());
    assert_eq!(report["complete"], true);
    assert_eq!(
        report["files"],
        json!([{"path": "AGENTS.md", "lines": 5, "paths": []}])
    );
    let command = &report["candidates"][2];
    assert_eq!(command["text"], "Run `cargo test` before pushing.");
    assert_eq!(command["convention"], 0.05);
    assert_eq!(command["unit"], "function");
    assert!(command["proposal"].is_null());
    let rule = &report["candidates"][1]["proposal"];
    assert_eq!(rule["id"], IDS[0]);
    assert_eq!(rule["level"], "note");
    assert_eq!(rule["from"], "AGENTS.md:7");
}

#[test]
fn toml_prints_question_tables_for_the_configuration_and_writes_nothing() {
    let project = Project::new();
    project.write("AGENTS.md", AGENTS);
    let printed = propose(
        &project,
        &arguments(ProposeFormat::Toml, false),
        &mut Rules::default(),
    );
    let questions = custom::parse(&printed.stdout).unwrap();
    let parsed: Vec<&str> = questions.iter().map(|q| q.id()).collect();
    assert_eq!(parsed, IDS);
    assert!(proposals(&project).is_empty());
}

#[test]
fn a_dry_run_counts_requests_and_tokens_and_writes_nothing() {
    let project = Project::new();
    project.write("AGENTS.md", AGENTS);
    let mut rules = Rules::default();
    let printed = propose(&project, &arguments(ProposeFormat::Table, true), &mut rules);
    assert_eq!(rules.calls, 0);
    assert!(
        printed.stdout.starts_with(
            "JevGate: dry run · 1 file · 5 lines · 1 requests, 0 answered by the cache · ~"
        ),
        "{}",
        printed.stdout
    );
    assert!(!project.0.join(".jevgate").exists());
    let mut args = arguments(ProposeFormat::Json, true);
    args.show_requests = true;
    let plan = |rules: &mut Rules| -> Value {
        serde_json::from_str(&propose(&project, &args, rules).stdout).unwrap()
    };
    let cold = plan(&mut rules);
    assert_eq!(cold["planned_requests"], 1);
    assert!(
        cold["requests"][0].get("jevgate").is_none(),
        "local metadata is not shown"
    );
    proposed(&project);
    let warm = plan(&mut rules);
    assert_eq!(
        (&warm["planned_cached"], &warm["planned_tokens"]),
        (&json!(1), &json!(0))
    );
}

#[test]
fn a_refused_request_leaves_the_run_incomplete() {
    let project = Project::new();
    project.write("AGENTS.md", AGENTS);
    let printed = propose(
        &project,
        &arguments(ProposeFormat::Table, false),
        &mut Refused,
    );
    assert_eq!(printed.code, 2);
    assert!(
        printed
            .stdout
            .contains("5 lines went unanswered: HTTP 402: credits exhausted"),
        "{}",
        printed.stdout
    );
    assert!(!printed.stdout.contains("No new proposals."));
    let toml = propose(
        &project,
        &arguments(ProposeFormat::Toml, false),
        &mut Refused,
    );
    assert_eq!(toml.notes, ["HTTP 402: credits exhausted"]);
    assert!(proposals(&project).is_empty());
}

#[test]
fn named_files_are_read_whatever_their_name_and_directories_select_instruction_files() {
    let project = Project::new();
    project.write("AGENTS.md", AGENTS);
    project.write("web/CLAUDE.md", "- Never call fetch in a component.\n");
    project.write("CONTRIBUTING.md", "- Never merge your own pull request.\n");
    let context = context(&project);
    let read = |paths: &[&str]| -> Vec<(String, Vec<String>)> {
        let paths: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
        let (files, _) = super::files::read(&paths, &context, 65_536).unwrap();
        files
            .iter()
            .map(|f| (proposal::slashed(&f.path), f.scope.clone()))
            .collect()
    };
    let web = ("web/CLAUDE.md".to_string(), vec!["web/**".to_string()]);
    assert_eq!(
        read(&[]),
        [("AGENTS.md".to_string(), Vec::new()), web.clone()]
    );
    assert_eq!(read(&["web"]), [web]);
    assert_eq!(
        read(&["CONTRIBUTING.md"]),
        [("CONTRIBUTING.md".to_string(), Vec::new())]
    );
    assert!(super::files::read(&[PathBuf::from("/")], &context, 65_536).is_err());
}

#[test]
fn a_file_outside_the_upload_patterns_is_not_read() {
    let project = Project::new();
    project.write("AGENTS.md", AGENTS);
    let mut context = context(&project);
    context.config.upload_deny = vec!["AGENTS.md".into()];
    let (files, skipped) = super::files::read(&[], &context, 65_536).unwrap();
    assert!(files.is_empty());
    assert_eq!(
        skipped[0].reason,
        "Outside upload_allow/upload_deny; not read."
    );
}

#[test]
fn a_file_that_is_not_utf8_is_not_read_and_the_rest_are_asked() {
    let project = Project::new();
    project.write("AGENTS.md", AGENTS);
    std::fs::create_dir_all(project.0.join("web")).unwrap();
    std::fs::write(
        project.0.join("web/CLAUDE.md"),
        b"- Never log \xff bodies.\n",
    )
    .unwrap();
    let mut rules = Rules::default();
    let printed = propose(
        &project,
        &arguments(ProposeFormat::Table, false),
        &mut rules,
    );
    assert_eq!((printed.code, rules.calls), (0, 2));
    assert!(
        printed
            .stdout
            .contains("Not read: web/CLAUDE.md (Source is not UTF-8: "),
        "{}",
        printed.stdout
    );
    assert_eq!(proposals(&project), IDS.map(|id| format!("{id}.toml")));
}

#[test]
fn translations_are_read_only_when_named() {
    let project = Project::new();
    project.write("AGENTS.md", AGENTS);
    let locales = ["fr", "ja", "pt-br", "zh-hans"];
    for locale in locales {
        project.write(
            &format!("docs/i18n/{locale}/CLAUDE.md"),
            "- Nunca registre corpos.\n",
        );
    }
    let context = context(&project);
    let read = |paths: &[&str]| -> (Vec<String>, Vec<String>) {
        let paths: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
        let (files, skipped) = super::files::read(&paths, &context, 65_536).unwrap();
        let read = files.iter().map(|f| proposal::slashed(&f.path)).collect();
        let left = skipped.iter().map(|s| proposal::slashed(&s.path)).collect();
        (read, left)
    };
    let translation = |locale: &str| format!("docs/i18n/{locale}/CLAUDE.md");
    let all: Vec<String> = locales.map(translation).to_vec();
    assert_eq!(read(&[]), (vec!["AGENTS.md".into()], all.clone()));
    assert_eq!(read(&["docs"]), (Vec::new(), all));
    assert_eq!(
        read(&[".", "docs/i18n/ja", "docs/i18n/fr/CLAUDE.md"]),
        (
            vec!["AGENTS.md".into(), translation("fr"), translation("ja")],
            vec![translation("pt-br"), translation("zh-hans")]
        ),
        "a translation named, or under a directory named inside it, is read"
    );
    let printed = propose(
        &project,
        &arguments(ProposeFormat::Table, true),
        &mut Rules::default(),
    );
    assert!(
        printed.stdout.ends_with(
            "\nNot read: 4 files, such as docs/i18n/fr/CLAUDE.md (A translation under a locale directory; name it to read it.)"
        ),
        "{}",
        printed.stdout
    );
}

#[test]
fn a_repository_without_instruction_files_asks_nothing() {
    let project = Project::new();
    project.write("src/lib.rs", "fn main() {}\n");
    let mut rules = Rules::default();
    let printed = propose(
        &project,
        &arguments(ProposeFormat::Table, false),
        &mut rules,
    );
    assert_eq!((printed.code, rules.calls), (0, 0));
    assert!(
        printed.stdout.starts_with(
            "No agent instruction file to read; name one, such as `jevgate rules propose CONTRIBUTING.md`."
        ),
        "{}",
        printed.stdout
    );
    assert!(proposals(&project).is_empty());
}

/// A directory named with a line break cannot end a proposal's comment and
/// add a key to the question file.
#[cfg(unix)]
#[test]
fn a_path_cited_in_a_comment_holds_no_line_break() {
    let project = Project::new();
    project.write(
        "x\nlevel = \"review\"\n#/CLAUDE.md",
        "- Never log request bodies.\n",
    );
    proposed(&project);
    let path = proposal_file(&project, IDS[0]);
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(
        text.starts_with(
            "# Proposed by `jevgate rules propose` from xlevel = \"review\"#/CLAUDE.md:1.\n"
        ),
        "{text}"
    );
    let question = custom::read_file(&path, "p.toml".into()).unwrap();
    assert_eq!(question.level, crate::schema::Strength::Note);
}

#[test]
fn rules_apply_where_their_file_loads() {
    let project = Project::new();
    project.write("app/[locale]/CLAUDE.md", "- Never read cookies here.\n");
    project.write(
        ".cursor/rules/api.mdc",
        "---\nglobs: src/api/**, *.ts\n---\n- Never return raw errors.\n",
    );
    let (files, _) = super::files::read(&[], &context(&project), 65_536).unwrap();
    let scopes: Vec<&[String]> = files.iter().map(|f| f.scope.as_slice()).collect();
    assert_eq!(
        scopes,
        [&["src/api/**", "*.ts"][..], &["app/[[]locale]/**"]]
    );
    let glob = crate::boundary::globs(&files[1].scope).unwrap();
    assert!(glob.is_match("app/[locale]/page.tsx"));
    assert!(!glob.is_match("app/l/page.tsx"));
}

#[test]
fn a_long_rule_is_quoted_up_to_a_sentence_and_whole_in_its_guidance() {
    let sentences = ["Bodies hold personal data of customers and partners."; 5].join(" ");
    let long = format!("Never log request bodies. {sentences} Always log the request id.");
    let project = Project::new();
    project.write("AGENTS.md", &format!("# Logs\n\n- {long}\n"));
    let report = json_run(&project, &mut Rules::default());
    let proposal = &report["candidates"][0]["proposal"];
    let question = proposal["question"].as_str().unwrap();
    assert!(
        question.chars().count() <= custom::QUESTION_CHARS,
        "{question}"
    );
    assert!(
        question.ends_with("partners.…\" (AGENTS.md:3)?"),
        "{question}"
    );
    assert_eq!(proposal["guidance"], format!("The whole rule: \"{long}\""));
}

#[test]
fn ids_come_from_the_first_words_of_the_rule() {
    assert_eq!(
        proposal::slug("**Error Handling**: Always handle errors explicitly. Do not ignore `_`.")
            .as_deref(),
        Some("error-handling-always-handle-errors")
    );
    assert_eq!(
        proposal::slug("3 retries, then Não fail").as_deref(),
        Some("retries-then-no-fail")
    );
    assert_eq!(proposal::slug("不要记录请求正文"), None);
    assert_eq!(
        proposal::slug(
            "Prefer `undefined` for absent values. Do not add special handling for `null`."
        )
        .as_deref(),
        Some("prefer-undefined-for-absent-values"),
        "the first sentence names the rule"
    );
    assert_eq!(
        proposal::slug("Wrap errors (e.g. with context). Always.").as_deref(),
        Some("wrap-errors-e-g-with-context"),
        "an abbreviation ends no sentence"
    );
    assert_eq!(
        proposal::slug("Imports. Use relative imports.").as_deref(),
        Some("imports-use-relative-imports"),
        "a first sentence of one word is too short to name a rule"
    );
}

/// [`Rules`], with the first line of every request a rule.
struct First(Rules);

impl Evaluator for First {
    fn evaluate(&mut self, request: &Value) -> Result<Value> {
        let mut body = self.0.evaluate(request)?;
        if let Some(first) = body["answers"].get_mut("c0_convention") {
            *first = json!({"type": "noul", "noul": 0.9});
        }
        Ok(body)
    }
}

#[test]
fn a_rule_without_ascii_words_takes_its_file_and_line_as_id() {
    let project = Project::new();
    project.write("AGENTS.md", "# 规则\n\n- 不要记录请求正文\n");
    let args = arguments(ProposeFormat::Table, false);
    propose(&project, &args, &mut First(Rules::default()));
    assert_eq!(proposals(&project), ["agents-3.toml"]);
}

#[test]
fn accept_moves_a_valid_proposal_and_refuses_the_rest_before_moving_any() {
    let project = Project::new();
    project.write("AGENTS.md", AGENTS);
    proposed(&project);
    let broken = "question = \"Not a question\"\nunit = \"function\"\n";
    std::fs::write(proposal_file(&project, IDS[1]), broken).unwrap();
    let before = context(&project);
    let error = accept(&ids(&IDS), &before).unwrap_err().to_string();
    assert!(error.contains("must be one yes/no question"), "{error}");
    assert!(proposal_file(&project, IDS[0]).exists(), "nothing moved");
    for (id, expected) in [
        ("../AGENTS", "Invalid proposal"),
        ("missing", "No proposal missing"),
    ] {
        let error = accept(&ids(&[id]), &before).unwrap_err().to_string();
        assert!(error.contains(expected), "{error}");
    }
    accept(&ids(&IDS[..1]), &before).unwrap();
    let moved = custom::directory(&project.0).join(format!("{}.toml", IDS[0]));
    assert!(moved.exists() && !proposal_file(&project, IDS[0]).exists());
    let reloaded = context(&project);
    assert_eq!(reloaded.questions[0].rule, format!("custom/{}", IDS[0]));
    std::fs::copy(&moved, proposal_file(&project, IDS[0])).unwrap();
    let error = accept(&ids(&IDS[..1]), &reloaded).unwrap_err().to_string();
    assert!(
        error.contains("is already defined in .jevgate/questions/never-log-request-bodies.toml"),
        "{error}"
    );
}

#[test]
fn the_accept_message_says_what_the_question_needs() {
    let project = Project::new();
    project.write(
        ".jevgate/questions/q.toml",
        "question = \"Does this change add a dependency?\"\nunit = \"hunk\"\nlevel = \"note\"\n",
    );
    let text = super::accept::accepted(&context(&project).questions[0]);
    assert_eq!(
        text,
        "Accepted custom/q into .jevgate/questions/q.toml; commit it.\n  It is a note, which never fails the gate: set level = \"review\" to enforce it.\n  It is asked only with --base."
    );
}

#[test]
fn every_proposal_is_a_valid_question_file_whatever_its_line() {
    let project = Project::new();
    let rules = [
        "Never use `unwrap()` in \"handlers\"; it's a panic.",
        "Never write \\ or ''' in a TOML string",
        "Never add a dependency: ask first.\n  - even a small one",
    ];
    let source: String = rules.iter().map(|l| format!("- {l}\n")).collect();
    project.write("docs/deep/nested/path/that/is/long/AGENTS.md", &source);
    proposed(&project);
    let written = proposals(&project);
    assert_eq!(written.len(), 3, "{written:?}");
    for name in written {
        let path = custom::within(&project.0, PROPOSALS).join(&name);
        let question = custom::read_file(&path, PathBuf::from(&name)).unwrap();
        assert_eq!(question.paths, ["docs/deep/nested/path/that/is/long/**"]);
    }
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

/// Answers a custom question about a function yes when the function logs a
/// request's body, and every built-in question clear.
struct Logs;

impl Evaluator for Logs {
    fn evaluate(&mut self, request: &Value) -> Result<Value> {
        let mut body = answer(request, 0);
        for (key, slot) in body["answers"].as_object_mut().unwrap() {
            let Some(rest) = key.strip_prefix("custom_") else {
                continue;
            };
            let index: usize = rest.split('_').next().unwrap().parse().unwrap();
            let source = request["state"]["functions"][index]["source"]
                .as_str()
                .unwrap_or_default();
            let logs = source.contains("log(") && source.contains(".body");
            *slot = json!({"type": "noul", "noul": if logs { 0.93 } else { 0.04 }});
        }
        Ok(body)
    }
}

/// The CI side of the version's done-when: a question proposed from an
/// AGENTS.md, raised to review and accepted by a person, fails `check
/// --base` on a change that breaks its rule and passes once it is fixed.
#[test]
fn an_accepted_proposal_fails_the_gate_on_a_change_that_breaks_its_rule() {
    let project = Project::new();
    project.write("AGENTS.md", AGENTS);
    project.write("src/lib.rs", &crate::tests::function("total"));
    git(&project, &["init", "-q"]);
    git(&project, &["add", "."]);
    git(&project, &["commit", "-qm", "base"]);
    proposed(&project);
    let proposal = proposal_file(&project, IDS[0]);
    let reviewed = std::fs::read_to_string(&proposal)
        .unwrap()
        .replace("level = \"note\"", "level = \"review\"");
    std::fs::write(&proposal, reviewed).unwrap();
    accept(&ids(&IDS[..1]), &context(&project)).unwrap();

    let check = |source: &str| {
        project.write("src/lib.rs", source);
        let mut options: CheckArgs = crate::tests::args();
        options.rules = Vec::new();
        context(&project).configure(&mut options).unwrap();
        options.base = Some(crate::revision::resolve(&project.0, "HEAD").unwrap());
        let report = crate::tests::run(&project, &options, &mut Logs);
        let rules: Vec<String> = report
            .files
            .iter()
            .flat_map(|f| &f.findings)
            .map(|f| f.rule.clone())
            .collect();
        (crate::gate::exit_code(&report), rules)
    };
    let breaking = "fn charge(request: &Request) -> u32 {\n    log(&request.body);\n    let total = request.total();\n    let tax = total / 10;\n    let fee = 1;\n    total + tax + fee\n}\n";
    assert_eq!(check(breaking), (1, vec![format!("custom/{}", IDS[0])]));
    let fixed = breaking.replace("log(&request.body);", "log(&request.id);");
    assert_eq!(check(&fixed), (0, Vec::new()));
}
