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
    assert_eq!(question.default_levels(), [FailOn::Review]);
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
    let levels = |level: &str| {
        configured(&format!("{QUESTION}level = \"{level}\"\n")).unwrap()[0].default_levels()
    };
    assert_eq!(levels("consider"), [FailOn::Consider]);
    assert_eq!(levels("note"), [FailOn::None]);
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
}
