//! Test value and redundancy, and the test recheck with subjects and setup.
use super::*;

const TESTS: &str = "fn total(values: &[i32]) -> i32 {\n    values.iter().sum()\n}\n\n#[cfg(test)]\nmod tests {\n    use super::*;\n\n    #[test]\n    fn adds_two() {\n        let values = vec![1, 2];\n        assert_eq!(total(&values), 3);\n    }\n\n    #[test]\n    fn adds_three() {\n        let values = vec![1, 2, 3];\n        assert_eq!(total(&values), 6);\n    }\n\n    #[test]\n    fn adds_four() {\n        let values = vec![1, 2, 3, 4];\n        assert_eq!(total(&values), 10);\n    }\n}\n";

#[test]
fn test_rules_need_include_tests_and_summarize_over_tested_subjects() {
    let project = Project::new();
    project.write("lib.rs", TESTS);
    project.write(
        "Cargo.toml",
        "[package]\nname = \"cases\"\n\n[dev-dependencies]\nrstest = \"0.26\"\n",
    );
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
    let redundancy: Vec<&str> = file
        .findings
        .iter()
        .filter(|f| f.rule == catalog::id(catalog::TEST_REDUNDANCY))
        .map(|f| f.message.as_str())
        .collect();
    assert_eq!(
        redundancy.len(),
        1,
        "the group reports its pairs: {redundancy:?}"
    );
}

#[test]
fn an_undecided_test_pair_is_asked_again_with_the_body_of_its_subject() {
    let (project, options) = tests_project(&[("lib.rs", TESTS)], catalog::TEST_REDUNDANCY);
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
    let first = first_request(&plan, "test-pair");
    assert!(first["state"]["subject"]["source"].is_null());
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
    let (project, mut options) = tests_project(&[("lib.rs", TESTS)], catalog::TEST_VALUE);
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
    let (project, mut options) = tests_project(
        &[("src/profile.ts", PROFILE), ("src/profile.test.ts", VITEST)],
        catalog::TEST_VALUE,
    );
    let (_, plan) = planned(&project, &options);
    let file = file_plan(&plan, "profile.test.ts");
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
    let first = first_request(&plan, "tests");
    assert!(
        first["state"]["subjects"][0]["source"].is_null(),
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
/// An RSpec file of `Invoice` and the class it tests.
const INVOICE: &[(&str, &str)] = &[
    ("lib/invoice.rb", INVOICE_RB),
    ("spec/invoice_spec.rb", INVOICE_SPEC),
];
const INVOICE_SPEC: &str = "require 'spec_helper'\n\nRSpec.describe Invoice do\n  def build_rows(count)\n    Array.new(count) { 1 }\n  end\n\n  let(:rows) { build_rows(2) }\n  let(:unused) { build_rows(9) }\n  subject { Invoice.new(rows) }\n\n  it \"doubles each row\" do\n    expect(subject.total).to eq 4\n  end\n\n  it \"doubles each row again\" do\n    expect(subject.total).to eq 4\n  end\nend\n";

#[test]
fn a_ruby_test_is_rechecked_with_its_groups_the_setup_it_reads_and_its_helpers() {
    let (project, options) = tests_project(INVOICE, catalog::TEST_VALUE);
    let (_, plan) = planned(&project, &options);
    let file = file_plan(&plan, "invoice_spec.rb");
    let first = first_request(&plan, "tests");
    assert_eq!(first["state"]["tests"][0]["suite"], "Invoice");
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
    let (project, mut options) = tests_project(INVOICE, catalog::TEST_REDUNDANCY);
    let (_, plan) = planned(&project, &options);
    let pair = first_request(&plan, "test-pair");
    assert!(pair["questions"]["distinct"].is_object());
    let note = pair["questions"]["overlap"]["instructions"]["note"]
        .as_str()
        .unwrap();
    assert!(note.contains("an alias and its original"), "{note}");
    let mut strength = |distinct: f64, refresh: bool| {
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
        let (project, options) = tests_project(&[("lib.rs", source)], catalog::TEST_REDUNDANCY);
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

#[test]
fn a_redundant_pair_is_a_review_only_when_both_tests_share_input_and_outcome() {
    let (project, mut options) = tests_project(&[("lib.rs", TESTS)], catalog::TEST_REDUNDANCY);
    let strengths = |options: &CheckArgs, same_input: f64, same_outcome: f64| {
        let mut eval = scripted(2);
        eval.overrides = vec![
            ("overlap", spread(0.0, 0.05, 0.95)),
            ("same_input", noul_at(same_input)),
            ("same_outcome", noul_at(same_outcome)),
        ];
        let report = run(&project, options, &mut eval);
        report.files[0]
            .findings
            .iter()
            .filter(|f| f.rule == catalog::id(catalog::TEST_REDUNDANCY))
            .map(|f| f.strength)
            .max()
    };
    assert_eq!(strengths(&options, 0.95, 0.95), Some(Strength::Review));
    options.refresh = true;
    assert_eq!(strengths(&options, 0.69, 0.95), Some(Strength::Consider));
    assert_eq!(strengths(&options, 0.95, 0.05), Some(Strength::Consider));
    // Tests that read the same apart from their names stay a review.
    let twins = "fn total(values: &[i32]) -> i32 {\n    values.iter().sum()\n}\n\n#[cfg(test)]\nmod tests {\n    use super::*;\n\n    #[test]\n    fn when_empty() {\n        let values = vec![1, 2];\n        assert_eq!(total(&values), 3);\n    }\n\n    #[test]\n    fn when_full() {\n        let values = vec![1, 2];\n        assert_eq!(total(&values), 3);\n    }\n}\n";
    let (project, options) = tests_project(&[("lib.rs", twins)], catalog::TEST_REDUNDANCY);
    let mut eval = scripted(2);
    eval.overrides = vec![
        ("overlap", spread(0.0, 0.05, 0.95)),
        ("same_input", noul_at(0.3)),
        ("same_outcome", noul_at(0.95)),
    ];
    let report = run(&project, &options, &mut eval);
    assert_eq!(report.files[0].findings[0].strength, Strength::Review);
    // A space inside a string can be what the tests differ in.
    let spaced = twins.replacen("vec![1, 2]", "vec![1,2]", 1);
    let (project, options) = tests_project(&[("lib.rs", &spaced)], catalog::TEST_REDUNDANCY);
    let report = run(&project, &options, &mut eval);
    assert_eq!(report.files[0].findings[0].strength, Strength::Consider);
    // Without a crate for parameterized tests, merging them is only a note.
    let (project, options) = project_with(&[("lib.rs", &spaced)], &[catalog::TEST_REDUNDANCY]);
    let mut options = options;
    options.include_tests = true;
    let report = run(&project, &options, &mut eval);
    assert_eq!(report.files[0].findings[0].strength, Strength::Note);
}

#[test]
fn copies_inside_tests_a_redundancy_finding_names_are_reported_once() {
    let case = |name: &str, text: &str, sum: i32| {
        format!(
            "    #[test]\n    fn {name}() {{\n        let text = \"{text}\";\n        let parts: Vec<&str> = text.split(',').map(|v| v.trim()).collect();\n        let values: Vec<i32> = parts.iter().map(|v| v.parse().unwrap()).collect();\n        let count = values.len();\n        let sum = total(&values);\n        assert_eq!(count, 2, \"both parsed values are kept\");\n        assert_eq!(sum, {sum}, \"the total of the parsed values\");\n    }}\n"
        )
    };
    let source = format!(
        "fn total(values: &[i32]) -> i32 {{\n    values.iter().sum()\n}}\n\n#[cfg(test)]\nmod tests {{\n    use super::*;\n\n{}\n{}}}\n",
        case("totals_small", "1, 2", 3),
        case("totals_large", "40, 50", 90)
    );
    let (project, mut options) = tests_project(&[("lib.rs", &source)], catalog::TEST_REDUNDANCY);
    options.rules.push(catalog::SHARED_LOGIC.into());
    let mut eval = scripted(2);
    eval.overrides = vec![
        ("overlap", spread(0.0, 0.9, 0.1)),
        ("required", noul_at(0.05)),
    ];
    let report = run(&project, &options, &mut eval);
    let rules: Vec<&str> = report.files[0]
        .findings
        .iter()
        .filter(|f| f.strength != Strength::Note)
        .map(|f| f.rule.as_str())
        .collect();
    assert_eq!(rules, [catalog::id(catalog::TEST_REDUNDANCY)], "{rules:?}");
}
