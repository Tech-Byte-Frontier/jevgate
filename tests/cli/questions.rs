//! Custom questions: their files, listing, validation and planning.
use super::*;

const BODY_LOGS: &str = "question = \"Does this function write a request body to a log?\"\nunit = \"function\"\nlevel = \"consider\"\n";

/// A project whose `.jevgate/questions/` holds `files`, each an id and its text.
fn with_questions(files: &[(&str, &str)]) -> Project {
    let project = Project::new();
    std::fs::create_dir_all(project.0.join(".jevgate/questions")).unwrap();
    for (id, text) in files {
        std::fs::write(
            project.0.join(format!(".jevgate/questions/{id}.toml")),
            text,
        )
        .unwrap();
    }
    std::fs::write(project.0.join("lib.rs"), JUDGED_RS).unwrap();
    project
}

#[test]
fn rules_lists_the_questions_of_the_questions_directory() {
    let project = with_questions(&[("body-logs", BODY_LOGS)]);
    let output = project.command().arg("rules").output().unwrap();
    let table = String::from_utf8(output.stdout).unwrap();
    assert!(
        table.contains("custom/body-logs") && table.contains("[function, consider at 0.80]"),
        "{table}"
    );
    assert!(table.contains("documentation, custom, default"), "{table}");
    let output = project
        .command()
        .args(["rules", "--format", "json"])
        .output()
        .unwrap();
    let rules: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let custom = rules.as_array().unwrap().last().unwrap();
    assert_eq!(custom["id"], "custom/body-logs");
    assert_eq!(custom["group"], "custom");
    assert_eq!(custom["custom"]["level"], "consider");
    assert_eq!(
        custom["custom"]["source"],
        ".jevgate/questions/body-logs.toml"
    );
}

#[test]
fn an_invalid_question_stops_every_command_that_reads_the_configuration() {
    let project = with_questions(&[("Body Logs", BODY_LOGS)]);
    for args in [&["rules"][..], &["check", "--dry-run"]] {
        let output = project.command().args(args).output().unwrap();
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(
            error
                .contains("Invalid question id \"Body Logs\" in .jevgate/questions/Body Logs.toml"),
            "{error}"
        );
    }
}
