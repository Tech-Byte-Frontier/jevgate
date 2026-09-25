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
fn an_undecided_test_pair_is_asked_again_with_the_body_of_its_subject() {
    let project = Project::new();
    project.write("lib.rs", TESTS);
    let mut options = args();
    options.include_tests = true;
    only(&mut options, catalog::TEST_REDUNDANCY);
    let (_, plan) = planned(&project, &options);
    let pair = plan.files[&0]
        .units
        .iter()
        .find(|u| u.rule == catalog::TEST_REDUNDANCY)
        .unwrap();
    let (request, _) = pair.recheck.as_ref().expect("a recheck");
    assert_eq!(request["jevgate"]["stage"], "recheck");
    assert!(
        request["state"]["subject"]["source"]
            .as_str()
            .unwrap()
            .contains("values.iter().sum()")
    );
    let first = plan
        .requests
        .iter()
        .find(|p| p.request["jevgate"]["stage"] == "test-pair")
        .unwrap();
    assert!(first.request["state"]["subject"]["source"].is_null());
    // An even spread over the three levels is settled by the recheck.
    let mut eval = scripted(3);
    eval.recheck_level = Some(1);
    let report = run(&project, &options, &mut eval);
    assert!(eval.stages.contains(&"recheck".to_string()));
    assert_eq!(
        report.files[0].dimensions["test_redundancy"].status,
        Status::Consider
    );
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

const INVOICE_RB: &str = "class Invoice\n  def initialize(rows)\n    @rows = rows\n  end\n\n  def total\n    @rows.sum { |row| row * 2 }\n  end\nend\n";
const INVOICE_SPEC: &str = "require 'spec_helper'\n\nRSpec.describe Invoice do\n  def build_rows(count)\n    Array.new(count) { 1 }\n  end\n\n  let(:rows) { build_rows(2) }\n  let(:unused) { build_rows(9) }\n  subject { Invoice.new(rows) }\n\n  it \"doubles each row\" do\n    expect(subject.total).to eq 4\n  end\n\n  it \"doubles each row again\" do\n    expect(subject.total).to eq 4\n  end\nend\n";

#[test]
fn a_ruby_test_is_rechecked_with_its_groups_the_setup_it_reads_and_its_helpers() {
    let project = Project::new();
    project.write("lib/invoice.rb", INVOICE_RB);
    project.write("spec/invoice_spec.rb", INVOICE_SPEC);
    let mut options = args();
    options.include_tests = true;
    only(&mut options, catalog::TEST_VALUE);
    let (_, plan) = planned(&project, &options);
    let file = plan
        .files
        .values()
        .find(|f| f.path.ends_with("invoice_spec.rb"))
        .unwrap();
    let first = plan
        .requests
        .iter()
        .find(|p| p.request["jevgate"]["stage"] == "tests")
        .unwrap();
    assert_eq!(first.request["state"]["tests"][0]["suite"], "Invoice");
    let (request, _) = file.units[0].recheck.as_ref().expect("a recheck");
    let setup = request["state"]["setup"].as_str().unwrap();
    assert_eq!(
        setup,
        "require 'spec_helper'\n\nlet(:rows) { build_rows(2) }\n\nsubject { Invoice.new(rows) }\n\ndef build_rows(count)\n    Array.new(count) { 1 }\n  end",
        "the `let` the test never reads is left out"
    );
    assert!(
        request["state"]["subjects"][0]["source"]
            .as_str()
            .unwrap()
            .contains("@rows.sum")
    );
    let note = request["questions"]["mock_only"]["instructions"]["note"]
        .as_str()
        .unwrap();
    assert!(
        note.contains("`let` and `subject` definitions it reads"),
        "{note}"
    );
}

#[test]
fn a_ruby_helper_is_found_in_the_tests_file_or_the_nearest_support_file() {
    let helper = |path: &str| test_units::SubjectSource {
        path: path.into(),
        source: format!("def mock_app # {path}"),
        shared: !path.ends_with("_test.rb"),
    };
    let found = |paths: &[&str]| {
        let defined: Vec<_> = paths.iter().map(|p| helper(p)).collect();
        test_units::nearest(std::path::Path::new("test/routing_test.rb"), &defined)
            .map(|h| h.path.to_str().unwrap().to_string())
    };
    let support = "test/test_helper.rb";
    let other_tree = "rack-protection/spec/support/spec_helpers.rb";
    let other_case_file = "test/json_test.rb";
    assert_eq!(
        found(&[other_tree, other_case_file, support]).as_deref(),
        Some(support)
    );
    assert_eq!(
        found(&[support, "test/routing_test.rb"]).as_deref(),
        Some("test/routing_test.rb")
    );
    assert_eq!(found(&[other_tree, other_case_file]), None);
    assert_eq!(found(&[support, "test/app_helper.rb"]), None);
}

#[test]
fn ruby_pairs_are_a_review_only_when_neither_test_checks_something_the_other_does_not() {
    let project = Project::new();
    project.write("lib/invoice.rb", INVOICE_RB);
    project.write("spec/invoice_spec.rb", INVOICE_SPEC);
    let mut options = args();
    options.include_tests = true;
    only(&mut options, catalog::TEST_REDUNDANCY);
    let (_, plan) = planned(&project, &options);
    let pair = plan
        .requests
        .iter()
        .find(|p| p.request["jevgate"]["stage"] == "test-pair")
        .unwrap();
    assert!(pair.request["questions"]["distinct"].is_object());
    let note = pair.request["questions"]["overlap"]["instructions"]["note"]
        .as_str()
        .unwrap();
    assert!(note.contains("an alias and its original"), "{note}");
    let strength = |distinct: f64, refresh: bool| {
        let mut options = args();
        options.include_tests = true;
        only(&mut options, catalog::TEST_REDUNDANCY);
        options.refresh = refresh;
        let mut eval = scripted(2);
        eval.overrides = vec![("distinct", noul_at(distinct))];
        let report = run(&project, &options, &mut eval);
        let file = report
            .files
            .iter()
            .find(|f| f.path.ends_with("invoice_spec.rb"))
            .unwrap()
            .clone();
        file.findings[0].strength
    };
    assert_eq!(strength(0.05, false), Strength::Review);
    assert_eq!(strength(0.3, true), Strength::Consider);
}

/// Two pairs of tests of `total`, each alike within and unlike the other.
const TWO_PAIRS: &str = "fn total(values: &[i32]) -> i32 {\n    values.iter().sum()\n}\n\n#[cfg(test)]\nmod tests {\n    use super::*;\n\n    #[test]\n    fn adds_two() {\n        let values = vec![1, 2];\n        assert_eq!(total(&values), 3);\n    }\n\n    #[test]\n    fn adds_three() {\n        let values = vec![1, 2, 3];\n        assert_eq!(total(&values), 6);\n    }\n\n    #[test]\n    fn empty_is_zero() {\n        let empty: Vec<i32> = Vec::new();\n        let sum = total(&empty);\n        assert!(sum == 0, \"an empty list sums to zero\");\n    }\n\n    #[test]\n    fn empty_slice_is_zero() {\n        let empty: Vec<i32> = Vec::with_capacity(8);\n        let sum = total(&empty);\n        assert!(sum == 0, \"an empty slice sums to zero\");\n    }\n}\n";

#[test]
fn overlapping_tests_are_grouped_only_when_their_pairs_connect_them() {
    let redundancy = |source: &str| {
        let project = Project::new();
        project.write("lib.rs", source);
        let mut options = args();
        options.include_tests = true;
        only(&mut options, catalog::TEST_REDUNDANCY);
        let report = run(&project, &options, &mut scripted(1));
        report.files[0]
            .findings
            .iter()
            .map(|f| f.message.clone())
            .collect::<Vec<_>>()
    };
    let disjoint = redundancy(TWO_PAIRS);
    assert_eq!(disjoint.len(), 2, "two pairs and no group: {disjoint:?}");
    assert!(
        disjoint
            .iter()
            .all(|m| !m.contains("tests of `total` overlap"))
    );
    let chained = redundancy(TESTS);
    assert!(
        chained
            .iter()
            .any(|m| m.contains("3 tests of `total` overlap")),
        "{chained:?}"
    );
}

#[test]
fn a_java_test_classs_setup_holds_its_mocks_and_setup_method() {
    let java = "package app;\n\nimport org.junit.jupiter.api.BeforeEach;\n\n@ExtendWith(MockitoExtension.class)\nclass FormatterTests {\n\n\t@Mock\n\tprivate TypeRepository types;\n\n\t@BeforeEach\n\tvoid setup() {\n\t\tthis.formatter = new Formatter(types);\n\t}\n\n\t@Test\n\tvoid parses() {\n\t\tassertThat(formatter.parse(\"Bird\")).isNotNull();\n\t}\n}\n";
    assert_eq!(
        test_units::file_setup(java, 1, 16),
        "package app;\n\nimport org.junit.jupiter.api.BeforeEach;\n\n@ExtendWith(MockitoExtension.class)\nclass FormatterTests {\n\n\t@Mock\n\tprivate TypeRepository types;\n\n@BeforeEach\n\tvoid setup() {\n\t\tthis.formatter = new Formatter(types);\n\t}"
    );
}

#[test]
fn a_mockmvc_test_is_rechecked_with_the_controller_method_its_request_reaches() {
    let project = Project::new();
    for (controller, entity) in [("OwnerController", "owners"), ("VetController", "vets")] {
        project.write(
            &format!("src/main/java/app/{controller}.java"),
            &format!("package app;\n\n@Controller\n@RequestMapping(\"/{entity}\")\nclass {controller} {{\n\t@GetMapping(\"/{{id}}\")\n\tpublic String show(@PathVariable int id, Model model) {{\n\t\tmodel.addAttribute(\"{entity}\", this.repository.findById(id));\n\t\treturn \"{entity}/details\";\n\t}}\n\n\t@PostMapping(\"/{{id}}\")\n\tpublic String update(@PathVariable int id) {{\n\t\treturn \"redirect:/{entity}\";\n\t}}\n}}\n"),
        );
    }
    project.write(
        "src/test/java/app/OwnerControllerTests.java",
        "package app;\n\n@WebMvcTest(OwnerController.class)\nclass OwnerControllerTests {\n\t@Test\n\tvoid showsOwner() throws Exception {\n\t\tmockMvc.perform(get(\"/owners/{id}\", 1))\n\t\t\t.andExpect(status().isOk())\n\t\t\t.andExpect(model().attributeExists(\"owners\"));\n\t}\n}\n",
    );
    let mut options = args();
    options.include_tests = true;
    options.rules = vec![catalog::TEST_VALUE.into()];
    let (_, plan) = planned(&project, &options);
    let recheck = plan
        .files
        .values()
        .flat_map(|f| &f.units)
        .find(|u| u.name == "showsOwner")
        .and_then(|u| u.recheck.as_ref())
        .map(|(request, _)| request["state"]["subjects"].clone())
        .unwrap();
    let subjects = recheck.as_array().unwrap();
    assert_eq!(subjects.len(), 1, "{subjects:?}");
    assert_eq!(subjects[0]["name"], "OwnerController::show");
    assert_eq!(subjects[0]["route"], "GET /owners/{id}");
    assert!(
        subjects[0]["source"]
            .as_str()
            .unwrap()
            .contains("return \"owners/details\"")
    );
}
