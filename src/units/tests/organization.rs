//! File organization: outlines of application and test files.
use super::*;

fn two_concerns() -> String {
    let mut source = String::from("struct Cache { entries: Vec<u8> }\n");
    for i in 0..7 {
        source.push_str(&function(&format!("warm{i}")));
    }
    source.push_str("struct Page { body: String }\n");
    for i in 0..7 {
        source.push_str(&function(&format!("render{i}")));
    }
    source
}

#[test]
fn file_organization_review_without_a_module_choice_is_a_file_wide_finding() {
    let project = Project::new();
    project.write("lib.rs", &two_concerns());
    let mut options = args();
    only(&mut options, catalog::FILE_ORGANIZATION);
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
    let project = Project::new();
    project.write("lib.rs", &two_concerns());
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
    let mut options = args();
    only(&mut options, catalog::FILE_ORGANIZATION);
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
fn outlines_carry_member_and_file_sizes() {
    let (project, options) = rule_project(&two_concerns(), catalog::FILE_ORGANIZATION);
    let mut mock = Mock::default();
    run(&project, &options, &mut mock);
    let state = &mock.requests[0]["state"];
    assert_eq!(state["file"]["lines"], 16 + 14 * 7);
    assert_eq!(state["members"][1]["lines"], 8);
}

/// Two suites of cases, each calling its own subject.
fn two_suites() -> String {
    let mut source =
        String::from("import { parse } from './parse';\nimport { render } from './render';\n\n");
    for subject in ["parse", "render"] {
        source.push_str(&format!("describe('{subject}', () => {{\n"));
        for i in 0..6 {
            source.push_str(&format!(
                "  it('case {i}', () => {{\n    const value = {subject}({{ id: {i} }});\n    expect(value.id).toBe({i});\n    expect(value).toBeDefined();\n    expect(value).not.toBeNull();\n    expect(typeof value).toBe('object');\n    expect(Object.keys(value)).toContain('id');\n    expect(value).toEqual({{ id: {i} }});\n  }});\n"
            ));
        }
        source.push_str("});\n");
    }
    source
}

#[test]
fn test_files_are_outlined_by_suite_without_include_tests() {
    let project = Project::new();
    project.write("tests/app.test.ts", &two_suites());
    let mut options = args();
    only(&mut options, catalog::FILE_ORGANIZATION);
    let mut eval = scripted(2);
    let report = run(&project, &options, &mut eval);
    let file = &report.files[0];
    assert_eq!(file.classification.as_ref().unwrap().kind, "tests");
    assert_eq!(
        file.status,
        Status::Consider,
        "test layout is one level lower"
    );
    let finding = &file.findings[0];
    assert!(
        finding.message.contains("separate test file"),
        "{}",
        finding.message
    );
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
fn short_files_are_too_small_to_split_and_never_clear() {
    let project = Project::new();
    project.write(
        "lib.rs",
        &format!("{}{}", function("warm"), function("render")),
    );
    let mut options = args();
    only(&mut options, catalog::FILE_ORGANIZATION);
    let mut mock = Mock::default();
    let report = run(&project, &options, &mut mock);
    let dimension = &report.files[0].dimensions["file_organization"];
    assert_eq!((dimension.units.too_small, mock.calls), (1, 0));
    assert_eq!(dimension.status, Status::NotApplicable);
}
