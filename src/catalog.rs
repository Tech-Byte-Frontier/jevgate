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
}

/// The documentation site, published from `site/` with each release.
pub const SITE: &str = "https://tech-byte-frontier.github.io/jevgate/";

impl Rule {
    /// The rule's page on the site, `site/src/rules/<ID>.md`: what it looks
    /// at, how often it was right, and findings it got wrong. A custom
    /// question, a team's own, has none: its page is the one on writing and
    /// testing custom questions.
    pub fn page(&self) -> String {
        if self.group == CUSTOM_GROUP {
            format!("{SITE}custom-questions.html")
        } else {
            format!("{SITE}rules/{}.html", self.id)
        }
    }
}

pub const FILE_ORGANIZATION: &str = "file_organization";
pub const FUNCTION_SIMPLIFICATION: &str = "function_simplification";
pub const SHARED_LOGIC: &str = "shared_logic";
pub const HARDCODED_VALUES: &str = "hardcoded_values";
pub const COMMENTS: &str = "comments";
pub const INJECTION: &str = "injection";
pub const SENSITIVE_DATA: &str = "sensitive_data";
pub const UNSAFE_SETTINGS: &str = "unsafe_settings";
pub const SECURITY: [&str; 3] = [INJECTION, SENSITIVE_DATA, UNSAFE_SETTINGS];
pub const ACCESS_CONTROL: &str = "access_control";
pub const WORKFLOWS: &str = "workflows";
pub const TEST_VALUE: &str = "test_value";
pub const TEST_REDUNDANCY: &str = "test_redundancy";
pub const LAWS: &str = "laws";
pub const AGENT_CONTEXT: &str = "agent_context";
pub const LARGE_DOCS: &str = "large_docs";
pub const DOC_STALENESS: &str = "doc_staleness";
pub const DOC_DUPLICATION: &str = "doc_duplication";
pub const DOCUMENTATION: [&str; 4] = [AGENT_CONTEXT, LARGE_DOCS, DOC_STALENESS, DOC_DUPLICATION];

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
            inspection: "Does the file do several separate kinds of work, or test several unrelated modules, that a maintainer could keep in separate files?",
            acceptable_example: "One feature, type, resource, screen or job and its helpers, even when it is long",
            requires_tests: false,
        },
        Rule {
            id: "maintainability/function-simplification",
            group: "maintainability",
            default_enabled: true,
            key: FUNCTION_SIMPLIFICATION,
            version: rule_version(FUNCTION_SIMPLIFICATION),
            scope: "functions and methods with bodies of five or more lines",
            unit: "one function's source",
            inspection: "Could the function be made noticeably simpler to read or change: is it long, deeply nested or repetitive, or does it mix separate jobs? Would splitting it into named functions, or flattening control flow nested four deep, make it easier to understand?",
            acceptable_example: "One job whose steps belong together or already call named functions",
            requires_tests: false,
        },
        Rule {
            id: "maintainability/shared-logic",
            group: "maintainability",
            default_enabled: true,
            key: SHARED_LOGIC,
            version: rule_version(SHARED_LOGIC),
            scope: "renamed or exact copies of two or more statements across selected files and explicit context",
            unit: "up to six sites of one repeated run of statements or tokens, each with its path and function",
            inspection: "Do repeated snippets express the same rule, lookup or sequence of steps, so that a maintainer should keep it in one place?",
            acceptable_example: "Calls every user of an API writes the same way, complementary operations, and code that must stay separate",
            requires_tests: false,
        },
        Rule {
            id: "maintainability/hardcoded-values",
            group: "maintainability",
            // Opt-in: 6 of its 37 labeled reviews and considers were right on
            // projects JevGate was never tuned on (16%), against 47 of 85 on
            // the projects it was tuned on (55%).
            default_enabled: false,
            key: HARDCODED_VALUES,
            version: rule_version(HARDCODED_VALUES),
            scope: "application functions and module constants that use literal values other than 0, 1, 2 or one-character strings",
            unit: "one function's source, or a file's module-level constants with their values",
            inspection: "Does a function or constant fix a value a maintainer should look at: one that singles out a record, place or user, differs between environments, is likely to change, or is an unexplained number?",
            acceptable_example: "Messages, keys, formats, small counts, and values a name or the code around them explains",
            requires_tests: false,
        },
        Rule {
            id: "security/injection",
            group: "security",
            default_enabled: false,
            key: INJECTION,
            version: rule_version(INJECTION),
            scope: "application functions with calls, built text or field assignments, and PHP page scripts",
            unit: "one function's source or a PHP file's top-level code; then its statements as sites, and up to three callers when the origin of its values is unclear",
            inspection: "Does a variable that another party controls reach the text of a query, command, code, markup, file path, requested URL or redirect target, or a deserializer, without being bound, escaped or checked?",
            acceptable_example: "Bound query parameters, argument lists, escaping templates, and values the program fixes or checks",
            requires_tests: false,
        },
        Rule {
            id: "security/sensitive-data",
            group: "security",
            default_enabled: false,
            key: SENSITIVE_DATA,
            version: rule_version(SENSITIVE_DATA),
            scope: "application functions with calls, built text or field assignments, and PHP page scripts",
            unit: "one function's source or a PHP file's top-level code; then its statements as sites and the message of each error it creates; one question per registered web error handler",
            inspection: "Does the function log a password, token, key or personal data, or send internal error details to a remote client? Does an error handler send clients more than the program's own messages and codes?",
            acceptable_example: "Logging record ids and messages; generic error responses with details kept in server logs",
            requires_tests: false,
        },
        Rule {
            id: "security/unsafe-settings",
            group: "security",
            default_enabled: false,
            key: UNSAFE_SETTINGS,
            version: rule_version(UNSAFE_SETTINGS),
            scope: "application functions, each file's top-level statements that call something, the settings objects of `next.config` files, and PHP page scripts",
            unit: "one function's source or the file's setup statements; then their statements as sites",
            inspection: "Does the code turn off a security check or choose a weak setting: certificate verification, password hashing, random tokens, CORS, cookies, or secrets in environment variables the build puts into browser code?",
            acceptable_example: "MD5 for cache keys, non-cryptographic random for shuffling, secure defaults",
            requires_tests: false,
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
        },
        Rule {
            id: "tests/laws",
            group: "tests",
            default_enabled: true,
            key: LAWS,
            version: rule_version(LAWS),
            scope: "Bend 2 laws that state a claim (an equality, a witness or a proposition) under a comment, outside tests",
            unit: "one law with its comment and the signatures, documentation and short bodies of the definitions it names",
            inspection: "Does the comment above a law promise more than, or something other than, what the law states, so a definition could break the promise while every proof passes?",
            acceptable_example: "A comment that puts its law in words; laws that declare a signature or a primitive",
            requires_tests: false,
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
        },
        Rule {
            id: "documentation/comments",
            group: "documentation",
            default_enabled: false,
            key: COMMENTS,
            version: rule_version(COMMENTS),
            scope: "comments and docstrings of application code, except license headers, tool directives and type annotations",
            unit: "one comment with the code it is about: the declaration it documents, the lines below it or the line it ends; then the whole definition it sits in",
            inspection: "Does a comment only repeat its code, hold sentences that add nothing, narrate an edit instead of the code as it is, or hold code turned off?",
            acceptable_example: "Reasons, constraints, caveats, references, and documentation of what a definition returns or guarantees beyond its signature",
            requires_tests: false,
        },
    ]
}

pub fn rule_version(key: &str) -> &'static str {
    match key {
        FILE_ORGANIZATION => "23",
        FUNCTION_SIMPLIFICATION => "16",
        SHARED_LOGIC => "24",
        TEST_VALUE => "7",
        TEST_REDUNDANCY => "4",
        INJECTION => "13",
        SENSITIVE_DATA => "9",
        HARDCODED_VALUES => "8",
        UNSAFE_SETTINGS => "7",
        AGENT_CONTEXT => "3",
        COMMENTS => "3",
        LARGE_DOCS => "4",
        ACCESS_CONTROL => "4",
        DOC_STALENESS | DOC_DUPLICATION => "4",
        WORKFLOWS => "2",
        _ => "1",
    }
}

#[cfg(test)]
pub fn keys() -> Vec<&'static str> {
    rules().into_iter().map(|r| r.key).collect()
}

/// Every rule selected when none are configured.
pub const DEFAULT_GROUP: &str = "default";
pub const ALL_GROUP: &str = "all";
/// The group of every custom question, whose rule IDs are `custom/<id>`.
pub const CUSTOM_GROUP: &str = "custom";

pub fn groups() -> Vec<&'static str> {
    let mut groups: Vec<&str> = rules().into_iter().map(|r| r.group).collect();
    groups.dedup();
    groups
}

/// Whether `name` is the rule's ID (`maintainability/file-organization`),
/// its name (`file-organization`, the ID after its group) or its key
/// (`file_organization`). A custom question has no short name: its id
/// could be a built-in rule's name, such as `comments`.
pub fn names(rule: &Rule, name: &str) -> bool {
    name == rule.key
        || name == rule.id
        || rule.group != CUSTOM_GROUP
            && rule
                .id
                .rsplit_once('/')
                .is_some_and(|(_, short)| short == name)
}

/// The rule keys a rule ID, name, key or group names; `None` when it names
/// nothing.
pub fn select(name: &str) -> Option<Vec<&'static str>> {
    select_in(&rules(), name)
}

/// The keys of the rules among `rules` that `name` names, as [`select`].
pub fn select_in(rules: &[Rule], name: &str) -> Option<Vec<&'static str>> {
    let selected: Vec<&str> = rules
        .iter()
        .filter(|r| match name {
            ALL_GROUP => true,
            DEFAULT_GROUP => r.default_enabled,
            _ => names(r, name) || r.group == name,
        })
        .map(|r| r.key)
        .collect();
    (!selected.is_empty()).then_some(selected)
}

/// The keys of the rules among `rules` that naming `name` turns on: as
/// [`select_in`], but a group turns on only the rules it runs by default
/// when it has any, so an opt-in rule of a default group (hardcoded values)
/// runs only when its own name or `all` asks for it. A group with none, such
/// as `security`, turns on every rule of it. Before 0.32 a level for
/// `maintainability` turned hardcoded values on: a configuration written for
/// 0.8 that way asked 1,846 requests of a 950-file project where its three
/// default rules needed 943, for a rule right 17% of the time.
pub fn enable_in(rules: &[Rule], name: &str) -> Option<Vec<&'static str>> {
    let selected = select_in(rules, name)?;
    Some(
        selected
            .into_iter()
            .filter(|key| {
                rules
                    .iter()
                    .find(|r| r.key == *key)
                    .is_none_or(|rule| enabled_by(name, rule, rules))
            })
            .collect(),
    )
}

/// Whether naming `name`, which addresses `rule`, turns it on: always,
/// except for an opt-in rule named by a group that runs rules by default.
fn enabled_by(name: &str, rule: &Rule, rules: &[Rule]) -> bool {
    rule.default_enabled
        || name != rule.group
        || !rules
            .iter()
            .any(|other| other.group == rule.group && other.default_enabled)
}

/// Whether a level set for `name` turns `rule` on, as [`enable_in`] does.
pub fn enables(name: &str, rule: &Rule, rules: &[Rule]) -> bool {
    specificity(name, rule) > 0 && enabled_by(name, rule, rules)
}

/// The built-in rules, then the custom questions in the order they are
/// defined.
pub fn with_custom(questions: &'static [crate::custom::Question]) -> Vec<Rule> {
    let mut all = rules();
    all.extend(questions.iter().map(crate::custom::Question::rule));
    all
}

/// Whether a rule key or ID names a custom question: `custom/<id>`.
pub fn custom(key: &str) -> bool {
    key.strip_prefix(CUSTOM_GROUP)
        .is_some_and(|rest| rest.starts_with('/'))
}

/// How specifically `name` addresses `rule`: 3 for the rule itself, 2 for its
/// group, 1 for `default` or `all`, 0 when it does not address it.
pub fn specificity(name: &str, rule: &Rule) -> u8 {
    if names(rule, name) {
        3
    } else if name == rule.group {
        2
    } else if name == ALL_GROUP || (name == DEFAULT_GROUP && rule.default_enabled) {
        1
    } else {
        0
    }
}

/// A rule by its key, catalog ID or name.
pub fn find(name: &str) -> Option<Rule> {
    rules().into_iter().find(|r| names(r, name))
}

/// A rule's ID by its key; a custom question's key is its ID.
pub fn id(key: &str) -> &str {
    match find(key) {
        Some(rule) => rule.id,
        None if custom(key) => key,
        None => "unknown",
    }
}

/// The thresholds and floors findings are decided with, as the report and
/// `jevgate rules --format json` record them.
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
        ("look_probability".into(), crate::policy::LOOK_PROBABILITY),
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
            "min_repeat_tokens".into(),
            crate::analysis::clones::RUN_TOKENS as f64,
        ),
        (
            "min_file_lines".into(),
            crate::units::outline::MIN_FILE_LINES as f64,
        ),
        (
            "min_bend_file_lines".into(),
            crate::units::outline::MIN_BEND_FILE_LINES as f64,
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
/// `--include-tests`, the levels that fail the default gate, how often its
/// reviews and considers were right on unseen projects, and its question,
/// then the custom questions with what they are asked about; what the
/// columns mean, groups, selection and the site's pages follow.
pub fn table(questions: &'static [crate::custom::Question]) -> String {
    let rules = with_custom(questions);
    let width = rules.iter().map(|r| r.id.len()).max().unwrap_or(0);
    let mut lines = vec![format!(
        "{:width$}  DEFAULT  BLOCKS    REVIEWS RIGHT  CONSIDERS RIGHT  QUESTION",
        "RULE"
    )];
    lines.extend(rules.iter().map(|rule| {
        let question = questions.iter().find(|q| q.rule == rule.id);
        table_row(rule, width, question)
    }));
    let mut groups = groups();
    if !questions.is_empty() {
        groups.push(CUSTOM_GROUP);
    }
    lines.push(String::new());
    lines.push(format!(
        "BLOCKS: the levels that fail the check by default, right at least {}% of the time over at least {} labeled findings on projects JevGate was never tuned on; an opt-in rule's levels fail it once the rule is selected, and a custom question fails it at its own level. The rest, and every finding in a preview language such as Kotlin, are reported without failing it until they measure up. Every finding is reported as a review; REVIEWS RIGHT and CONSIDERS RIGHT: for the level its measured questions composed, the share of labeled findings right on those projects, a debatable one counting as not right, or below {} labels how many were right of those labeled; - for a look-here question's findings, not yet measured; tests/laws is labeled only on Bend 2 projects, which these numbers leave out.",
        crate::maturity::MIN_PERCENT_RIGHT,
        crate::maturity::MIN_LABELS,
        crate::maturity::MIN_LABELS
    ));
    lines.push(format!(
        "Groups: {}, {DEFAULT_GROUP} (every rule marked yes or tests), {ALL_GROUP}. A group turns on its rules marked yes or tests, or every rule of it when it has none (security, documentation); an opt-in rule of another group runs when named, or with {ALL_GROUP}.",
        groups.join(", ")
    ));
    lines.push("Select with --rule and --skip-rule, or [rules] in jevgate.toml; `tests` rules need --include-tests. --fail-on and [rules] levels replace the default gate.".into());
    lines.push(format!(
        "Custom questions come from [[question]] in jevgate.toml and {}/*.toml; `jevgate rules add` copies measured ones from the gallery.",
        crate::custom::DIRECTORY
    ));
    lines.push(format!(
        "How the shares are measured: {SITE}accuracy.html; each rule's page, with findings it got wrong: {SITE}rules/RULE.html."
    ));
    lines.join("\n")
}

/// A rule's row; a custom question fails the default gate at its own level
/// and shows what it is asked about after its question.
fn table_row(rule: &Rule, width: usize, question: Option<&crate::custom::Question>) -> String {
    use crate::{maturity, schema::Strength};
    let default = match (rule.default_enabled, rule.requires_tests) {
        (false, _) => "opt-in",
        (true, true) => "tests",
        (true, false) => "yes",
    };
    let levels = question.map_or_else(|| maturity::mature_levels(rule.key), |q| q.blocks());
    let blocks = if levels.is_empty() {
        "-".to_string()
    } else {
        let names: Vec<String> = levels.iter().map(crate::output::label).collect();
        names.join(", ")
    };
    let right = |level| {
        maturity::measure(rule.key, level)
            .and_then(|m| m.unseen.summary())
            .unwrap_or_else(|| "-".into())
    };
    let asked = question.map_or(String::new(), |q| format!(" [{}]", q.summary()));
    format!(
        "{:width$}  {default:7}  {blocks:8}  {:13}  {:15}  {}{asked}",
        rule.id,
        right(Strength::Review),
        right(Strength::Consider),
        rule.inspection
    )
}

/// Every rule for `jevgate rules --format json`: its catalog entry, its
/// labels per level (`maturity`) and where they come from
/// (`evaluation_dataset`; for a custom question, the file that defines it),
/// and the decision policy; a custom question also carries its definition
/// (`custom`).
pub fn describe(questions: &'static [crate::custom::Question]) -> Value {
    Value::Array(
        with_custom(questions)
            .into_iter()
            .map(|r| {
                let key = r.key;
                let question = questions.iter().find(|q| q.rule == r.id);
                let mut value = serde_json::to_value(r).unwrap();
                value["maturity"] = crate::maturity::describe(key);
                value["evaluation_dataset"] = question
                    .map_or_else(|| crate::maturity::dataset(key), |q| q.provenance().into())
                    .into();
                value["decision_policy"] = serde_json::json!(policy());
                if let Some(question) = question {
                    value["custom"] = question.describe();
                }
                value
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    fn site() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("site/src")
    }

    /// SARIF links each rule to its page, so every rule needs one: the page
    /// shows the facts `site/generate.py` writes under the rule's anchor, and
    /// the site's table of contents lists it.
    #[test]
    fn every_rule_has_a_page_on_the_site() {
        let summary = std::fs::read_to_string(site().join("SUMMARY.md")).unwrap();
        for rule in rules() {
            let page = std::fs::read_to_string(site().join(format!("rules/{}.md", rule.id)))
                .unwrap_or_else(|_| panic!("site/src/rules/{}.md is missing", rule.id));
            let facts = format!("_rules.md:{}}}}}", rule.id.replace('/', "-"));
            assert!(
                page.contains(&facts),
                "{} does not include {facts}",
                rule.id
            );
            assert!(
                summary.contains(&format!("(rules/{}.md)", rule.id)),
                "{}",
                rule.id
            );
        }
        assert_eq!(
            find("shared-logic").unwrap().page(),
            "https://tech-byte-frontier.github.io/jevgate/rules/maintainability/shared-logic.html"
        );
    }

    /// Every Markdown file under `dir`, at any depth; other files, such as
    /// the `.DS_Store` a file browser leaves, are not pages.
    fn markdown(dir: &Path) -> Vec<PathBuf> {
        let entries = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().path());
        entries
            .flat_map(|path| {
                if path.is_dir() {
                    markdown(&path)
                } else {
                    vec![path]
                }
            })
            .filter(|path| path.extension().is_some_and(|extension| extension == "md"))
            .collect()
    }

    #[test]
    fn every_rule_page_names_a_rule() {
        let root = site().join("rules");
        for page in markdown(&root) {
            let relative = page.strip_prefix(&root).unwrap().with_extension("");
            let parts: Vec<_> = relative.iter().map(|part| part.to_string_lossy()).collect();
            let id = parts.join("/");
            assert!(
                rules().iter().any(|rule| rule.id == id),
                "{} names no rule",
                page.display()
            );
        }
    }
}
