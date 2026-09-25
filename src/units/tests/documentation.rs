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

/// A repetition finding in `path` between its section at `line` and a
/// section of `other`.
fn repeat_file(path: &str, line: usize, other: (&str, usize)) -> crate::schema::FileResult {
    let mut file = plan_file(path);
    let dimension = file.dimensions.remove("doc_staleness").unwrap();
    file.dimensions.insert("doc_duplication".into(), dimension);
    let finding = &mut file.findings[0];
    finding.rule = "documentation/duplication".into();
    finding.category = None;
    finding.line = line;
    finding.message = format!(
        "Section `Setup` states everything section `Setup` of `{}` states (0.98).",
        other.0
    );
    let at = |path: &str, line: usize| crate::schema::Location {
        path: path.into(),
        start_line: line,
        end_line: line + 4,
        symbol: Some("Setup".into()),
    };
    finding.locations = vec![at(path, line), at(other.0, other.1)];
    file
}

#[test]
fn a_section_repeated_in_several_documents_is_one_finding() {
    use crate::schema::Strength;
    let hub = ("docs/hub.md", 1);
    let mut files = vec![
        repeat_file("docs/b.md", 3, hub),
        repeat_file("docs/a.md", 5, hub),
        repeat_file("docs/c.md", 7, hub),
        repeat_file("docs/y.md", 2, ("docs/z.md", 4)),
        repeat_file("docs/x.md", 1, hub),
        repeat_file("docs/a.md", 20, hub),
    ];
    files[4].findings[0].category = Some("conflict".into());
    let primary = grouped_primary(&mut files);
    assert_eq!(primary.strength, Strength::Consider);
    assert!(
        primary
            .message
            .ends_with("The same text recurs in 3 more sections: section `Setup` of `docs/a.md`, `docs/b.md`, `docs/c.md`."),
        "{}",
        primary.message
    );
    assert!(primary.action.starts_with("Keep one copy"));
    let located: Vec<(&str, usize)> = primary
        .locations
        .iter()
        .map(|l| (l.path.to_str().unwrap(), l.start_line))
        .collect();
    assert_eq!(
        located,
        [
            ("docs/a.md", 5),
            ("docs/hub.md", 1),
            ("docs/b.md", 3),
            ("docs/c.md", 7),
            ("docs/a.md", 20)
        ]
    );
    for member in [&files[0], &files[2], &files[5]] {
        assert_eq!(member.findings[0].strength, Strength::Note);
        assert!(
            member.findings[0]
                .message
                .ends_with("Grouped with the other copies of this section at docs/a.md:5."),
            "{}",
            member.findings[0].message
        );
    }
    assert_eq!(
        files[3].findings[0].strength,
        Strength::Consider,
        "a lone pair"
    );
    assert_eq!(
        files[4].findings[0].strength,
        Strength::Consider,
        "a disagreement names its own fix"
    );
    // The group is identified by its head section, whichever pair leads it.
    let fingerprint = primary.fingerprint.clone();
    let mut fewer = vec![
        repeat_file("docs/c.md", 7, hub),
        repeat_file("docs/b.md", 3, hub),
    ];
    super::grouping::group_repeats(&mut fewer);
    assert_eq!(fewer[1].findings[0].fingerprint, fingerprint);
}

#[test]
fn package_readmes_are_not_paired_and_a_family_is_asked_against_its_head() {
    let project = Project::new();
    project.write("package.json", r#"{"name":"root","private":true}"#);
    let setup = "Install the package with your package manager, add the API key to your environment and import the provider instance before you call generate text";
    for name in ["alpha", "beta"] {
        project.write(
            &format!("packages/{name}/package.json"),
            &format!(r#"{{"name":"{name}"}}"#),
        );
        project.write(
            &format!("packages/{name}/README.md"),
            &format!("# {name}\n\n## Setup\n\n{setup}.\n"),
        );
    }
    let prerequisites = "To follow this quickstart you need Node.js 22 or later, pnpm installed on your local development machine and a gateway API key from the dashboard";
    for page in ["next", "nuxt", "svelte"] {
        project.write(
            &format!("docs/{page}.mdx"),
            &format!(
                "---\ntitle: {page}\n---\nimport {{ Note }} from '@/components';\n\n# {page} quickstart\n\n## Prerequisites\n\n<Note>\n  {prerequisites}.\n</Note>\n"
            ),
        );
    }
    let mut options = args();
    only(&mut options, catalog::DOC_DUPLICATION);
    let mut eval = scripted(2);
    eval.overrides = vec![
        ("conflict", spread(0.98, 0.02, 0.0)),
        ("translation", noul_at(0.02)),
    ];
    let report = run(&project, &options, &mut eval);
    let findings: Vec<&crate::schema::Finding> =
        report.files.iter().flat_map(|f| &f.findings).collect();
    assert!(
        findings
            .iter()
            .all(|f| f.locations.iter().all(|l| !l.path.starts_with("packages"))),
        "{findings:?}"
    );
    let judged: usize = report
        .files
        .iter()
        .filter_map(|f| f.dimensions.get(catalog::DOC_DUPLICATION))
        .map(|d| d.units.judged)
        .sum();
    assert_eq!(judged, 2, "two members against their head, not three pairs");
    let considers: Vec<_> = findings
        .iter()
        .filter(|f| f.strength == Strength::Consider)
        .collect();
    assert_eq!(considers.len(), 1, "{findings:?}");
    let message = &considers[0].message;
    assert!(message.starts_with("Section `Prerequisites`"), "{message}");
    assert!(
        message.contains("The same text recurs in 1 more section: `docs/"),
        "{message}"
    );
    assert_eq!(considers[0].locations.len(), 3);
    assert_eq!(
        considers[0].locations[0].start_line, 8,
        "the file's own line"
    );
}

const RELATIONS: [&str; 5] = ["alike", "contradict", "different", "overlap", "repeats"];

/// A relation Choice torn between repeating, contradicting and overlapping,
/// which settles nothing.
fn torn_relation() -> Value {
    json!({"type":"choice","choice":"overlap","confidence":0.3,"probabilities":{
        "alike":0.05,"contradict":0.3,"different":0.05,"overlap":0.3,"repeats":0.3}})
}

/// Two documents sharing a section, checked for repetition with the given
/// answers, the relation torn unless `overrides` names it; the README's
/// duplication dimension and findings.
fn pair_answered(
    overrides: Vec<(&'static str, Value)>,
) -> (crate::schema::Dimension, Vec<crate::schema::Finding>) {
    let overrides = std::iter::once(("relation", torn_relation()))
        .chain(overrides)
        .collect();
    let project = Project::new();
    let shared = "Install the dependencies, start the local database, copy the example environment file and run the development server before opening a pull request";
    project.write("README.md", &format!("# Setup\n{shared}.\n"));
    project.write("docs/guide.md", &format!("# Getting started\n{shared}.\n"));
    let mut options = args();
    only(&mut options, catalog::DOC_DUPLICATION);
    let mut eval = scripted(0);
    eval.overrides = overrides;
    let report = run(&project, &options, &mut eval);
    let readme = report
        .files
        .into_iter()
        .find(|f| f.path == std::path::Path::new("README.md"))
        .unwrap();
    (
        readme.dimensions[catalog::DOC_DUPLICATION].clone(),
        readme.findings,
    )
}

#[test]
fn a_section_repeating_most_of_another_is_a_note_that_never_claims_all() {
    let (dimension, findings) = pair_answered(vec![
        ("a_covers", spread(0.05, 0.45, 0.5)),
        ("subject", noul_at(0.95)),
    ]);
    assert_eq!(dimension.units.note, 1, "{}", dimension.decision_basis);
    assert_eq!(findings[0].strength, Strength::Note);
    assert!(
        findings[0]
            .message
            .starts_with("Section `Setup` states most or all of what section `Getting started`"),
        "{}",
        findings[0].message
    );
}

#[test]
fn a_disagreement_that_leads_is_a_note_and_detail_is_no_disagreement() {
    let (dimension, findings) = pair_answered(vec![
        ("conflict", spread(0.1, 0.3, 0.6)),
        ("subject", noul_at(0.95)),
    ]);
    assert_eq!(dimension.units.note, 1, "{}", dimension.decision_basis);
    assert_eq!(findings[0].category.as_deref(), Some("conflict"));
    assert!(
        findings[0]
            .message
            .contains("may give different values or instructions for the same thing (0.60)"),
        "{}",
        findings[0].message
    );
    // Differing only in detail is acceptable, like agreeing.
    let (dimension, findings) = pair_answered(vec![("conflict", spread(0.2, 0.7, 0.1))]);
    assert_eq!(dimension.units.clear, 1, "{}", dimension.decision_basis);
    assert!(findings.is_empty(), "{findings:?}");
}

#[test]
fn different_subjects_settle_undecided_repetition_but_not_a_decided_one() {
    let undecided = || ("b_covers", spread(0.4, 0.3, 0.3));
    let (dimension, _) = pair_answered(vec![undecided(), ("subject", noul_at(0.05))]);
    assert_eq!(dimension.units.clear, 1, "{}", dimension.decision_basis);
    let (dimension, _) = pair_answered(vec![undecided(), ("subject", noul_at(0.95))]);
    assert_eq!(dimension.units.uncertain, 1, "{}", dimension.decision_basis);
    assert_eq!(
        dimension.undecided[0].questions,
        ["repeats the other section"]
    );
    let (dimension, findings) = pair_answered(vec![
        ("a_covers", spread(0.0, 0.05, 0.95)),
        ("subject", noul_at(0.05)),
    ]);
    assert_eq!(dimension.units.consider, 1, "{}", dimension.decision_basis);
    assert!(findings[0].message.contains("states everything"));
}

#[test]
fn a_relation_that_rules_out_repetition_settles_only_undecided_checks() {
    let undecided = || ("b_covers", spread(0.4, 0.3, 0.3));
    let same = || ("subject", noul_at(0.95));
    let (dimension, _) = pair_answered(vec![
        undecided(),
        same(),
        ("relation", choice_of("alike", &RELATIONS)),
    ]);
    assert_eq!(dimension.units.clear, 1, "{}", dimension.decision_basis);
    // A leaning conflict stays a note: the relation only clears what is
    // undecided, and a repetition it names settles nothing.
    let (dimension, findings) = pair_answered(vec![
        undecided(),
        ("conflict", spread(0.1, 0.3, 0.6)),
        same(),
        ("relation", choice_of("repeats", &RELATIONS)),
    ]);
    assert_eq!(dimension.units.note, 1, "{}", dimension.decision_basis);
    assert!(findings[0].message.contains("may give different values"));
    let (dimension, _) = pair_answered(vec![
        ("a_covers", spread(0.0, 0.05, 0.95)),
        same(),
        ("relation", choice_of("different", &RELATIONS)),
    ]);
    assert_eq!(dimension.units.consider, 1, "{}", dimension.decision_basis);
}

/// The agent-context dimension of a one-section `AGENTS.md` answered with
/// `overrides`, and `kind` for the section's kind when it is asked.
fn instruction_dimension(
    overrides: Vec<(&'static str, Value)>,
    kind: Value,
) -> (crate::schema::Dimension, Vec<crate::schema::Finding>) {
    let mut options = args();
    only(&mut options, catalog::AGENT_CONTEXT);
    // A fresh project each time, so no answer comes from the cache.
    let project = Project::new();
    project.write("Cargo.toml", "[package]\nname = \"demo\"\n");
    project.write("src/lib.rs", "");
    project.write(
        "AGENTS.md",
        "# Testing\nTests live beside the code and use the fixtures in `testdata/`.\n",
    );
    let mut eval = scripted(0);
    eval.overrides = overrides;
    eval.recheck_overrides = vec![("kind", kind)];
    let report = run(&project, &options, &mut eval);
    let file = report
        .files
        .into_iter()
        .find(|f| f.path == std::path::Path::new("AGENTS.md"))
        .unwrap();
    (
        file.dimensions[catalog::AGENT_CONTEXT].clone(),
        file.findings,
    )
}

const SECTION_KINDS: [&str; 5] = [
    "commands",
    "description",
    "generic",
    "instructions",
    "record",
];

/// A kind Choice torn between instructions, a description and generic
/// advice, which settles none of them.
fn torn_kind() -> Value {
    json!({"type":"choice","choice":"instructions","confidence":0.3,"probabilities":{
        "commands":0.05,"description":0.3,"generic":0.3,"instructions":0.3,"record":0.05}})
}

#[test]
fn an_instruction_section_the_files_do_not_show_settles_description_and_commands() {
    let undecided = || noul_at(0.5);
    let (settled, _) = instruction_dimension(
        vec![("s0_describes", undecided()), ("s0_commands", undecided())],
        torn_kind(),
    );
    assert_eq!(settled.units.clear, 1, "{}", settled.decision_basis);
    let (open, _) = instruction_dimension(
        vec![("s0_describes", undecided()), ("s0_generic", undecided())],
        torn_kind(),
    );
    assert_eq!(open.units.uncertain, 1, "{}", open.decision_basis);
    assert_eq!(open.undecided[0].questions, ["generic advice"]);
    let (unsure, _) = instruction_dimension(
        vec![
            ("s0_inferable", spread(0.4, 0.2, 0.4)),
            ("s0_describes", undecided()),
        ],
        torn_kind(),
    );
    assert_eq!(
        unsure.undecided[0].questions,
        ["restates the repository", "description only"]
    );
}

#[test]
fn an_undecided_instruction_section_settles_by_its_kind() {
    let undecided = || noul_at(0.5);
    // Instructions clear every undecided signal, "restates" included.
    let (cleared, _) = instruction_dimension(
        vec![
            ("s0_inferable", spread(0.4, 0.2, 0.4)),
            ("s0_describes", undecided()),
            ("s0_generic", undecided()),
        ],
        choice_of("instructions", &SECTION_KINDS),
    );
    assert_eq!(cleared.units.clear, 1, "{}", cleared.decision_basis);
    // A section's own kind raises its undecided signal, at most a consider.
    let (raised, findings) = instruction_dimension(
        vec![("s0_generic", undecided())],
        choice_of("generic", &SECTION_KINDS),
    );
    assert_eq!(raised.units.consider, 1, "{}", raised.decision_basis);
    assert!(
        findings[0]
            .message
            .contains("gives only advice that applies to any project (0.90)"),
        "{}",
        findings[0].message
    );
    // A decided signal is never moved by the kind.
    let (decided, _) = instruction_dimension(
        vec![("s0_describes", noul_at(0.95))],
        choice_of("instructions", &SECTION_KINDS),
    );
    assert_eq!(decided.units.consider, 1, "{}", decided.decision_basis);
}

const DOCUMENT_KINDS: [&str; 5] = [
    "collection",
    "guide",
    "introduction",
    "migration",
    "reference",
];

/// A long guide checked for large docs, its split answered with `split`
/// and its kind, when asked, with `kind`.
fn large_doc(split: Value, kind: Value) -> crate::schema::FileResult {
    let project = Project::new();
    let mut text = String::from("# Quickstart\n");
    for part in ["Install", "Routing", "Templates", "Sessions", "Deploying"] {
        text.push_str(&format!("\n## {part}\n\n"));
        text.push_str(&"A line of the guide.\n".repeat(70));
    }
    project.write("docs/quickstart.md", &text);
    let mut options = args();
    only(&mut options, catalog::LARGE_DOCS);
    let mut eval = scripted(0);
    eval.overrides = vec![("split", split), ("history", noul_at(0.05))];
    eval.recheck_overrides = vec![("kind", kind)];
    run(&project, &options, &mut eval)
        .files
        .into_iter()
        .find(|f| f.path == std::path::Path::new("docs/quickstart.md"))
        .unwrap()
}

#[test]
fn an_undecided_large_document_is_asked_its_kind() {
    let torn = || spread(0.31, 0.38, 0.31);
    let guide = large_doc(torn(), choice_of("guide", &DOCUMENT_KINDS));
    let dimension = &guide.dimensions[catalog::LARGE_DOCS];
    assert_eq!(dimension.units.clear, 1, "{}", dimension.decision_basis);
    let collection = large_doc(torn(), choice_of("collection", &DOCUMENT_KINDS));
    assert_eq!(collection.findings.len(), 1);
    assert_eq!(collection.findings[0].strength, Strength::Consider);
    assert!(
        collection.findings[0]
            .message
            .contains("holds several unrelated subjects (0.90)"),
        "{}",
        collection.findings[0].message
    );
    // A decided split is not asked its kind.
    let decided = large_doc(
        spread(0.9, 0.1, 0.0),
        choice_of("collection", &DOCUMENT_KINDS),
    );
    assert_eq!(decided.status, Status::Clear);
    assert!(
        decided.judgments.iter().all(|j| j.question != "kind"),
        "{:?}",
        decided.judgments
    );
}
