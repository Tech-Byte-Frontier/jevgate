//! Agent instruction files and project documentation.
use super::*;

#[test]
fn instruction_sections_are_at_most_consider_and_name_their_harnesses() {
    let project = Project::new();
    project.write("Cargo.toml", "[package]\nname = \"demo\"\n");
    project.write("src/lib.rs", "");
    project.write("web/app.ts", "");
    project.write(
        "AGENTS.md",
        "# Stack\nThis is a Rust project.\n\n# Web\nUse the design tokens in `web/theme.ts`.\n\n# Release\nTag with `v` then push.\n",
    );
    let mut options = args();
    only(&mut options, catalog::AGENT_CONTEXT);
    let mut eval = scripted(0);
    let top = json!({"type":"score","score":2.0,"confidence":1.0,
        "probabilities":{"0":0.0,"1":0.0,"2":1.0}});
    let web = json!({"type":"choice","choice":"web/","confidence":1.0,
        "probabilities":{"src/":0.0,"web/":1.0,"none":0.0}});
    eval.overrides = vec![("s0_inferable", top), ("s1_scope", web)];
    let report = run(&project, &options, &mut eval);
    let file = report
        .files
        .iter()
        .find(|f| f.path == std::path::Path::new("AGENTS.md"))
        .unwrap();
    let dimension = &file.dimensions[catalog::AGENT_CONTEXT];
    assert_eq!(
        (
            dimension.units.judged,
            dimension.units.consider,
            dimension.units.note
        ),
        (3, 1, 1)
    );
    let stack = &file.findings[0];
    assert_eq!(stack.strength, Strength::Consider, "capped below review");
    assert_eq!(
        (stack.rule.as_str(), stack.line),
        ("documentation/agent-context", 1)
    );
    assert!(
        stack.message.starts_with("Section `Stack` restates what the repository's files show (1.00). Codex, GitHub Copilot, Cursor, Windsurf, Cline and Claude Code load it at the start of every session"),
        "{}",
        stack.message
    );
    assert_eq!(file.findings[1].symbol.as_deref(), Some("Web"));
    assert!(
        file.findings[1]
            .message
            .contains("applies only to work in `web/`"),
        "{}",
        file.findings[1].message
    );
    assert!(file.findings[1].action.starts_with("Optional: move it"));
    let load = report.context_load.as_ref().unwrap();
    assert!(load.harnesses.iter().any(|h| h.harness == "Codex"));
}

fn git(project: &Project, args: &[&str]) {
    let status = std::process::Command::new("git")
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .current_dir(&*project.0)
        .output()
        .unwrap();
    assert!(status.status.success(), "{status:?}");
}

#[test]
fn stale_sections_and_repeated_sections_are_checked_after_the_first_pass() {
    let project = Project::new();
    let shared = "Install the dependencies, start the local database, copy the example environment file and run the development server before opening a pull request";
    project.write(
        "README.md",
        &format!("# Setup\n{shared}.\nRun `scripts/setup.sh` first.\n"),
    );
    project.write("docs/guide.md", &format!("# Getting started\n{shared}.\n"));
    let mut options = args();
    options.rules = vec![
        catalog::DOC_STALENESS.into(),
        catalog::DOC_DUPLICATION.into(),
    ];
    let mut eval = scripted(2);
    let report = run(&project, &options, &mut eval);
    let readme = report
        .files
        .iter()
        .find(|f| f.path == std::path::Path::new("README.md"))
        .unwrap();
    let messages: Vec<&str> = readme.findings.iter().map(|f| f.message.as_str()).collect();
    assert!(
        messages.iter().any(|m| m.contains(
            "tells the reader to use `scripts/setup.sh`, which is not in the repository"
        )),
        "{messages:?}"
    );
    assert!(
        readme
            .findings
            .iter()
            .any(|f| f.rule == "documentation/duplication" && f.locations.len() == 2),
        "{messages:?}"
    );
    assert!(
        readme
            .findings
            .iter()
            .all(|f| f.strength == Strength::Consider)
    );
}

#[test]
fn a_finished_plan_covers_its_section_checks() {
    let project = Project::new();
    project.write("src/old.ts", "export {}\n");
    git(&project, &["init", "-q"]);
    git(&project, &["add", "."]);
    git(&project, &["commit", "-q", "-m", "one"]);
    git(&project, &["tag", "v0.2.0"]);
    std::fs::remove_file(project.0.join("src/old.ts")).unwrap();
    project.write("src/new.ts", "export {}\n");
    project.write(
        "docs/plans/v0.2.0-plan.md",
        "# v0.2.0 plan\n## Task 1\nEdit `src/old.ts` to add the handler.\n",
    );
    git(&project, &["add", "-A"]);
    git(&project, &["commit", "-q", "-m", "two"]);
    let mut options = args();
    options.rules = vec![catalog::DOC_STALENESS.into()];
    let mut eval = scripted(2);
    let report = run(&project, &options, &mut eval);
    let plan = report
        .files
        .iter()
        .find(|f| f.path.ends_with("v0.2.0-plan.md"))
        .unwrap();
    assert_eq!(plan.findings.len(), 1, "{:?}", plan.findings);
    let message = &plan.findings[0].message;
    assert!(
        message.contains("a plan whose work is finished"),
        "{message}"
    );
    assert!(
        message.contains("release tag v0.2.0") && message.contains("`src/old.ts`"),
        "{message}"
    );
    let dimension = &plan.dimensions[catalog::DOC_STALENESS];
    assert_eq!(dimension.units.covered, 1, "{}", dimension.decision_basis);
}

fn plan_file(path: &str) -> crate::schema::FileResult {
    let mut file = hardcoded_file(path, "consider", &[]);
    let dimension = file.dimensions.remove("hardcoded_values").unwrap();
    file.dimensions.insert("doc_staleness".into(), dimension);
    let finding = &mut file.findings[0];
    finding.rule = "documentation/staleness".into();
    finding.category = Some(super::grouping::FINISHED_PLAN.into());
    finding.message = format!("`{path}` is a plan whose work is finished: tag v1 (0.95).");
    file
}

#[test]
fn finished_plans_in_one_directory_are_one_finding_named_by_the_directory() {
    use crate::schema::{Status, Strength};
    let mut files = vec![
        plan_file("docs/plans/b.md"),
        plan_file("docs/plans/a.md"),
        plan_file("docs/plans/c.md"),
        plan_file("docs/other/d.md"),
    ];
    let primary = grouped_primary(&mut files);
    assert_eq!(primary.strength, Strength::Consider);
    assert!(
        primary.message.ends_with(
            "The other 2 plans in `docs/plans` are finished too: `docs/plans/b.md`, `docs/plans/c.md`."
        ),
        "{}",
        primary.message
    );
    assert_eq!(primary.locations.len(), 3);
    for member in [&files[0], &files[2]] {
        assert_eq!(member.findings[0].strength, Strength::Note);
        assert_eq!(member.dimensions["doc_staleness"].units.note, 1);
        assert_eq!(member.status, Status::Note);
    }
    assert_eq!(
        files[3].findings[0].strength,
        Strength::Consider,
        "a plan alone in its directory"
    );
    // The group keeps its identity when a plan is added or removed.
    let fingerprint = primary.fingerprint.clone();
    let mut fewer = vec![plan_file("docs/plans/c.md"), plan_file("docs/plans/b.md")];
    super::grouping::group_repeats(&mut fewer);
    assert_eq!(fewer[1].findings[0].fingerprint, fingerprint);
}
