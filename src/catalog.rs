use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Serialize)]
pub struct Rule {
    pub id: &'static str,
    pub key: &'static str,
    /// The part of the ID before the slash, usable wherever a rule is.
    pub group: &'static str,
    /// Selected when no rules are configured; opt-in rules are not.
    pub default_enabled: bool,
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
pub const HARDCODED_VALUES: &str = "hardcoded_values";
pub const TEST_VALUE: &str = "test_value";
pub const TEST_REDUNDANCY: &str = "test_redundancy";

const DATASET: &str = "focused development set; not calibrated";

pub fn rules() -> Vec<Rule> {
    vec![
        Rule {
            id: "maintainability/file-organization",
            group: "maintainability",
            default_enabled: true,
            key: FILE_ORGANIZATION,
            version: rule_version(FILE_ORGANIZATION),
            scope: "application files with two or more members and 100 or more lines of member code",
            unit: "file outline: member signatures, callers that import the file, and groups; no bodies",
            inspection: "Would moving some members into a separate module make the file easier to understand and maintain?",
            acceptable_example: "One feature, one type and its helpers, or one set of related utilities",
            requires_tests: false,
            evaluation_dataset: DATASET,
            thresholds_validated: false,
        },
        Rule {
            id: "maintainability/function-simplification",
            group: "maintainability",
            default_enabled: true,
            key: FUNCTION_SIMPLIFICATION,
            version: rule_version(FUNCTION_SIMPLIFICATION),
            scope: "functions and methods with bodies of five or more lines",
            unit: "one function's source",
            inspection: "Would splitting the function into named functions make it easier to understand? For control flow nested four deep or four-branch chains: would flattening it help?",
            acceptable_example: "One job whose steps belong together or already call named functions",
            requires_tests: false,
            evaluation_dataset: DATASET,
            thresholds_validated: false,
        },
        Rule {
            id: "maintainability/shared-logic",
            group: "maintainability",
            default_enabled: true,
            key: SHARED_LOGIC,
            version: rule_version(SHARED_LOGIC),
            scope: "renamed or exact copies of two or more statements across selected files and explicit context",
            unit: "one representative pair per clone group, with its renamed names and values",
            inspection: "Do the two sites perform the same steps for the same purpose, so one shared implementation would serve both?",
            acceptable_example: "Different work that only looks alike, or repetition the behavior requires",
            requires_tests: false,
            evaluation_dataset: DATASET,
            thresholds_validated: false,
        },
        Rule {
            id: "maintainability/hardcoded-values",
            group: "maintainability",
            default_enabled: true,
            key: HARDCODED_VALUES,
            version: rule_version(HARDCODED_VALUES),
            scope: "application functions and module constants that use literal values other than 0, 1, 2 or one-character strings",
            unit: "one function's source with its literal values, or a file's module-level constants",
            inspection: "Does a value fixed in code change between deployments, need a descriptive name, or special-case one identity?",
            acceptable_example: "Messages, formats, protocol names and values whose meaning the code around them makes clear",
            requires_tests: false,
            evaluation_dataset: DATASET,
            thresholds_validated: false,
        },
        Rule {
            id: "tests/value",
            group: "tests",
            default_enabled: true,
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
            group: "tests",
            default_enabled: true,
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
        FILE_ORGANIZATION => "15",
        FUNCTION_SIMPLIFICATION => "13",
        SHARED_LOGIC => "19",
        TEST_VALUE => "3",
        _ => "1",
    }
}

pub fn keys() -> Vec<&'static str> {
    rules().into_iter().map(|r| r.key).collect()
}

/// Every rule selected when none are configured.
pub const DEFAULT_GROUP: &str = "default";
pub const ALL_GROUP: &str = "all";

pub fn groups() -> Vec<&'static str> {
    let mut groups: Vec<&str> = rules().into_iter().map(|r| r.group).collect();
    groups.dedup();
    groups
}

/// The rule keys a rule ID, key or group names; `None` when it names nothing.
pub fn select(name: &str) -> Option<Vec<&'static str>> {
    let selected: Vec<&str> = rules()
        .into_iter()
        .filter(|r| match name {
            ALL_GROUP => true,
            DEFAULT_GROUP => r.default_enabled,
            _ => r.key == name || r.id == name || r.group == name,
        })
        .map(|r| r.key)
        .collect();
    (!selected.is_empty()).then_some(selected)
}

/// How specifically `name` addresses `rule`: 3 for the rule itself, 2 for its
/// group, 1 for `default` or `all`, 0 when it does not address it.
pub fn specificity(name: &str, rule: &Rule) -> u8 {
    if name == rule.key || name == rule.id {
        3
    } else if name == rule.group {
        2
    } else if name == ALL_GROUP || (name == DEFAULT_GROUP && rule.default_enabled) {
        1
    } else {
        0
    }
}

/// A rule key or its catalog ID.
pub fn find(name: &str) -> Option<Rule> {
    rules().into_iter().find(|r| r.key == name || r.id == name)
}

pub fn id(key: &str) -> &'static str {
    find(key).map_or("unknown", |r| r.id)
}

pub fn policy() -> BTreeMap<String, f64> {
    use crate::policy::REVIEW_PROBABILITY;
    BTreeMap::from([
        ("review_probability".into(), REVIEW_PROBABILITY),
        ("clear_probability".into(), REVIEW_PROBABILITY),
        ("consider_probability".into(), REVIEW_PROBABILITY),
        (
            "consider_leading_probability".into(),
            crate::policy::LEADING_PROBABILITY,
        ),
        (
            "location_probability".into(),
            crate::policy::LOCATION_PROBABILITY,
        ),
        (
            "min_body_lines".into(),
            crate::analysis::units::MIN_BODY_LINES as f64,
        ),
        (
            "min_clone_bytes".into(),
            crate::analysis::clones::MIN_BYTES as f64,
        ),
        (
            "min_clone_statements".into(),
            crate::analysis::clones::MIN_CLONE_STATEMENTS as f64,
        ),
        (
            "min_file_lines".into(),
            crate::units::outline::MIN_FILE_LINES as f64,
        ),
        (
            "deep_nesting".into(),
            crate::analysis::nesting::DEEP_NESTING as f64,
        ),
        (
            "long_branch_chain".into(),
            crate::analysis::nesting::LONG_CHAIN as f64,
        ),
    ])
}

/// One line per rule: ID, whether it runs by default, whether it needs
/// `--include-tests`, and its question; groups and selection follow.
pub fn table() -> String {
    let rules = rules();
    let width = rules.iter().map(|r| r.id.len()).max().unwrap_or(0);
    let mut lines = vec![format!("{:width$}  DEFAULT  QUESTION", "RULE")];
    for rule in &rules {
        let default = match (rule.default_enabled, rule.requires_tests) {
            (false, _) => "opt-in",
            (true, true) => "tests",
            (true, false) => "yes",
        };
        lines.push(format!(
            "{:width$}  {default:7}  {}",
            rule.id, rule.inspection
        ));
    }
    lines.push(String::new());
    lines.push(format!(
        "Groups: {}, {DEFAULT_GROUP} (every rule marked yes or tests), {ALL_GROUP}.",
        groups().join(", ")
    ));
    lines.push("Select with --rule and --skip-rule, or [rules] in jevgate.toml; `tests` rules need --include-tests.".into());
    lines.join("\n")
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
