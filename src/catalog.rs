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
pub const INJECTION: &str = "injection";
pub const SENSITIVE_DATA: &str = "sensitive_data";
pub const UNSAFE_SETTINGS: &str = "unsafe_settings";
pub const SECURITY: [&str; 3] = [INJECTION, SENSITIVE_DATA, UNSAFE_SETTINGS];
pub const ACCESS_CONTROL: &str = "access_control";
pub const WORKFLOWS: &str = "workflows";
pub const TEST_VALUE: &str = "test_value";
pub const TEST_REDUNDANCY: &str = "test_redundancy";
pub const AGENT_CONTEXT: &str = "agent_context";
pub const LARGE_DOCS: &str = "large_docs";
pub const DOC_STALENESS: &str = "doc_staleness";
pub const DOC_DUPLICATION: &str = "doc_duplication";
pub const DOCUMENTATION: [&str; 4] = [AGENT_CONTEXT, LARGE_DOCS, DOC_STALENESS, DOC_DUPLICATION];

const DATASET: &str = "focused development set; not calibrated";

pub fn rules() -> Vec<Rule> {
    vec![
        Rule {
            id: "maintainability/file-organization",
            group: "maintainability",
            default_enabled: true,
            key: FILE_ORGANIZATION,
            version: rule_version(FILE_ORGANIZATION),
            scope: "application and test files with two or more members and 100 or more lines of member code",
            unit: "file outline: member signatures and sizes, callers that import the file, and groups; a test file lists its cases with their suites and subjects; no bodies",
            inspection: "Would moving some members into a separate module (or tests into a separate test file) make the file easier to navigate and maintain?",
            acceptable_example: "One algorithm, one type and its helpers, one feature, or the tests of one subject",
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
            id: "security/injection",
            group: "security",
            default_enabled: false,
            key: INJECTION,
            version: rule_version(INJECTION),
            scope: "application functions with calls, built text or field assignments",
            unit: "one function's source; then its statements as sites, and up to three callers when the origin of its values is unclear",
            inspection: "Does a variable that another party controls reach the text of a query, command, code, markup, file path or URL without being bound, escaped or checked?",
            acceptable_example: "Bound query parameters, argument lists, escaping templates, and values the program fixes or checks",
            requires_tests: false,
            evaluation_dataset: DATASET,
            thresholds_validated: false,
        },
        Rule {
            id: "security/sensitive-data",
            group: "security",
            default_enabled: false,
            key: SENSITIVE_DATA,
            version: rule_version(SENSITIVE_DATA),
            scope: "application functions with calls, built text or field assignments",
            unit: "one function's source; then its statements as sites and the message of each error it creates; one question per registered web error handler",
            inspection: "Does the function log a password, token, key or personal data, or send internal error details to a remote client? Does an error handler send clients more than the program's own messages and codes?",
            acceptable_example: "Logging record ids and messages; generic error responses with details kept in server logs",
            requires_tests: false,
            evaluation_dataset: DATASET,
            thresholds_validated: false,
        },
        Rule {
            id: "security/unsafe-settings",
            group: "security",
            default_enabled: false,
            key: UNSAFE_SETTINGS,
            version: rule_version(UNSAFE_SETTINGS),
            scope: "application functions, and each file's top-level statements that call something",
            unit: "one function's source or the file's setup statements; then their statements as sites",
            inspection: "Does the code turn off a security check or choose a weak setting: certificate verification, password hashing, random tokens, CORS or cookies?",
            acceptable_example: "MD5 for cache keys, non-cryptographic random for shuffling, secure defaults",
            requires_tests: false,
            evaluation_dataset: DATASET,
            thresholds_validated: false,
        },
        Rule {
            id: "security/access-control",
            group: "security",
            default_enabled: false,
            key: ACCESS_CONTROL,
            version: rule_version(ACCESS_CONTROL),
            scope: "SQL files: row-level security policies, SECURITY DEFINER functions and grants, in their final state across migrations; SpacetimeDB TypeScript modules: public tables, views and reducers",
            unit: "one policy with its table and the functions it calls, one SECURITY DEFINER function, or one grant; one SpacetimeDB public table with its user columns, or one view or reducer with the functions it calls and the framework version",
            inspection: "Does a policy let every user it applies to reach other users' rows, or trust a value users can change? Does a SECURITY DEFINER function leave search_path open or skip checking the caller? Does a grant open writes or private reads to every user? Does a public table hold users' own data, a view return other users' rows, or a reducer change rows its arguments choose, or admin-only settings, without checking the caller?",
            acceptable_example: "Policies tied to the user, account or membership; role checks; restrictive policies; public data; grants narrowed by row-level security; reducers that check the caller through `ctx.sender`, the module owner, an admin or a trusted service identity, or run only on a schedule",
            requires_tests: false,
            evaluation_dataset: DATASET,
            thresholds_validated: false,
        },
        Rule {
            id: "security/workflows",
            group: "security",
            default_enabled: false,
            key: WORKFLOWS,
            version: rule_version(WORKFLOWS),
            scope: "GitHub Actions jobs in .github/workflows",
            unit: "one job with the workflow's triggers and permissions, and the ${{ }} expressions in its run scripts",
            inspection: "Can a run script execute text that people outside the repository write? Does a job run pull request code while it has secrets or a write token?",
            acceptable_example: "Untrusted text passed through env variables; pull_request workflows; jobs that run only the base branch's code",
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
        Rule {
            id: "documentation/agent-context",
            group: "documentation",
            default_enabled: false,
            key: AGENT_CONTEXT,
            version: rule_version(AGENT_CONTEXT),
            scope: "agent instruction files that a harness loads: AGENTS.md, CLAUDE.md, GEMINI.md, and Claude, Cursor, Copilot, Windsurf and Cline rules",
            unit: "one file's heading sections, with the repository's manifests, linters and directories",
            inspection: "Does a section restate what the repository's files show, give generic advice, repeat what linters check, or record past work?",
            acceptable_example: "Project-specific commands, constraints, decisions and workflows the code does not show",
            requires_tests: false,
            evaluation_dataset: DATASET,
            thresholds_validated: false,
        },
        Rule {
            id: "documentation/large-docs",
            group: "documentation",
            default_enabled: false,
            key: LARGE_DOCS,
            version: rule_version(LARGE_DOCS),
            scope: "project Markdown of 300 or more lines: root files, README and CONTRIBUTING anywhere, docs/ and doc/ (read even when ignored)",
            unit: "one document's headings in order, without its text",
            inspection: "Would splitting the document make it easier to find and maintain, or does it mainly record past work?",
            acceptable_example: "One long guide, reference or concept, and living procedures",
            requires_tests: false,
            evaluation_dataset: DATASET,
            thresholds_validated: false,
        },
        Rule {
            id: "documentation/staleness",
            group: "documentation",
            default_enabled: false,
            key: DOC_STALENESS,
            version: rule_version(DOC_STALENESS),
            scope: "agent instruction files and project docs that name paths or scripts the repository lacks, or a released version",
            unit: "a document's headings when Git shows its release tagged or its paths deleted; then each section naming missing paths or scripts, with what Git shows about them",
            inspection: "Is the document a plan whose work is finished, or does a section tell the reader to use a path or script that no longer exists?",
            acceptable_example: "Outputs a command writes, local or ignored files, examples, and paths named as removed",
            requires_tests: false,
            evaluation_dataset: DATASET,
            thresholds_validated: false,
        },
        Rule {
            id: "documentation/duplication",
            group: "documentation",
            default_enabled: false,
            key: DOC_DUPLICATION,
            version: rule_version(DOC_DUPLICATION),
            scope: "sections of different agent instruction files and project docs that share much of their wording",
            unit: "one candidate pair of sections",
            inspection: "Does one section state everything the other states, or do the two give different values or instructions for the same thing?",
            acceptable_example: "Sections on the same subject where each adds something",
            requires_tests: false,
            evaluation_dataset: DATASET,
            thresholds_validated: false,
        },
    ]
}

pub fn rule_version(key: &str) -> &'static str {
    match key {
        FILE_ORGANIZATION => "17",
        FUNCTION_SIMPLIFICATION => "13",
        SHARED_LOGIC => "19",
        TEST_VALUE => "4",
        INJECTION => "5",
        SENSITIVE_DATA => "4",
        UNSAFE_SETTINGS => "2",
        HARDCODED_VALUES | ACCESS_CONTROL | DOC_STALENESS | DOC_DUPLICATION => "2",
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
