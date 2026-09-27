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
    let request = plan.files.values().next().unwrap().units[0]
        .recheck
        .as_ref()
        .unwrap()
        .request();
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
    let kind = |choice: &str| {
        let mut probabilities: serde_json::Map<String, Value> =
            questions::outline_kind(false)["criteria"]
                .as_object()
                .unwrap()
                .keys()
                .map(|k| (k.clone(), json!(0.0)))
                .collect();
        probabilities.insert(choice.into(), json!(0.85));
        probabilities.insert("algorithm".into(), json!(0.15));
        let mut eval = scripted(3);
        eval.recheck_overrides = vec![(
            "kind",
            json!({"type":"choice","choice":choice,"confidence":0.8,"probabilities":probabilities}),
        )];
        run(&project, &options, &mut eval)
    };
    assert_eq!(
        kind("per_feature").files[0].dimensions["file_organization"].status,
        Status::Clear,
        "the same kind of code written out per feature is one job"
    );
    let finding = &kind("several").files[0].findings[0];
    assert_eq!(finding.strength, Strength::Consider);
    assert!(
        finding.message.contains("several unrelated features"),
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
        kind: Some(kind), ..
    } = &unit.detail
    else {
        panic!("the kind is asked from the outline");
    };
    assert!(kind.request()["state"]["file"]["source"].is_null());
}

#[test]
fn a_request_answered_once_fits_whatever_the_calibration() {
    // About 79 KB: the recheck sends it whole, which the estimate fits at the
    // default 3.0 bytes per token and not at 2.0.
    let padding = format!("// {}\n", "x".repeat(100)).repeat(700);
    let (project, options) = organized("lib.rs", &format!("{}{padding}", two_concerns()));
    let first = run(&project, &options, &mut scripted(3));
    assert_eq!(first.stages["recheck"].successful_requests, 1);
    let context = project.context();
    let (inputs, mut report) = crate::tests::snapshot(&project, &options);
    let store = crate::storage::Store::open(&project.0).unwrap();
    let mut mock = Mock::default();
    let mut session = crate::tests::session(&options, &context, &store, &mut mock);
    session.budget = TokenBudget {
        bytes_per_token: 2.0,
    };
    session.evaluate(&inputs, &mut report).unwrap();
    assert_eq!(mock.calls, 0, "every request comes from the cache");
    assert_eq!(report.stages["recheck"].cache_hits, 1);
    let status = |report: &Report| {
        report.files[0].dimensions["file_organization"]
            .status
            .clone()
    };
    assert_eq!(status(&report), status(&first));
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
    let request = plan.files.values().next().unwrap().units[0]
        .recheck
        .as_ref()
        .unwrap()
        .request();
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

#[test]
fn a_bend_file_s_split_is_at_most_a_consider_and_a_note_in_titled_sections() {
    let defs = |sectioned: bool| -> String {
        let mut source = String::from("import Base\n\n");
        for i in 0..60 {
            if sectioned && i % 20 == 0 {
                source.push_str(&format!("# ---- part {i} ----\n\n"));
            }
            source.push_str(&format!(
                "def f{i}(x: Nat) -> Nat:\n  match x:\n    case 0n:\n      1n\n    case 1n+p:\n      f{i}(p)\n\n"
            ));
        }
        source
    };
    let strength = |sectioned: bool| {
        let (project, options) = organized("src/lib.bend", &defs(sectioned));
        let report = run(&project, &options, &mut scripted(2));
        report.files[0].dimensions["file_organization"]
            .status
            .clone()
    };
    assert_eq!(strength(false), Status::Consider);
    assert_eq!(strength(true), Status::Note);
}

/// A parser and a renderer of ten functions each, every function naming its
/// part's two types: two parts of over two hundred lines, 470 in all.
fn two_parts() -> String {
    parts_of_steps(20)
}

/// `two_parts` with `steps` statements in each function.
fn parts_of_steps(steps: usize) -> String {
    let mut source = String::new();
    for (part, state, item) in [
        ("parse", "Parser", "Token"),
        ("render", "Renderer", "Glyph"),
    ] {
        source.push_str(&format!(
            "struct {state} {{ at: usize }}\nstruct {item} {{ size: u8 }}\n"
        ));
        for i in 0..10 {
            source.push_str(&format!(
                "fn {part}_{i}(state: &{state}, item: &{item}) -> usize {{\n"
            ));
            for step in 0..steps {
                source.push_str(&format!(
                    "    let v{step} = state.at + item.size as usize + {step};\n"
                ));
            }
            source.push_str(&format!("    v{}\n}}\n", steps - 1));
        }
    }
    source
}

/// Answers that clear the outline and find each part a job of its own.
fn parts_of_their_own(own: f64, own_job: f64) -> Scripted {
    let mut eval = scripted(0);
    let chosen = if own_job >= 0.5 {
        "own_job"
    } else {
        "same_kind"
    };
    let role = json!({"type": "choice", "choice": chosen, "confidence": 0.5,
        "probabilities": {"own_job": own_job, "same_kind": 1.0 - own_job, "support": 0.0, "core": 0.0}});
    eval.overrides = vec![
        ("own", json!({"type": "noul", "noul": own})),
        ("role", role),
    ];
    eval
}

#[test]
fn a_long_file_s_parts_are_planned_with_their_source_and_the_rest_by_signature() {
    let (project, options) = organized("src/lib.rs", &two_parts());
    let (_, plan) = planned(&project, &options);
    let Detail::Outline { parts, .. } = &plan.files.values().next().unwrap().units[0].detail else {
        panic!("an outline");
    };
    let names: Vec<Vec<&str>> = parts
        .iter()
        .map(|p| p.names.iter().map(String::as_str).take(3).collect())
        .collect();
    assert_eq!(
        names,
        [
            ["Parser", "Token", "parse_0"],
            ["Renderer", "Glyph", "render_0"]
        ]
    );
    let request = parts[0].follow_up.request();
    let source = request["state"]["part"]["source"].as_str().unwrap();
    assert!(source.contains("fn parse_9(") && !source.contains("fn render_0("));
    assert_eq!(request["state"]["rest"][0]["name"], "Renderer");
    assert!(
        request["state"]["rest"][2]["signature"]
            .as_str()
            .unwrap()
            .contains("render_0")
    );
    for (path, source) in [
        ("scripts/lib.rs", two_parts()),
        // 370 lines, short enough to read whole.
        ("src/short.rs", parts_of_steps(15)),
    ] {
        let (project, options) = organized(path, &source);
        let (_, plan) = planned(&project, &options);
        let Detail::Outline { parts, .. } = &plan.files.values().next().unwrap().units[0].detail
        else {
            panic!("an outline");
        };
        assert!(parts.is_empty(), "{path} is not asked about its parts");
    }
}

#[test]
fn a_part_of_a_long_file_that_does_a_job_of_its_own_is_a_consider_naming_it() {
    let (project, mut options) = organized("src/lib.rs", &two_parts());
    let report = run(&project, &options, &mut parts_of_their_own(0.8, 0.7));
    assert_eq!(report.stages["parts"].successful_requests, 2);
    let finding = &report.files[0].findings[0];
    assert_eq!(finding.strength, Strength::Consider);
    assert!(
        finding.message.starts_with("`Parser`, `Token`, `parse_0`")
            && finding.message.contains("do a job of their own"),
        "{}",
        finding.message
    );
    assert_eq!(finding.symbol.as_deref(), Some("Parser"));
    assert_eq!(finding.locations.len(), 12);
    options.refresh = true;
    for (own, own_job) in [(0.6, 0.9), (0.9, 0.4)] {
        let report = run(&project, &options, &mut parts_of_their_own(own, own_job));
        assert_eq!(
            report.files[0].dimensions["file_organization"].status,
            Status::Clear,
            "own {own}, own job {own_job}"
        );
    }
}

#[test]
fn an_outline_with_a_finding_is_not_asked_about_its_parts() {
    let (project, options) = organized("src/lib.rs", &two_parts());
    let mut eval = parts_of_their_own(0.9, 0.9);
    eval.level = 2;
    let report = run(&project, &options, &mut eval);
    assert!(!report.stages.contains_key("parts"));
    assert!(
        report.files[0].findings[0]
            .message
            .contains("several features")
    );
}
