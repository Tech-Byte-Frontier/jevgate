//! Test value and redundancy, and the test recheck with subjects and setup.
use super::*;

const TESTS: &str = "fn total(values: &[i32]) -> i32 {\n    values.iter().sum()\n}\n\n#[cfg(test)]\nmod tests {\n    use super::*;\n\n    #[test]\n    fn adds_two() {\n        let values = vec![1, 2];\n        assert_eq!(total(&values), 3);\n    }\n\n    #[test]\n    fn adds_three() {\n        let values = vec![1, 2, 3];\n        assert_eq!(total(&values), 6);\n    }\n\n    #[test]\n    fn adds_four() {\n        let values = vec![1, 2, 3, 4];\n        assert_eq!(total(&values), 10);\n    }\n}\n";

#[test]
fn test_rules_need_include_tests_and_summarize_over_tested_subjects() {
    let project = Project::new();
    project.write("lib.rs", TESTS);
    let options = args();
    let (_, plan) = planned(&project, &options);
    assert!(!plan.files[&0].rules.contains_key(catalog::TEST_VALUE));
    let mut options = args();
    options.include_tests = true;
    options.rules = vec![catalog::TEST_VALUE.into(), catalog::TEST_REDUNDANCY.into()];
    let report = run(&project, &options, &mut scripted(1));
    let file = &report.files[0];
    let values = &file.dimensions["test_value"];
    assert_eq!(values.units.judged, 3);
    assert_eq!(
        values.status,
        Status::Uncertain,
        "Noul answers at 0.5 stay uncertain"
    );
    let redundancy = &file.dimensions["test_redundancy"];
    assert_eq!(redundancy.units.judged, 3);
    assert_eq!(redundancy.status, Status::Consider);
    let over = file
        .findings
        .iter()
        .find(|f| f.message.contains("3 tests of `total` overlap"))
        .unwrap();
    assert_eq!(over.strength, Strength::Consider);
    assert_eq!(over.locations.len(), 3);
}

#[test]
fn undecided_weak_test_signals_do_not_block_a_clear_test() {
    let project = Project::new();
    project.write("lib.rs", TESTS);
    let mut options = args();
    options.include_tests = true;
    options.rules = vec![catalog::TEST_VALUE.into()];
    let mut eval = scripted(0);
    eval.overrides
        .push(("internal", json!({"type":"noul","noul":0.45})));
    eval.overrides
        .push(("several", json!({"type":"noul","noul":0.4})));
    let report = run(&project, &options, &mut eval);
    assert_eq!(
        report.files[0].dimensions["test_value"].status,
        Status::Clear
    );
    options.refresh = true;
    let mut hollow = scripted(0);
    hollow
        .overrides
        .push(("own_logic", json!({"type":"noul","noul":0.5})));
    hollow
        .recheck_overrides
        .push(("own_logic", json!({"type":"noul","noul":0.5})));
    let report = run(&project, &options, &mut hollow);
    assert!(hollow.stages.contains(&"recheck".to_string()));
    assert_eq!(
        report.files[0].dimensions["test_value"].status,
        Status::Uncertain
    );
}

const VITEST: &str = "import { vi } from 'vitest'\nimport { repo } from './repo'\nimport { getProfile } from './profile'\nvi.mock('./repo')\n\ndescribe('profile', () => {\n  beforeEach(() => {\n    repo.findProfile.mockReset()\n  })\n\n  it('returns the stored profile', async () => {\n    repo.findProfile.mockResolvedValue({ id: 'u1', name: 'Ana' })\n    const result = await getProfile('u1')\n    expect(result).toEqual({ id: 'u1', name: 'Ana' })\n  })\n})\n";
const PROFILE: &str = "export async function getProfile(id: string) {\n  if (!id) {\n    throw new Error('missing id')\n  }\n  const profile = await repo.findProfile(id)\n  return profile\n}\n";

#[test]
fn an_undecided_test_is_asked_again_with_its_subjects_and_setup() {
    let project = Project::new();
    project.write("src/profile.ts", PROFILE);
    project.write("src/profile.test.ts", VITEST);
    let mut options = args();
    options.include_tests = true;
    only(&mut options, catalog::TEST_VALUE);
    let (_, plan) = planned(&project, &options);
    let file = plan
        .files
        .values()
        .find(|f| f.path.ends_with("profile.test.ts"))
        .unwrap();
    let (request, _) = file.units[0].recheck.as_ref().expect("a recheck");
    let state = &request["state"];
    assert!(
        state["subjects"][0]["source"]
            .as_str()
            .unwrap()
            .contains("repo.findProfile(id)")
    );
    let setup = state["setup"].as_str().unwrap();
    assert!(
        setup.starts_with("import { vi } from 'vitest'") && setup.contains("vi.mock('./repo')"),
        "{setup}"
    );
    assert!(setup.contains("beforeEach(() => {\n    repo.findProfile.mockReset()\n  })"));
    assert!(!setup.contains("describe("), "{setup}");
    assert_eq!(
        request["jevgate"]["sources"].as_array().unwrap().len(),
        2,
        "the subject's file is checked for freshness"
    );
    let first = plan
        .requests
        .iter()
        .find(|p| p.request["jevgate"]["stage"] == "tests")
        .unwrap();
    assert!(
        first.request["state"]["subjects"][0]["source"].is_null(),
        "the first pass sends signatures only"
    );
    let test_value = |report: &Report| {
        report
            .files
            .iter()
            .find(|f| f.path.ends_with("profile.test.ts"))
            .unwrap()
            .clone()
    };
    let mut eval = scripted(0);
    eval.overrides = vec![("own_logic", noul_at(0.5))];
    let report = run(&project, &options, &mut eval);
    assert!(eval.stages.contains(&"recheck".to_string()));
    assert_eq!(
        test_value(&report).dimensions["test_value"].status,
        Status::Clear
    );
    options.refresh = true;
    let mut eval = scripted(0);
    eval.overrides = vec![("own_logic", noul_at(0.5))];
    eval.recheck_overrides = vec![("mock_only", noul_at(0.95))];
    let file = test_value(&run(&project, &options, &mut eval));
    assert_eq!(file.findings[0].strength, Strength::Review);
    assert!(
        file.findings[0]
            .message
            .contains("only checks values its mocks")
    );
}

#[test]
fn a_test_files_setup_is_its_head_and_hooks_and_long_parts_are_left_out() {
    let python = "import pytest\nfrom app import total\n\nclass TotalTest(TestCase):\n    def setUp(self):\n        self.rows = [1, 2]\n\n    def test_total(self):\n        self.assertEqual(total(self.rows), 3)\n";
    let setup = test_units::file_setup(python, 1, 8);
    assert_eq!(
        setup,
        "import pytest\nfrom app import total\n\ndef setUp(self):\n        self.rows = [1, 2]"
    );
    let long = format!("{}it('x', () => {{}})\n", "// padding\n".repeat(500));
    assert_eq!(test_units::file_setup(&long, 1, 501), "");
}
