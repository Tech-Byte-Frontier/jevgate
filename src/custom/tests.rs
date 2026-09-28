use super::*;
use crate::tests::Project;

const QUESTION: &str = r#"
[[question]]
id = "no-body-logs"
question = "Does this function write a request body to a log?"
unit = "function"
"#;

fn configured(text: &str) -> Result<Vec<Question>> {
    let config: crate::config::Config = toml::from_str(text)?;
    load(
        Path::new("."),
        (Path::new("jevgate.toml"), &config.question),
        None,
    )
}

#[test]
fn a_question_names_its_rule_and_takes_the_defaults() {
    let questions = configured(QUESTION).unwrap();
    let question = &questions[0];
    assert_eq!(question.rule, "custom/no-body-logs");
    assert_eq!(question.id(), "no-body-logs");
    assert_eq!(question.threshold, 0.8);
    assert_eq!(
        (question.level, question.unit),
        (Strength::Review, Kind::Function)
    );
    assert_eq!(question.blocks(), [Strength::Review]);
    assert!(question.applies_to(Path::new("deep/any.rs")));
    assert!(!question.names_files());
    assert!(
        question
            .next_step
            .contains("`jevgate: allow(custom/no-body-logs) reason`")
    );
    assert_eq!(question.source, Path::new("jevgate.toml"));
    assert_eq!(question.version.len(), VERSION_CHARS);
    assert_eq!(
        question.summary(),
        "function, review at 0.80",
        "what the rules table shows"
    );
}

#[test]
fn every_field_counts_toward_the_version_and_levels_follow_the_level() {
    let changed = |extra: &str| {
        configured(&format!("{QUESTION}{extra}\n")).unwrap()[0]
            .version
            .clone()
    };
    let plain = changed("");
    for extra in [
        "threshold = 0.9",
        "level = \"note\"",
        "background = \"Bodies hold personal data.\"",
        "guidance = \"Ids are fine.\"",
    ] {
        assert_ne!(changed(extra), plain, "{extra}");
    }
    assert_eq!(
        changed("paths = [\"src/**\"]"),
        plain,
        "paths only choose files"
    );
    assert_eq!(changed("next_step = \"Log the id.\""), plain);
    let blocks =
        |level: &str| configured(&format!("{QUESTION}level = \"{level}\"\n")).unwrap()[0].blocks();
    assert_eq!(blocks("consider"), [Strength::Consider]);
    assert!(blocks("note").is_empty(), "a note never fails the gate");
}

#[test]
fn paths_choose_files_and_let_file_and_hunk_questions_name_other_text() {
    let text = QUESTION.replace("\"function\"", "\"hunk\"") + "paths = [\"infra/**/*.tf\"]\n";
    let question = &configured(&text).unwrap()[0];
    assert!(question.applies_to(Path::new("infra/net/main.tf")));
    assert!(!question.applies_to(Path::new("src/main.rs")));
    assert!(question.names_files());
    let function = QUESTION.to_string() + "paths = [\"src/**\"]\n";
    assert!(!configured(&function).unwrap()[0].names_files());
}

#[test]
fn an_invalid_field_is_an_error_naming_the_question_and_its_file() {
    let long = "x".repeat(QUESTION_CHARS);
    for (extra, problem) in [
        ("threshold = 0.3", "threshold 0.3 is outside 0.5 to 0.99"),
        ("threshold = 1.0", "threshold 1 is outside"),
        ("paths = [\"src/[a\"]", "invalid paths"),
        (
            &format!("background = \"{}\"", "y".repeat(TEXT_CHARS + 1)) as &str,
            "`background` is 2001 characters",
        ),
        (
            &format!("next_step = \"{}\"", "z".repeat(NEXT_STEP_CHARS + 1)),
            "`next_step` is 301 characters",
        ),
    ] {
        let error = configured(&format!("{QUESTION}{extra}\n"))
            .unwrap_err()
            .to_string();
        assert!(
            error.starts_with("Question custom/no-body-logs in jevgate.toml: ")
                && error.contains(problem),
            "{error}"
        );
    }
    let unasked = QUESTION.replace("to a log?", "to a log.");
    let error = configured(&unasked).unwrap_err().to_string();
    assert!(error.contains("ending in `?`"), "{error}");
    let wordy = QUESTION.replace("Does this function", &format!("Does {long} this function"));
    let error = configured(&wordy).unwrap_err().to_string();
    assert!(
        error.contains("move detail to background or guidance"),
        "{error}"
    );
}

#[test]
fn a_question_holds_no_character_a_terminal_acts_on_or_hides() {
    let mirrored = QUESTION.replace("write a request", "write\\u202E a request");
    for (text, problem) in [
        (mirrored, "`question` holds U+202E"),
        (
            format!("{QUESTION}guidance = \"Fine\\u0000.\"\n"),
            "`guidance` holds U+0000",
        ),
        (
            format!("{QUESTION}next_step = \"Fix\\nit.\"\n"),
            "`next_step` holds U+000A",
        ),
        (
            format!("{QUESTION}background = \"One\\u2028two.\"\n"),
            "`background` holds U+2028",
        ),
    ] {
        let error = configured(&text).unwrap_err().to_string();
        assert!(
            error.starts_with("Question custom/no-body-logs in jevgate.toml: ")
                && error.ends_with(&format!(
                    "{problem}, which a terminal acts on or hides; remove it"
                )),
            "{error}"
        );
    }
    let lines = format!("{QUESTION}guidance = \"Counts:\\n\\t- a body.\\r\\nFine: an id.\"\n");
    assert!(configured(&lines).is_ok(), "guidance may hold lines");
}

#[test]
fn ids_are_required_unique_and_safe_to_name_a_rule() {
    let error = configured(&QUESTION.replace("id = \"no-body-logs\"\n", ""))
        .unwrap_err()
        .to_string();
    assert_eq!(error, "Question 1 in jevgate.toml needs an id");
    for id in [
        "No-Logs",
        "no_logs",
        "no--logs",
        "-logs",
        "logs-",
        "9logs",
        &"a".repeat(49),
    ] {
        let error = configured(&QUESTION.replace("no-body-logs", id))
            .unwrap_err()
            .to_string();
        assert!(error.starts_with("Invalid question id"), "{id}: {error}");
    }
    let twice = format!("{QUESTION}{QUESTION}");
    let error = configured(&twice).unwrap_err().to_string();
    assert!(
        error.contains("defined twice: in jevgate.toml and in jevgate.toml"),
        "{error}"
    );
    for (text, problem) in [
        (
            QUESTION.replace("\"function\"", "\"method\""),
            "unknown variant `method`",
        ),
        (
            QUESTION.to_string() + "levle = \"note\"\n",
            "unknown field `levle`",
        ),
    ] {
        let error = format!("{:#}", configured(&text).unwrap_err());
        assert!(error.contains(problem), "{error}");
    }
}

#[test]
fn question_files_are_read_from_a_git_tree_as_it_holds_them() {
    let project = Project::new();
    let body = "question = \"Does this file mix two features?\"\nunit = \"file\"\n";
    project.write(".jevgate/questions/one-feature.toml", body);
    project.write(".jevgate/questions/.draft.toml", "not = valid");
    project.git(&["init", "-q"]);
    project.git(&["add", "."]);
    project.git(&["commit", "-qm", "questions"]);
    let at_head = || load_at(&project.0, "HEAD", (Path::new("jevgate.toml"), &[]));
    project.write(
        ".jevgate/questions/one-feature.toml",
        &format!("{body}level = \"note\"\n"),
    );
    project.write(".jevgate/questions/later.toml", body);
    let questions = at_head().unwrap();
    assert_eq!(
        questions.len(),
        1,
        "the tree's files, not the working tree's"
    );
    assert_eq!(questions[0].rule, "custom/one-feature");
    assert_eq!(questions[0].level, Strength::Review, "as committed");
    assert_eq!(
        questions[0].source,
        Path::new(".jevgate/questions/one-feature.toml")
    );
    #[cfg(unix)]
    {
        project.write("secret.txt", "TYPESAFE_API_KEY=do-not-expose\n");
        std::os::unix::fs::symlink(
            project.0.join("secret.txt"),
            super::directory(&project.0).join("leak.toml"),
        )
        .unwrap();
        project.git(&["add", "."]);
        project.git(&["commit", "-qm", "link"]);
        let error = format!("{:#}", at_head().unwrap_err());
        assert!(
            error.contains(
                "Cannot read .jevgate/questions/leak.toml: a link is not a question file"
            ),
            "{error}"
        );
        assert!(!error.contains("do-not-expose"), "{error}");
    }
}

#[test]
fn question_files_are_named_by_their_id_beside_the_configuration() {
    let project = Project::new();
    let body = "question = \"Does this file mix two features?\"\nunit = \"file\"\n";
    project.write(".jevgate/questions/one-feature.toml", body);
    project.write(".jevgate/questions/notes.md", "not a question");
    project.write(".jevgate/questions/.draft.toml", "not = valid");
    let directory = super::directory(&project.0);
    let load_all = |specs: &[Spec]| {
        load(
            &project.0,
            (&project.0.join("jevgate.toml"), specs),
            Some(&directory),
        )
    };
    let questions = load_all(&[]).unwrap();
    assert_eq!(questions.len(), 1, "hidden and other files are left out");
    assert_eq!(questions[0].rule, "custom/one-feature");
    assert_eq!(
        questions[0].source,
        Path::new(".jevgate/questions/one-feature.toml")
    );
    let config: crate::config::Config =
        toml::from_str(&QUESTION.replace("no-body-logs", "one-feature")).unwrap();
    let error = load_all(&config.question).unwrap_err().to_string();
    assert_eq!(
        error,
        "Question custom/one-feature is defined twice: in jevgate.toml and in .jevgate/questions/one-feature.toml"
    );
    project.write(
        ".jevgate/questions/one-feature.toml",
        &format!("id = \"other\"\n{body}"),
    );
    let error = load_all(&[]).unwrap_err().to_string();
    assert!(
        error.contains("does not match its file name; name the file other.toml"),
        "{error}"
    );
    let absent = load(
        &project.0,
        (&project.0.join("jevgate.toml"), &[]),
        Some(&project.0.join("missing")),
    );
    assert!(absent.unwrap().is_empty());
    #[cfg(unix)]
    {
        project.write("secret.txt", "TYPESAFE_API_KEY=do-not-expose\n");
        std::fs::remove_file(directory.join("one-feature.toml")).unwrap();
        std::os::unix::fs::symlink(project.0.join("secret.txt"), directory.join("leak.toml"))
            .unwrap();
        let error = format!("{:#}", load_all(&[]).unwrap_err());
        assert!(
            error.contains("Cannot read .jevgate/questions/leak.toml"),
            "{error}"
        );
        assert!(
            !error.contains("do-not-expose"),
            "a linked file is never parsed: {error}"
        );
    }
}

#[test]
fn a_question_file_git_ignores_is_named_with_the_rule_that_ignores_it() {
    let project = Project::new();
    let git = |args: &[&str]| {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(&project.0)
            .args(args)
            .output()
            .unwrap()
            .status;
        assert!(status.success(), "git {args:?}");
    };
    git(&["init", "-q"]);
    project.write(".jevgate/.gitignore", super::super::storage::IGNORE);
    let file = Path::new(".jevgate/questions/one-feature.toml");
    project.write(
        &file.to_string_lossy(),
        "question = \"Is it?\"\nunit = \"file\"\n",
    );
    assert_eq!(
        ignored(&project.0, file),
        None,
        "JevGate's own .gitignore keeps it"
    );
    project.write(".gitignore", "target/\n/.jevgate/\n");
    let warning = ignored(&project.0, file).unwrap();
    assert!(
        warning.contains("Git ignores .jevgate/questions (.gitignore:2:/.jevgate/)"),
        "{warning}"
    );
    project.write(".gitignore", "/.jevgate/*\n!/.jevgate/questions/\n");
    assert_eq!(ignored(&project.0, file), None);
    // The `.gitignore` JevGate wrote in `.jevgate/` before 0.29 hides them
    // whatever the root one says, until a check rewrites it.
    project.write(".jevgate/.gitignore", "*\n");
    let warning = ignored(&project.0, file).unwrap();
    assert!(
        warning.contains("(.jevgate/.gitignore:1:*)")
            && warning.contains("an earlier JevGate wrote .jevgate/.gitignore, and the next check"),
        "{warning}"
    );
    project.write(".jevgate/.gitignore", "*\n# kept by hand\n");
    let warning = ignored(&project.0, file).unwrap();
    assert!(
        warning.ends_with("add `!questions/` and `!questions/**` to .jevgate/.gitignore"),
        "{warning}"
    );
}

const EXAMPLES: &str = r#"
[[question.failing]]
path = "src/api/orders.ts"
code = "export function charge(req) { log(req.body); }"

[[question.passing]]
file = ".jevgate/questions/examples/audit.ts"
path = "src/api/audit.ts"

[[question.passing]]
file = "tests/fixtures/ok.ts"
"#;

#[test]
fn examples_come_failing_then_passing_and_leave_the_version_alone() {
    let plain = configured(QUESTION).unwrap();
    let questions = configured(&format!("{QUESTION}{EXAMPLES}")).unwrap();
    let examples = &questions[0].examples;
    let shown: Vec<(Expected, usize, &Path)> = examples
        .iter()
        .map(|e| (e.expected, e.number, e.path.as_path()))
        .collect();
    assert_eq!(
        shown,
        [
            (Expected::Failing, 1, Path::new("src/api/orders.ts")),
            (Expected::Passing, 1, Path::new("src/api/audit.ts")),
            (Expected::Passing, 2, Path::new("tests/fixtures/ok.ts")),
        ],
        "a file example stands for its own path unless `path` names another"
    );
    assert!(matches!(&examples[0].text, Text::Inline(code) if code.contains("req.body")));
    assert!(matches!(&examples[1].text, Text::File(file) if file.ends_with("audit.ts")));
    assert_eq!(
        questions[0].version, plain[0].version,
        "examples change nothing a check asks"
    );
    assert_eq!(
        questions[0].describe()["examples"],
        serde_json::json!({"failing": 1, "passing": 2})
    );
    let project = Project::new();
    project.write(
        ".jevgate/questions/no-body-logs.toml",
        "question = \"Is it?\"\nunit = \"file\"\n[[failing]]\npath = \"a.sh\"\ncode = \"rm -rf /\"\n",
    );
    let directory = super::directory(&project.0);
    let filed = load(
        &project.0,
        (&project.0.join("jevgate.toml"), &[]),
        Some(&directory),
    )
    .unwrap();
    assert_eq!(filed[0].examples.len(), 1, "a question file's [[failing]]");
}

#[test]
fn an_invalid_example_is_an_error_naming_it_and_its_question() {
    let paths = "paths = [\"src/**\"]\n";
    for (example, problem) in [
        ("", "failing example 1: give either `code` or `file`"),
        (
            "code = \"x\"\nfile = \"src/x.ts\"\n",
            "give either `code` or `file`",
        ),
        ("code = \"  \"\npath = \"src/x.ts\"\n", "`code` is empty"),
        ("code = \"x\"\n", "`path` is required with `code`"),
        (
            "code = \"x\"\npath = \"/etc/x.ts\"\n",
            "`path` \"/etc/x.ts\" must be a path relative",
        ),
        (
            "code = \"x\"\npath = \"src/../x.ts\"\n",
            "`path` \"src/../x.ts\" must be a path relative",
        ),
        (
            "code = \"x\"\npath = \"src\\\\x.ts\"\n",
            "with `/` between its parts",
        ),
        (
            "file = \"src/.env\"\n",
            "src/.env is hidden, in a dependency or build directory, or a credential",
        ),
        (
            "file = \".git/config\"\npath = \"src/config\"\n",
            ".git/config is hidden",
        ),
        ("file = \"src/server.pem\"\n", "or a credential"),
        (
            "file = \".jevgate/questions/x.ts\"\n",
            "its path .jevgate/questions/x.ts is outside the question's paths; set `path`",
        ),
        (
            "code = \"x\"\npath = \"lib/x.ts\"\n",
            "its path lib/x.ts is outside the question's paths",
        ),
    ] {
        let text = format!("{QUESTION}{paths}[[question.failing]]\n{example}");
        let error = configured(&text).unwrap_err().to_string();
        assert!(
            error.starts_with("Question custom/no-body-logs in jevgate.toml: failing example 1: ")
                && error.contains(problem),
            "{example}: {error}"
        );
    }
    let unknown =
        format!("{QUESTION}[[question.failing]]\ncode = \"x\"\npath = \"x.ts\"\nnote = \"y\"\n");
    let error = format!("{:#}", configured(&unknown).unwrap_err());
    assert!(error.contains("unknown field `note`"), "{error}");
    let kept = format!(
        "{QUESTION}{paths}[[question.passing]]\nfile = \".jevgate/questions/examples/a.ts\"\npath = \"src/a.ts\"\n"
    );
    assert!(
        configured(&kept).is_ok(),
        "the question directory holds example files"
    );
}
