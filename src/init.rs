//! `jevgate init`: a commented `jevgate.toml` that limits uploads to the
//! detected source directories and lists the rule groups. Offline.
use crate::{catalog, discovery, syntax};
use anyhow::{Context, Result, ensure};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

pub const CONFIG_FILE: &str = "jevgate.toml";

/// Write the configuration at the project root and return its path and the
/// allowed upload patterns. An existing file is kept unless `force`.
pub fn run(root: &Path, force: bool) -> Result<(PathBuf, Vec<String>)> {
    let path = root.join(CONFIG_FILE);
    ensure!(
        force || !path.exists(),
        "{} already exists; pass --force to replace it",
        path.display()
    );
    let mut allow = source_patterns(root)?;
    allow.extend(instruction_patterns(root)?);
    std::fs::write(&path, render(&allow))
        .with_context(|| format!("Cannot write {}", path.display()))?;
    Ok((path, allow))
}

/// `dir/**` for each top-level directory holding source or tests in a
/// supported language. Root-level files are usually tool configuration, so
/// they are named only when no directory holds source.
fn source_patterns(root: &Path) -> Result<Vec<String>> {
    let classifier = discovery::Classifier::new(&Default::default())?;
    let mut dirs = BTreeSet::new();
    let mut files = BTreeSet::new();
    for entry in crate::inventory::walker(root) {
        let entry = entry.context("Failed while detecting source directories")?;
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let relative = &crate::discovery::relative(entry.path(), root)?;
        if !syntax::supported(relative) || !matches!(classifier.role(relative), "source" | "test") {
            continue;
        }
        let mut parts = relative.iter();
        let first = parts.next().unwrap_or_default().to_string_lossy();
        if parts.next().is_some() {
            dirs.insert(format!("{first}/**"));
        } else {
            files.insert(first.into_owned());
        }
    }
    Ok(if dirs.is_empty() { files } else { dirs }
        .into_iter()
        .collect())
}

/// Patterns for the agent instruction files present, so the documentation
/// rules can upload them: by name wherever they appear, or by rule directory.
fn instruction_patterns(root: &Path) -> Result<Vec<String>> {
    let found = crate::docs::discover::discover(root)?;
    let patterns: BTreeSet<String> = found
        .agent
        .iter()
        .map(|path| {
            let parts: Vec<String> = path
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect();
            let name = parts.last().map_or("", String::as_str);
            let hidden = parts.iter().position(|p| p.starts_with('.'));
            if crate::docs::discover::AGENT_NAMES.contains(&name) {
                format!("**/{name}")
            } else if let Some(at) = hidden.filter(|at| at + 2 < parts.len()) {
                format!("{}/**", parts[at..at + 2].join("/"))
            } else {
                parts.join("/")
            }
        })
        .collect();
    Ok(patterns.into_iter().collect())
}

fn render(allow: &[String]) -> String {
    let list = |items: &[String]| {
        let quoted: Vec<String> = items.iter().map(|i| format!("{i:?}")).collect();
        format!("[{}]", quoted.join(", "))
    };
    let allow = if allow.is_empty() {
        "# upload_allow = [\"src/**\"]\n".to_string()
    } else {
        format!("upload_allow = {}\n", list(allow))
    };
    let rules: String = catalog::groups().into_iter().map(group_example).collect();
    format!(
        r#"#:schema https://raw.githubusercontent.com/Tech-Byte-Frontier/jevgate/v{version}/jevgate.schema.json
# JevGate configuration, written by `jevgate init`. Unknown keys are errors.
# `jevgate rules` lists every rule; `jevgate check --dry-run --show-requests`
# shows what would be uploaded without sending anything.

# Only these paths may be uploaded: the detected source and test directories,
# and agent instruction files for the documentation rules.
{allow}# Never uploaded, even when allowed above.
upload_deny = ["**/.env*", "**/*.pem", "**/*.key"]

# Also judge tests (test value and redundancy), as --include-tests does.
# File organization judges test files either way, but not yet those of the
# preview languages (C, C++, Kotlin, Swift, Bash, Dart, Scala, Elixir, Lua).
# include_tests = true

# The model, pinned so results stay repeatable; --model overrides it. The
# default follows the key: this one for TypeSafe, typesafe/jev-1.13 for
# OpenRouter, typesafe-ai/jev for Vercel AI Gateway.
# model = "{model}"

# Budgets for one invocation; flags can only lower them. A check of the
# whole repository asks about one request per file, so a request ceiling
# sized for pull requests stops it; max_cost bounds the spend instead (a
# whole check of a 1,000-file project costs a few cents).
# max_cost = 1.00
# max_seconds = 300

# Unset, the default rules run and only rule levels measured right at least
# 80% of the time on projects JevGate was never tuned on fail the check
# ("mature"; `jevgate rules` shows them), never in a preview language; other
# findings are reported without failing it. A group or rule ID set to a
# level is judged, a group's opt-in rules only when named on their own
# (security and documentation are opt-in whole), and fails the check at
# exactly that level: "review", "consider" (also fails on review), "mature",
# "uncertain", "report" (judge, never fail) or "off". A rule's own entry wins
# over its group's. Test rules also need include_tests or --include-tests.
[rules]
{rules}
# Levels for the files some paths match, such as report-only tooling. The last
# scope that matches a file and names a rule wins; flags win over scopes.
# [[scope]]
# paths = ["scripts/**", "tools/**"]
# fail_on = ["report"]

# Custom questions: a team convention as a yes/no question whose yes is a
# finding, the rule custom/<id>. It fails the gate at its level. One question
# per file also works, in .jevgate/questions/<id>.toml, where
# `jevgate rules add` copies measured questions from the gallery.
# [[question]]
# id = "no-body-logs"
# question = "Does this function write a request body, or a field of one, to a log?"
# guidance = "Logging the method, path, request id or status is fine."
# unit = "function"  # function, file, test, section, comment, or hunk (with --base)
# paths = ["src/api/**"]
# level = "review"  # review, consider or note
# [[question.failing]]  # code that breaks the rule; `jevgate rules test` asks it
# path = "src/api/orders.py"
# code = "def create(req):\n    log.info(req.body)\n"
# [[question.passing]]  # code that keeps it
# path = "src/api/orders.py"
# code = "def create(req):\n    log.info(req.id)\n"
"#,
        model = crate::options::DEFAULT_MODEL,
        version = env!("CARGO_PKG_VERSION"),
    )
}

/// A commented `[rules]` line for one group: a level to set, and the rules
/// it turns on; then a line of its own for each opt-in rule of a group that
/// runs rules by default, which the group's level leaves out.
fn group_example(group: &str) -> String {
    let members: Vec<_> = catalog::rules()
        .into_iter()
        .filter(|r| r.group == group)
        .collect();
    let opt_in = members.iter().all(|r| !r.default_enabled);
    let name = |r: &catalog::Rule| {
        r.id.trim_start_matches(&format!("{group}/")[..])
            .to_string()
    };
    let names: Vec<String> = members
        .iter()
        .filter(|r| r.default_enabled || opt_in)
        .map(name)
        .collect();
    let (level, suffix) = if opt_in {
        ("consider", " (opt-in)")
    } else {
        ("review", "")
    };
    let mut lines = format!("# {group} = \"{level}\"  # {}{suffix}\n", names.join(", "));
    for rule in members.iter().filter(|r| !r.default_enabled && !opt_in) {
        lines.push_str(&format!(
            "# \"{}\" = \"consider\"  # {} (opt-in, only by name)\n",
            rule.id,
            name(rule)
        ));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::{Config, Rules},
        tests::Project,
    };

    #[test]
    fn written_configuration_is_valid_and_limits_uploads_to_sources() {
        let project = Project::new();
        for file in [
            "src/lib.rs",
            "tests/api.rs",
            "main.py",
            "docs/guide.md",
            "target/x.rs",
            "AGENTS.md",
            "web/CLAUDE.md",
            ".cursor/rules/style.mdc",
        ] {
            project.write(file, "fn f() {}\n");
        }
        let dir = project.0.to_path_buf();
        let (path, allow) = run(&dir, false).unwrap();
        assert_eq!(
            allow,
            [
                "src/**",
                "tests/**",
                "**/AGENTS.md",
                "**/CLAUDE.md",
                ".cursor/rules/**"
            ]
        );
        let text = std::fs::read_to_string(&path).unwrap();
        let config: Config = toml::from_str(&text).unwrap();
        assert_eq!(config.upload_allow, allow);
        let levels = |config: Config| match config.rules {
            Rules::Levels(levels) => levels,
            Rules::List(_) => panic!("rules is a table of levels"),
        };
        assert!(levels(config).is_empty(), "the default rules and gate");
        assert!(
            text.contains("# \"maintainability/hardcoded-values\" = \"consider\"  # hardcoded-values (opt-in, only by name)\n"),
            "{text}"
        );
        assert!(!text.contains("shared-logic, hardcoded-values"), "{text}");
        assert!(
            text.contains("shows them), never in a preview language;"),
            "{text}"
        );
        let uncommented: Vec<&str> = text
            .lines()
            .map(|line| {
                let example = catalog::groups().into_iter().any(|g| {
                    line.starts_with(&format!("# {g} = ")) || line.starts_with(&format!("# \"{g}/"))
                });
                if example { &line[2..] } else { line }
            })
            .collect();
        let config: Config = toml::from_str(&uncommented.join("\n")).unwrap();
        assert_eq!(levels(config).len(), catalog::groups().len() + 1);
        let example: String = text
            .lines()
            .skip_while(|line| *line != "# [[question]]")
            .map(|line| format!("{}\n", line.trim_start_matches("# ")))
            .collect();
        let questions = crate::custom::parse(&example).unwrap();
        assert_eq!(
            (questions[0].rule.as_str(), questions[0].examples.len()),
            ("custom/no-body-logs", 2),
            "the example and its examples are valid"
        );
        assert!(run(&dir, false).is_err(), "an existing file is kept");
        assert!(run(&dir, true).is_ok());
    }
}
