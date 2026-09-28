//! `rules propose` and `rules accept`: offline plans, a missing key, and
//! accepting a proposal.
use super::*;

const AGENTS: &str =
    "# Conventions\n\n- Never log request bodies.\n- Run `make test` before pushing.\n";

fn with_agents() -> Project {
    let project = Project::new();
    std::fs::write(project.0.join("AGENTS.md"), AGENTS).unwrap();
    project
}

#[test]
fn a_dry_run_plans_the_lines_of_the_instruction_files_offline() {
    let project = with_agents();
    let output = project
        .command()
        .args(["rules", "propose", "--dry-run"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(
        text.starts_with(
            "JevGate: dry run · 1 file · 2 lines · 1 requests, 0 answered by the cache"
        ),
        "{text}"
    );
    let output = project
        .command()
        .args(["rules", "propose", "--dry-run", "--show-requests"])
        .output()
        .unwrap();
    let plan: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        plan["requests"][0]["state"]["candidates"][0]["text"],
        "Never log request bodies."
    );
    assert!(
        !project.0.join(".jevgate").exists(),
        "a dry run writes nothing"
    );
    let output = project
        .command()
        .args([
            "rules",
            "propose",
            "--dry-run",
            "--show-requests",
            "--format",
            "table",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn without_a_key_nothing_is_proposed_and_the_run_is_incomplete() {
    let project = with_agents();
    let output = project
        .command()
        .args(["rules", "propose"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(
        text.contains("2 lines went unanswered: No API key configured."),
        "{text}"
    );
    assert!(!project.0.join(".jevgate/proposals").exists());
}

#[test]
fn an_accepted_proposal_is_listed_as_a_rule() {
    let project = with_agents();
    std::fs::create_dir_all(project.0.join(".jevgate/proposals")).unwrap();
    std::fs::write(
        project.0.join(".jevgate/proposals/no-body-logs.toml"),
        "# jevgate-proposal: 0123456789ab\nquestion = \"Does this function log a request body?\"\nunit = \"function\"\nlevel = \"note\"\n",
    )
    .unwrap();
    let output = project
        .command()
        .args(["rules", "accept", "no-body-logs"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.starts_with("Accepted custom/no-body-logs"), "{text}");
    assert!(
        project
            .0
            .join(".jevgate/questions/no-body-logs.toml")
            .exists()
    );
    let output = project.command().arg("rules").output().unwrap();
    let table = String::from_utf8(output.stdout).unwrap();
    assert!(table.contains("custom/no-body-logs"), "{table}");
    let output = project
        .command()
        .args(["rules", "accept", "no-body-logs"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2), "already accepted");
}
