use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Serialize)]
pub struct Rule {
    pub id: &'static str,
    pub key: &'static str,
    pub version: &'static str,
    pub scope: &'static str,
    pub unit: &'static str,
    pub inspection: &'static str,
    pub acceptable_example: &'static str,
    pub requires_tests: bool,
    pub evaluation_dataset: &'static str,
    pub thresholds_validated: bool,
}

pub const FILE_ORGANIZATION: &str = "file_organization";
pub const FUNCTION_SIMPLIFICATION: &str = "function_simplification";
pub const SHARED_LOGIC: &str = "shared_logic";
pub const TEST_VALUE: &str = "test_value";
pub const TEST_REDUNDANCY: &str = "test_redundancy";

const DATASET: &str = "focused development set; not calibrated";

pub fn rules() -> Vec<Rule> {
    vec![
        Rule {
            id: "maintainability/file-organization",
            key: FILE_ORGANIZATION,
            version: rule_version(FILE_ORGANIZATION),
            scope: "application files with two or more members",
            unit: "file outline: member signatures and groups, no bodies",
            inspection: "Do the file's members serve one purpose, or groups with separate purposes that could be their own modules?",
            acceptable_example: "Steps and helpers of one job kept together",
            requires_tests: false,
            evaluation_dataset: DATASET,
            thresholds_validated: false,
        },
        Rule {
            id: "maintainability/function-simplification",
            key: FUNCTION_SIMPLIFICATION,
            version: rule_version(FUNCTION_SIMPLIFICATION),
            scope: "functions and methods with bodies of five or more lines",
            unit: "one function's source",
            inspection: "Does the function perform two or more substantial tasks, or could its nesting or branching be flattened?",
            acceptable_example: "One task, with every step serving it",
            requires_tests: false,
            evaluation_dataset: DATASET,
            thresholds_validated: false,
        },
        Rule {
            id: "maintainability/shared-logic",
            key: SHARED_LOGIC,
            version: rule_version(SHARED_LOGIC),
            scope: "renamed or exact copies of two or more statements across selected files and explicit context",
            unit: "one candidate pair with its renamed names and values",
            inspection: "Do the two sites perform the same steps for the same purpose, so one shared implementation would serve both?",
            acceptable_example: "Different work that only looks alike, or repetition the behavior requires",
            requires_tests: false,
            evaluation_dataset: DATASET,
            thresholds_validated: false,
        },
        Rule {
            id: "tests/value",
            key: TEST_VALUE,
            version: rule_version(TEST_VALUE),
            scope: "test cases, with --include-tests",
            unit: "one test's source and the signatures it calls",
            inspection: "Does the test check only its mocks, recompute the expected value with the code's own logic, assert internal details, or mix unrelated behaviors?",
            acceptable_example: "A test that checks a result or effect a caller can observe",
            requires_tests: true,
            evaluation_dataset: DATASET,
            thresholds_validated: false,
        },
        Rule {
            id: "tests/redundancy",
            key: TEST_REDUNDANCY,
            version: rule_version(TEST_REDUNDANCY),
            scope: "similar tests of one function, with --include-tests",
            unit: "one candidate pair of tests and their shared subject",
            inspection: "Do the two tests check the same behavior, with different or equivalent inputs?",
            acceptable_example: "Tests of different behaviors of one function",
            requires_tests: true,
            evaluation_dataset: DATASET,
            thresholds_validated: false,
        },
    ]
}

pub fn rule_version(key: &str) -> &'static str {
    match key {
        FILE_ORGANIZATION => "13",
        FUNCTION_SIMPLIFICATION => "11",
        SHARED_LOGIC => "17",
        TEST_VALUE => "2",
        _ => "1",
    }
}

pub fn keys() -> Vec<&'static str> {
    rules().into_iter().map(|r| r.key).collect()
}

/// A rule key or its catalog ID.
pub fn find(name: &str) -> Option<Rule> {
    rules().into_iter().find(|r| r.key == name || r.id == name)
}

pub fn id(key: &str) -> &'static str {
    find(key).map_or("unknown", |r| r.id)
}

pub fn policy() -> BTreeMap<String, f64> {
    use crate::response::REVIEW_PROBABILITY;
    BTreeMap::from([
        ("review_probability".into(), REVIEW_PROBABILITY),
        ("clear_probability".into(), REVIEW_PROBABILITY),
        ("consider_probability".into(), REVIEW_PROBABILITY),
        (
            "location_probability".into(),
            crate::response::LOCATION_PROBABILITY,
        ),
        (
            "min_body_lines".into(),
            crate::analysis::units::MIN_BODY_LINES as f64,
        ),
        (
            "min_clone_bytes".into(),
            crate::analysis::clones::MIN_BYTES as f64,
        ),
    ])
}

pub fn describe() -> Value {
    Value::Array(
        rules()
            .into_iter()
            .map(|r| {
                let mut value = serde_json::to_value(r).unwrap();
                value["decision_policy"] = serde_json::json!(policy());
                value
            })
            .collect(),
    )
}
