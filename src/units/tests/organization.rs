//! File organization: outlines of application and test files.
use super::*;

/// Two concerns of sixteen functions each: 258 lines, long enough that a
/// split is weighed (shorter files are notes).
fn two_concerns() -> String {
    let mut source = String::from("struct Cache { entries: Vec<u8> }\n");
    for i in 0..16 {
        source.push_str(&function(&format!("warm{i}")));
    }
    source.push_str("struct Page { body: String }\n");
    for i in 0..16 {
        source.push_str(&function(&format!("render{i}")));
    }
    source
}

#[test]
fn file_organization_review_without_a_module_choice_is_a_file_wide_finding() {
    let (project, options) = organized("lib.rs", &two_concerns());
    let report = run(&project, &options, &mut scripted(2));
    let file = &report.files[0];
    assert_eq!(file.status, Status::Review);
    let finding = &file.findings[0];
    assert!(
        finding.message.contains("several features"),
        "{}",
        finding.message
    );
    assert_eq!(finding.locations[0].start_line, 1);
    assert!(finding.symbol.is_none());
}

#[test]
fn an_uncertain_outline_is_rechecked_once_with_the_application_source() {
    let project = Project::new();
    let source = format!(
        "{}#[cfg(test)]\nmod tests {{\n    #[test]\n    fn hidden_check() {{\n        assert_eq!(1, 1);\n    }}\n}}\n",
        two_concerns()
    );
    project.write("lib.rs", &source);
    let (options, report) = run_rechecked(&project, catalog::FILE_ORGANIZATION, 0);
    assert_eq!(report.stages["outline"].successful_requests, 1);
    assert_eq!(report.stages["recheck"].successful_requests, 1);
    assert_eq!(
        report.files[0].dimensions["file_organization"].status,
        Status::Clear
    );
    let (_, plan) = planned(&project, &options);
    let (request, _) = plan.files.values().next().unwrap().units[0]
        .recheck
        .as_ref()
        .unwrap();
    let sent = request["state"]["file"]["source"].as_str().unwrap();
    assert!(sent.contains("fn warm0") && !sent.contains("hidden_check"));
}

#[test]
fn an_undecided_recheck_is_decided_by_the_kind_of_file() {
    let (project, mut options) = organized("lib.rs", &two_concerns());
    let (_, report) = run_rechecked(&project, catalog::FILE_ORGANIZATION, 3);
    assert_eq!(
        report.files[0].dimensions["file_organization"].status,
        Status::Clear,
        "one algorithm rules a split out"
    );
    let asked = |stage: &str| report.stages[stage].successful_requests;
    assert_eq!(
        (asked("recheck"), asked("trace")),
        (1, 1),
        "the kind is its own request"
    );
    options.refresh = true;
    let mut eval = scripted(3);
    let mut probabilities: serde_json::Map<String, Value> =
        questions::outline_kind(false)["criteria"]
            .as_object()
            .unwrap()
            .keys()
            .map(|k| (k.clone(), json!(0.0)))
            .collect();
    probabilities.insert("per_feature".into(), json!(0.85));
    probabilities.insert("algorithm".into(), json!(0.15));
    eval.recheck_overrides = vec![(
        "kind",
        json!({"type":"choice","choice":"per_feature","confidence":0.8,"probabilities":probabilities}),
    )];
    let finding = &first_finding(&project, &options, &mut eval);
    assert_eq!(finding.strength, Strength::Consider);
    assert!(
        finding
            .message
            .contains("same kind of code for several features"),
        "{}",
        finding.message
    );
}

#[test]
fn an_outline_too_long_for_a_recheck_is_decided_by_its_kind_alone() {
    let project = Project::new();
    // The comment makes the source too long to send whole; the outline is not.
    let padding = format!("// {}\n", "x".repeat(100)).repeat(1200);
    project.write("lib.rs", &format!("{}{padding}", two_concerns()));
    let (options, report) = run_rechecked(&project, catalog::FILE_ORGANIZATION, 3);
    assert_eq!(
        report.files[0].dimensions["file_organization"].status,
        Status::Clear,
        "one algorithm rules a split out"
    );
    let asked = |stage: &str| {
        report
            .stages
            .get(stage)
            .map_or(0, |s| s.successful_requests)
    };
    assert_eq!((asked("recheck"), asked("trace")), (0, 1));
    let (_, plan) = planned(&project, &options);
    let unit = &plan.files.values().next().unwrap().units[0];
    assert!(unit.recheck.is_none());
    let Detail::Outline {
        kind: Some((kind, _)),
        ..
    } = &unit.detail
    else {
        panic!("the kind is asked from the outline");
    };
    assert!(kind["state"]["file"]["source"].is_null());
}

#[test]
fn outlines_carry_member_and_file_sizes() {
    let (project, options) = rule_project(&two_concerns(), catalog::FILE_ORGANIZATION);
    let mut mock = Mock::default();
    run(&project, &options, &mut mock);
    let state = &mock.requests[0]["state"];
    assert_eq!(state["file"]["lines"], 2 + 32 * 8);
    assert_eq!(state["members"][1]["lines"], 8);
}

#[test]
fn a_split_of_a_short_file_is_a_note() {
    let short: String = (0..14).map(|i| function(&format!("warm{i}"))).collect();
    let (project, options) = organized("lib.rs", &short);
    let report = run(&project, &options, &mut scripted(2));
    assert_eq!(
        report.files[0].findings[0].strength,
        Strength::Note,
        "112 lines read easily whole"
    );
}

#[test]
fn a_group_that_holds_most_of_the_file_is_not_named() {
    let (project, mut options) = organized("tests/app.test.ts", &two_suites());
    let named = run(&project, &options, &mut moving_g1());
    assert_eq!(named.files[0].findings[0].strength, Strength::Consider);
    // Ten of twelve tests in the first suite: moving it would move the file.
    project.write("tests/app.test.ts", &suites(10, 2));
    options.refresh = true;
    let unnamed = run(&project, &options, &mut moving_g1());
    assert_eq!(
        unnamed.files[0].findings[0].strength,
        Strength::Note,
        "a consider that names no group is a note"
    );
}

/// A project holding `source` at `path`, checked for file organization only.
fn organized(path: &str, source: &str) -> (Project, CheckArgs) {
    let project = Project::new();
    project.write(path, source);
    let mut options = args();
    only(&mut options, catalog::FILE_ORGANIZATION);
    (project, options)
}

/// Answers at the top level that pick G1 as the group to move.
fn moving_g1() -> Scripted {
    let mut eval = scripted(2);
    eval.overrides = vec![("module", choice_of("G1", &["G1", "G2", "none"]))];
    eval
}

/// Two suites of six cases, each calling its own subject.
fn two_suites() -> String {
    suites(6, 6)
}

/// A suite of `parse` cases and one of `render` cases.
fn suites(parse: usize, render: usize) -> String {
    let mut source =
        String::from("import { parse } from './parse';\nimport { render } from './render';\n\n");
    for (subject, cases) in [("parse", parse), ("render", render)] {
        source.push_str(&format!("describe('{subject}', () => {{\n"));
        for i in 0..cases {
            source.push_str(&format!(
                "  it('case {i}', () => {{\n    const value = {subject}({{ id: {i} }});\n    expect(value.id).toBe({i});\n    expect(value).toBeDefined();\n    expect(value).not.toBeNull();\n    expect(typeof value).toBe('object');\n    expect(Object.keys(value)).toContain('id');\n    expect(value).toEqual({{ id: {i} }});\n{}  }});\n",
                "    expect(value).toBeTruthy();\n".repeat(12)
            ));
        }
        source.push_str("});\n");
    }
    source
}

#[test]
fn test_files_are_outlined_by_suite_without_include_tests() {
    let (project, mut options) = organized("tests/app.test.ts", &two_suites());
    let report = run(&project, &options, &mut moving_g1());
    let file = &report.files[0];
    assert_eq!(file.classification.as_ref().unwrap().kind, "tests");
    assert_eq!(
        file.status,
        Status::Consider,
        "test layout is one level lower"
    );
    let finding = &file.findings[0];
    assert!(
        finding.message.contains("separate test file") && finding.message.contains("G1"),
        "{}",
        finding.message
    );
    // Without a group to name, the finding is a note.
    options.refresh = true;
    let report = run(&project, &options, &mut scripted(2));
    assert_eq!(report.files[0].findings[0].strength, Strength::Note);
    options.refresh = false;
    let (_, plan) = planned(&project, &options);
    let (request, _) = plan.files.values().next().unwrap().units[0]
        .recheck
        .as_ref()
        .unwrap();
    let state = &request["state"];
    assert_eq!(state["members"][0]["kind"], "test");
    assert_eq!(state["members"][0]["suite"], "parse");
    let groups: Vec<usize> = state["groups"]
        .as_array()
        .unwrap()
        .iter()
        .map(|g| g["members"].as_array().unwrap().len())
        .collect();
    assert_eq!(groups, [6, 6], "one group per suite");
    assert!(
        request["questions"]["split"]["instructions"]["question"]
            .as_str()
            .unwrap()
            .contains("separate test file")
    );
    // Without file organization a test file is skipped as before.
    let mut options = args();
    only(&mut options, catalog::FUNCTION_SIMPLIFICATION);
    let mut mock = Mock::default();
    let report = run(&project, &options, &mut mock);
    assert_eq!(
        (mock.calls, report.files[0].status.clone()),
        (0, Status::NotApplicable)
    );
}

#[test]
fn a_java_test_outline_names_each_subject_by_the_class_that_owns_it() {
    let (project, options) = organized(
        "src/main/java/app/StringUtil.java",
        "package app;\n\nclass StringUtil {\n\tstatic boolean isBlank(String s) {\n\t\treturn s.isBlank();\n\t}\n\n\tstatic String join(String a, String b) {\n\t\treturn a + b;\n\t}\n}\n",
    );
    project.write( "src/main/java/app/Paths.java", "package app;\n\nclass Paths {\n\tstatic String join(String a, String b) {\n\t\treturn a + \"/\" + b;\n\t}\n}\n", );
    let mut tests = String::from("package app;\n\nclass StringUtilTest {\n");
    for i in 0..12 {
        let call = if i % 2 == 0 {
            "StringUtil.isBlank(\" \")"
        } else {
            "StringUtil.join(\"a\", \"b\")"
        };
        tests.push_str(&format!( "\t@Test\n\tvoid case{i}() {{\n\t\tObject value = {call};\n\t\tassertNotNull(value);\n\t\tassertEquals(value, value);\n\t\tassertTrue(value != null);\n\t\tassertFalse(value == null);\n\t\tassertSame(value, value);\n\t}}\n\n" ));
    }
    tests.push_str("}\n");
    project.write("src/test/java/app/StringUtilTest.java", &tests);
    let (_, plan) = planned(&project, &options);
    let outline = plan
        .requests
        .iter()
        .map(|p| &p.request["state"])
        .find(|s| s["file"]["path"] == "src/test/java/app/StringUtilTest.java")
        .unwrap();
    let subjects: Vec<&Value> = outline["members"]
        .as_array()
        .unwrap()
        .iter()
        .take(2)
        .map(|m| &m["subjects"][0])
        .collect();
    // `join` has two owners, so it stays a bare name.
    assert_eq!(subjects, [&json!("StringUtil::isBlank"), &json!("join")]);
    assert!(outline["members"][0].get("suite").is_none());
}

#[test]
fn short_files_are_too_small_to_split_and_never_clear() {
    let (project, options) = organized(
        "lib.rs",
        &format!("{}{}", function("warm"), function("render")),
    );
    let mut mock = Mock::default();
    let report = run(&project, &options, &mut mock);
    let dimension = &report.files[0].dimensions["file_organization"];
    assert_eq!((dimension.units.too_small, mock.calls), (1, 0));
    assert_eq!(dimension.status, Status::NotApplicable);
}
