//! Shared logic: candidate pairs across files and inside tests.
use super::*;

const LOAD: &str = "fn load_user(path: &str) -> Result<User> {\n    let text = std::fs::read_to_string(path)?;\n    let value: Value = serde_json::from_str(&text)?;\n    let name = value[\"name\"].as_str().unwrap_or(\"anonymous\").trim().to_string();\n    Ok(User { name })\n}\n";

#[test]
fn copies_inside_one_test_raise_at_most_a_consider() {
    let project = Project::new();
    let block = "    let text = std::fs::read_to_string(path).unwrap();\n    let value: Value = serde_json::from_str(&text).unwrap();\n    let name = value[\"name\"].as_str().unwrap_or(\"anonymous\").trim().to_string();\n    assert_eq!(name, expected);\n";
    let second = block.replace("text", "body").replace("value", "parsed");
    project.write(
        "tests/cases.rs",
        &format!("#[test]\nfn reads_names() {{\n    let path = \"a.json\";\n    let expected = \"a\";\n{block}    let path = \"b.json\";\n{second}}}\n"),
    );
    let mut options = args();
    options.include_tests = true;
    only(&mut options, catalog::SHARED_LOGIC);
    let mut same = scripted(2);
    same.overrides
        .push(("required", json!({"type":"noul","noul":0.05})));
    let finding = &first_finding(&project, &options, &mut same);
    assert_eq!(finding.strength, Strength::Consider);
    assert!(
        finding.message.contains("inside one test"),
        "{}",
        finding.message
    );
    options.refresh = true;
    let mut related = scripted(1);
    related
        .overrides
        .push(("required", json!({"type":"noul","noul":0.05})));
    let report = run(&project, &options, &mut related);
    assert_eq!(report.files[0].findings[0].strength, Strength::Note);
}

#[test]
fn copies_across_test_cases_are_one_level_lower_than_copies_in_support_code() {
    let block = "    let text = std::fs::read_to_string(path).unwrap();\n    let value: Value = serde_json::from_str(&text).unwrap();\n    let name = value[\"name\"].as_str().unwrap_or(\"anonymous\").trim().to_string();\n";
    let second = block.replace("text", "body").replace("value", "parsed");
    let cases = format!(
        "#[test]\nfn reads_a() {{\n    let path = \"a.json\";\n{block}    assert_eq!(name, \"a\");\n}}\n\n#[test]\nfn reads_b() {{\n    let path = \"b.json\";\n{second}    assert_eq!(name, \"b\");\n}}\n"
    );
    let support = format!(
        "#[test]\nfn loads() {{\n    assert_eq!(load_a(\"a.json\"), load_b(\"b.json\"));\n}}\n\nfn load_a(path: &str) -> String {{\n{block}    name\n}}\n\nfn load_b(path: &str) -> String {{\n{second}    name\n}}\n"
    );
    let mut options = args();
    options.include_tests = true;
    only(&mut options, catalog::SHARED_LOGIC);
    let strength = |source: &str| {
        let project = Project::new();
        project.write("tests/cases.rs", source);
        let mut same = scripted(2);
        same.overrides
            .push(("required", json!({"type":"noul","noul":0.05})));
        let report = run(&project, &options, &mut same);
        let finding = report.files[0].findings[0].clone();
        (finding.strength, finding.message)
    };
    let (in_cases, message) = strength(&cases);
    assert_eq!(in_cases, Strength::Consider);
    assert!(message.contains("across test cases"), "{message}");
    assert_eq!(strength(&support).0, Strength::Review);
}

#[test]
fn duplicate_pairs_across_files_quote_both_sites_and_respect_required_repetition() {
    let project = Project::new();
    project.write("a.rs", LOAD);
    project.write(
        "b.rs",
        &LOAD
            .replace("load_user", "load_team")
            .replace("\"name\"", "\"title\""),
    );
    let mut options = args();
    only(&mut options, catalog::SHARED_LOGIC);
    let mut same = scripted(2);
    same.overrides
        .push(("required", json!({"type":"noul","noul":0.05})));
    let report = run(&project, &options, &mut same);
    assert_eq!(report.stages["duplicate-pair"].successful_requests, 1);
    let a = &report.files[0];
    let finding = &a.findings[0];
    assert_eq!(finding.rule, "maintainability/shared-logic");
    assert_eq!(finding.locations.len(), 2);
    assert_eq!(finding.locations[1].path, std::path::Path::new("b.rs"));
    assert!(finding.quote.as_ref().unwrap().starts_with("let text"));
    assert!(
        finding.message.contains("`\"name\"`→`\"title\"`")
            || finding.message.contains("`name`→`title`"),
        "{}",
        finding.message
    );
    assert!(
        report.files[1].findings.is_empty(),
        "the pair is reported once"
    );
    assert_eq!(
        report.files[1].dimensions["shared_logic"].status,
        Status::NotApplicable
    );
    options.refresh = true;
    let mut required = scripted(2);
    required
        .overrides
        .push(("required", json!({"type":"noul","noul":0.9})));
    let report = run(&project, &options, &mut required);
    assert_eq!(
        report.files[0].dimensions["shared_logic"].status,
        Status::Clear
    );
}
