//! Shared logic: candidate pairs across files and inside tests.
use super::*;

pub(super) const LOAD: &str = "fn load_user(path: &str) -> Result<User> {\n    let text = std::fs::read_to_string(path)?;\n    let value: Value = serde_json::from_str(&text)?;\n    let name = value[\"name\"].as_str().unwrap_or(\"anonymous\").trim().to_string();\n    Ok(User { name })\n}\n";

#[test]
fn copies_inside_one_test_raise_at_most_a_consider() {
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
fn copies_in_test_code_are_at_most_a_consider_and_across_cases_one_level_lower() {
    let block = "    let text = std::fs::read_to_string(path).unwrap();\n    let value: Value = serde_json::from_str(&text).unwrap();\n    let name = value[\"name\"].as_str().unwrap_or(\"anonymous\").trim().to_string();\n    let name = name.to_lowercase();\n";
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
    let at = |source: &str, level: usize| {
        let project = Project::new();
        project.write("tests/cases.rs", source);
        let mut same = scripted(level);
        same.overrides
            .push(("required", json!({"type":"noul","noul":0.05})));
        let report = run(&project, &options, &mut same);
        let finding = report.files[0].findings[0].clone();
        (finding.strength, finding.message)
    };
    let strength = |source: &str| at(source, 2);
    let (in_cases, message) = strength(&cases);
    assert_eq!(in_cases, Strength::Consider);
    assert!(message.contains("across test cases"), "{message}");
    // Support code of tests: a review is a consider, a consider stays one.
    assert_eq!(strength(&support).0, Strength::Consider);
    assert_eq!(at(&support, 1).0, Strength::Consider);
    assert_eq!(at(&cases, 1).0, Strength::Note);
    // A short copy across test cases, such as a login step, is a note.
    let short = cases
        .replace("    let name = name.to_lowercase();\n", "")
        .replace(
            "    let text = std::fs::read_to_string(path).unwrap();\n",
            "",
        )
        .replace(
            "    let body = std::fs::read_to_string(path).unwrap();\n",
            "",
        );
    assert_eq!(strength(&short).0, Strength::Note);
    // The same cases in two test files mirror each other: a note.
    let project = Project::new();
    let (a, b) = cases.split_at(cases.find("\n\n#[test]").unwrap());
    project.write("tests/a.rs", a);
    project.write("tests/b.rs", b.trim_start());
    let mut same = scripted(2);
    same.overrides
        .push(("required", json!({"type":"noul","noul":0.05})));
    let report = run(&project, &options, &mut same);
    let finding = report
        .files
        .iter()
        .flat_map(|f| &f.findings)
        .next()
        .unwrap();
    assert_eq!(finding.strength, Strength::Note, "{}", finding.message);
}

#[test]
fn short_copies_are_at_most_a_consider() {
    let body = "\t\tStringBuilder builder = StringUtil.borrowBuilder();\n\t\thtml(QuietAppendable.wrap(builder), new Document.OutputSettings());\n\t\treturn StringUtil.releaseBuilder(builder);\n";
    let longer = "\t\tStringBuilder builder = StringUtil.borrowBuilder();\n\t\thtml(QuietAppendable.wrap(builder), new Document.OutputSettings());\n\t\tbuilder.append(tagName).append(attributes.size());\n\t\tbuilder.append(namespace);\n\t\treturn StringUtil.releaseBuilder(builder);\n";
    let mut options = args();
    only(&mut options, catalog::SHARED_LOGIC);
    let strength = |body: &str| {
        let project = Project::new();
        for class in ["Attribute", "Attributes"] {
            project.write(
                &format!("src/main/java/app/{class}.java"),
                &format!("package app;\n\nclass {class} {{\n\tString html() {{\n{body}\t}}\n}}\n"),
            );
        }
        let mut same = scripted(2);
        same.overrides
            .push(("required", json!({"type":"noul","noul":0.05})));
        let report = run(&project, &options, &mut same);
        let finding = report
            .files
            .iter()
            .flat_map(|f| &f.findings)
            .next()
            .unwrap()
            .clone();
        (finding.strength, finding.message)
    };
    let (short, message) = strength(body);
    assert_eq!(short, Strength::Consider);
    assert!(message.contains("a person should decide"), "{message}");
    assert_eq!(strength(longer).0, Strength::Review);
}

#[test]
fn a_consider_from_the_same_steps_answer_needs_its_measured_threshold() {
    let short_copy = "\t\tStringBuilder builder = StringUtil.borrowBuilder();\n\t\thtml(QuietAppendable.wrap(builder), new Document.OutputSettings());\n\t\treturn StringUtil.releaseBuilder(builder);\n";
    let mut options = args();
    only(&mut options, catalog::SHARED_LOGIC);
    let finding = |files: &[(String, String)], same: Value| {
        let project = Project::new();
        for (path, text) in files {
            project.write(path, text);
        }
        let mut answers = scripted(2);
        answers
            .overrides
            .push(("required", json!({"type":"noul","noul":0.05})));
        answers.overrides.push(("same", same));
        let report = run(&project, &options, &mut answers);
        report
            .files
            .iter()
            .flat_map(|f| &f.findings)
            .next()
            .cloned()
    };
    let strength =
        |files: &[(String, String)], same: Value| finding(files, same).map(|f| f.strength);
    let copies = [
        ("a.rs".to_string(), LOAD.to_string()),
        (
            "b.rs".to_string(),
            LOAD.replace("load_user", "load_team")
                .replace("\"name\"", "\"title\""),
        ),
    ];
    // 0.15 on "different work that only looks alike": a note, which does
    // not call application code test cases.
    let note = finding(&copies, spread(0.15, 0.35, 0.5)).unwrap();
    assert_eq!(note.strength, Strength::Note);
    assert!(
        note.message.contains("repeat related steps") && !note.message.contains("test"),
        "{}",
        note.message
    );
    assert_eq!(
        strength(&copies, spread(0.1, 0.4, 0.5)),
        Some(Strength::Consider)
    );
    // A short copy's review lowered to a consider: the top level set it.
    let java = ["Attribute", "Attributes"].map(|class| {
        (
            format!("src/main/java/app/{class}.java"),
            format!("package app;\n\nclass {class} {{\n\tString html() {{\n{short_copy}\t}}\n}}\n"),
        )
    });
    assert_eq!(
        strength(&java, spread(0.15, 0.0, 0.85)),
        Some(Strength::Consider)
    );
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

#[test]
fn copies_in_example_code_are_notes() {
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
    let mut same = scripted(2);
    same.overrides
        .push(("required", json!({"type":"noul","noul":0.05})));
    let report = run(&project, &options, &mut same);
    let finding = &report.files[0].findings[0];
    assert_eq!(finding.strength, Strength::Note);
}
