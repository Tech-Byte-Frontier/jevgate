//! Shared logic: candidate groups across files and inside tests, asked one
//! look-here question each.
use super::*;

pub(super) const LOAD: &str = "fn load_user(path: &str) -> Result<User> {\n    let text = std::fs::read_to_string(path)?;\n    let value: Value = serde_json::from_str(&text)?;\n    let name = value[\"name\"].as_str().unwrap_or(\"anonymous\").trim().to_string();\n    Ok(User { name })\n}\n";

/// `LOAD` and its copy loading a team, in `a.rs` and `b.rs`.
fn copied() -> Project {
    let project = Project::new();
    project.write("a.rs", LOAD);
    project.write(
        "b.rs",
        &LOAD
            .replace("load_user", "load_team")
            .replace("\"name\"", "\"title\""),
    );
    project
}

#[test]
fn copies_across_files_are_one_look_finding_quoting_every_site() {
    let project = copied();
    let mut options = args();
    only(&mut options, catalog::SHARED_LOGIC);
    let mut mock = Mock::default();
    run(&project, &options, &mut mock);
    let request = &mock.requests[0];
    let questions: Vec<&String> = request["questions"].as_object().unwrap().keys().collect();
    assert_eq!(questions, ["look"]);
    let sites = request["state"]["sites"].as_array().unwrap();
    assert_eq!(sites.len(), 2);
    assert_eq!(sites[1]["path"], "b.rs");
    assert_eq!(sites[0]["function"], "load_user");
    options.refresh = true;
    let report = run(&project, &options, &mut scripted(2));
    assert_eq!(report.stages["duplicate-pair"].successful_requests, 1);
    let finding = &report.files[0].findings[0];
    assert_eq!(finding.rule, "maintainability/shared-logic");
    assert_eq!(
        (finding.strength, finding.measured_as),
        (Strength::Review, None)
    );
    assert_eq!(finding.locations.len(), 2);
    assert_eq!(finding.locations[1].path, std::path::Path::new("b.rs"));
    assert!(finding.quote.as_ref().unwrap().starts_with("let text"));
    assert!(
        finding.message.contains("may repeat one piece of logic"),
        "{}",
        finding.message
    );
    assert!(report.files[1].findings.is_empty(), "reported once");
    options.refresh = true;
    let report = run(&project, &options, &mut scripted(1));
    assert_eq!(
        report.files[0].dimensions["shared_logic"].status,
        Status::Clear,
        "below the look probability"
    );
}

#[test]
fn a_short_rule_written_out_in_two_files_is_a_candidate() {
    // One line each: no statement window, but a run of repeated tokens
    // whose local names differ and whose member names and literals match.
    let hero = "export function Hero({ place }: Props) {\n  const settlement = place.kind === undefined || place.kind === 'settlement'\n  return settlement ? <Banner /> : null\n}\n";
    let page = "export function Page(props: PageProps) {\n  const location = props.location\n  const town = location.kind === undefined || location.kind === 'settlement'\n  return <Layout wide={town} />\n}\n";
    let (project, options) = project_with(
        &[("src/Hero.tsx", hero), ("src/Page.tsx", page)],
        &[catalog::SHARED_LOGIC],
    );
    let report = run(&project, &options, &mut scripted(2));
    let finding = &report.files[0].findings[0];
    assert_eq!(finding.locations.len(), 2);
    assert!(
        finding
            .quote
            .as_ref()
            .unwrap()
            .contains(".kind === undefined"),
        "{:?}",
        finding.quote
    );
    // A shape of names and punctuation alone is no rule to share.
    let calls = "export function a(x: X) {\n  const y = f(x, g(x), h(x, x), k(x))\n  return y\n}\n";
    let (project, options) = project_with(
        &[
            ("src/a.ts", calls),
            ("src/b.ts", &calls.replace("function a", "function b")),
        ],
        &[catalog::SHARED_LOGIC],
    );
    let (_, plan) = planned(&project, &options);
    assert!(stages(&plan).is_empty(), "{:?}", stages(&plan));
}

#[test]
fn copies_inside_tests_are_candidates_when_tests_are_judged() {
    let project = Project::new();
    let block = "    let text = std::fs::read_to_string(path).unwrap();\n    let value: Value = serde_json::from_str(&text).unwrap();\n    let name = value[\"name\"].as_str().unwrap_or(\"anonymous\").trim().to_string();\n    let name = name.to_lowercase();\n    assert_eq!(name, expected);\n";
    let second = block.replace("text", "body").replace("value", "parsed");
    project.write(
        "tests/cases.rs",
        &format!("#[test]\nfn reads_names() {{\n    let path = \"a.json\";\n    let expected = \"a\";\n{block}    let path = \"b.json\";\n{second}}}\n"),
    );
    let mut options = args();
    options.include_tests = true;
    only(&mut options, catalog::SHARED_LOGIC);
    let report = run(&project, &options, &mut scripted(2));
    let finding = &report.files[0].findings[0];
    assert_eq!(finding.strength, Strength::Review);
    assert!(finding.symbol.is_some() || !finding.locations.is_empty());
    // Without judging tests, the test file's copies are not compared.
    options.include_tests = false;
    options.refresh = true;
    let mut mock = Mock::default();
    run(&project, &options, &mut mock);
    assert_eq!(mock.calls, 0);
}

#[test]
fn copies_in_example_code_are_not_reported() {
    let project = Project::new();
    project.write("examples/login/apis.rs", LOAD);
    project.write(
        "examples/login/views.rs",
        &LOAD
            .replace("load_user", "load_team")
            .replace("\"name\"", "\"title\""),
    );
    let mut options = args();
    only(&mut options, catalog::SHARED_LOGIC);
    let report = run(&project, &options, &mut scripted(2));
    assert!(report.files[0].findings.is_empty());
    assert_eq!(
        report.files[0].dimensions["shared_logic"].status,
        Status::Clear,
        "examples spell a flow out on purpose"
    );
}
