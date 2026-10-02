//! File organization: outlines of application and test files, asked one
//! look-here question each.
use super::*;

/// Two concerns of sixteen functions each: 258 lines, past the floor below
/// which a file is too small to split.
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
fn a_file_the_look_question_flags_is_one_review_for_an_agent_to_verify() {
    let (project, options) = organized("lib.rs", &two_concerns());
    let mut eval = scripted(2);
    let report = run(&project, &options, &mut eval);
    assert_eq!(eval.stages, ["first"], "no recheck, kind or parts");
    let file = &report.files[0];
    assert_eq!(file.status, Status::Review);
    let finding = &file.findings[0];
    assert_eq!(finding.strength, Strength::Review);
    assert_eq!(
        finding.measured_as, None,
        "a look-here finding is not yet measured"
    );
    assert_eq!(finding.gate, Some(crate::schema::Gating::Measuring));
    assert!(
        finding.message.contains("several separate kinds of work")
            && finding
                .action
                .contains("dismiss this finding with a reason"),
        "{}",
        finding.message
    );
    assert_eq!(finding.locations[0].start_line, 1);
    assert!(finding.symbol.is_none());
    // Below the look probability the outline is clear: there is no middle.
    for level in [0, 1] {
        let (project, options) = organized("lib.rs", &two_concerns());
        let report = run(&project, &options, &mut scripted(level));
        let dimension = &report.files[0].dimensions["file_organization"];
        assert_eq!(
            (dimension.status.clone(), dimension.units.clear),
            (Status::Clear, 1)
        );
    }
}

#[test]
fn a_function_another_file_passes_by_path_names_that_file_as_its_user() {
    let project = Project::new();
    let selectors: String = (0..14).map(|i| function(&format!("warm{i}"))).collect();
    project.write("src/selectors.rs", &selectors);
    project.write(
        "src/runner.rs",
        "use crate::selectors;\n\npub fn run(values: &[i32]) -> i32 {\n    apply(values, selectors::warm0)\n}\n\nfn apply(values: &[i32], f: fn(&[i32]) -> i32) -> i32 {\n    f(values)\n}\n",
    );
    let mut options = args();
    only(&mut options, catalog::FILE_ORGANIZATION);
    let (_, plan) = planned(&project, &options);
    let outline = plan
        .requests
        .iter()
        .map(|p| &p.request)
        .find(|r| r["state"]["file"]["path"] == "src/selectors.rs")
        .unwrap();
    let used_by = |name: &str| {
        outline["state"]["members"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["name"] == name)
            .unwrap()["used_by"]
            .clone()
    };
    assert_eq!(used_by("warm0"), json!(["src/runner.rs"]));
    assert!(used_by("warm1").is_null());
}

#[test]
fn outlines_carry_member_and_file_sizes() {
    let (project, options) = rule_project(&two_concerns(), catalog::FILE_ORGANIZATION);
    let mut mock = Mock::default();
    run(&project, &options, &mut mock);
    let request = &mock.requests[0];
    let state = &request["state"];
    assert_eq!(state["file"]["lines"], 2 + 32 * 8);
    assert_eq!(state["members"][1]["lines"], 8);
    let questions: Vec<&String> = request["questions"].as_object().unwrap().keys().collect();
    assert_eq!(questions, ["look"]);
}

/// A project holding `source` at `path`, checked for file organization only.
fn organized(path: &str, source: &str) -> (Project, CheckArgs) {
    let project = Project::new();
    project.write(path, source);
    let mut options = args();
    only(&mut options, catalog::FILE_ORGANIZATION);
    (project, options)
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
    let (project, options) = organized("tests/app.test.ts", &two_suites());
    let report = run(&project, &options, &mut scripted(2));
    let file = &report.files[0];
    assert_eq!(file.classification.as_ref().unwrap().kind, "tests");
    assert_eq!(file.status, Status::Review);
    let finding = &file.findings[0];
    assert!(
        finding.message.contains("several separate subjects"),
        "{}",
        finding.message
    );
    let (_, plan) = planned(&project, &options);
    let request = &plan.requests[0].request;
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
        request["questions"]["look"]["instructions"]["note"]
            .as_str()
            .unwrap()
            .contains("test cases")
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

#[test]
fn a_bend_file_is_weighed_for_a_split_only_past_its_own_floor() {
    let defs = |count: usize| -> String {
        let mut source = String::from("import Base\n\n");
        for i in 0..count {
            source.push_str(&format!(
                "def f{i}(x: Nat) -> Nat:\n  match x:\n    case 0n:\n      1n\n    case 1n+p:\n      f{i}(p)\n\n"
            ));
        }
        source
    };
    // 240 member lines: past the floor of other languages, not Bend's.
    let (project, options) = organized("src/lib.bend", &defs(40));
    let mut mock = Mock::default();
    let report = run(&project, &options, &mut mock);
    let dimension = &report.files[0].dimensions["file_organization"];
    assert_eq!((dimension.units.too_small, mock.calls), (1, 0));
    // 360 member lines are weighed.
    let (project, options) = organized("src/lib.bend", &defs(60));
    let (_, plan) = planned(&project, &options);
    assert_eq!(stages(&plan), ["outline"]);
}
