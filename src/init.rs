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
        let relative = entry.path().strip_prefix(root)?;
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
    let mut rules = String::new();
    for group in catalog::groups() {
        let members: Vec<_> = catalog::rules()
            .into_iter()
            .filter(|r| r.group == group)
            .collect();
        let names: Vec<&str> = members
            .iter()
            .map(|r| r.id.trim_start_matches(&format!("{group}/")[..]))
            .collect();
        let comment = format!("# {}", names.join(", "));
        if members.iter().all(|r| r.default_enabled) {
            rules.push_str(&format!("{group} = \"review\"  {comment}\n"));
        } else {
            rules.push_str(&format!("# {group} = \"consider\"  {comment} (opt-in)\n"));
        }
    }
    format!(
        r#"# JevGate configuration, written by `jevgate init`. Unknown keys are errors.
# `jevgate rules` lists every rule; `jevgate check --dry-run --show-requests`
# shows what would be uploaded without sending anything.

# Only these paths may be uploaded: the detected source and test directories,
# and agent instruction files for the documentation rules.
{allow}# Never uploaded, even when allowed above.
upload_deny = ["**/.env*", "**/*.pem", "**/*.key"]

# Also judge tests (test value and redundancy), as --include-tests does.
# File organization judges test files either way.
# include_tests = true

# TypeSafe is the default provider; select OpenRouter to use its Jev route.
# provider = "openrouter"

# The model, pinned so results stay repeatable; --model overrides it.
# model = "{model}"

# Budgets for one invocation; flags can only lower them.
# max_requests = 200
# concurrency = 4

# Each group or rule ID set to a level is judged and fails the check at that
# level: "review", "consider" (also fails on review), "uncertain", "report"
# (judge, never fail) or "off". A rule's own entry wins over its group's.
# Test rules also need include_tests or --include-tests.
[rules]
{rules}
# Levels for the files some paths match, such as report-only tooling. The last
# scope that matches a file and names a rule wins; flags win over scopes.
# [[scope]]
# paths = ["scripts/**", "tools/**"]
# fail_on = ["report"]
"#,
        model = crate::options::DEFAULT_MODEL
    )
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
        let config: Config = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(config.upload_allow, allow);
        let Rules::Levels(levels) = config.rules else {
            panic!("rules is a table of levels");
        };
        assert!(levels.contains_key("maintainability") && levels.contains_key("tests"));
        assert!(run(&dir, false).is_err(), "an existing file is kept");
        assert!(run(&dir, true).is_ok());
    }
}
