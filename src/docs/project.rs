//! What an agent could learn from the repository's own files: manifests with
//! their dependencies and scripts, the tools that check formatting and lint
//! rules, and the directories. Sent as evidence beside instruction sections.
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

/// Names listed per manifest field; the rest are counted.
const LISTED: usize = 40;
const MANIFESTS: usize = 80;
const DIRECTORIES: usize = 40;

#[derive(Clone, Debug, Default)]
pub struct Project {
    /// Manifests with their names, dependencies and scripts.
    pub manifests: Vec<Value>,
    /// Tools configured to check or fix formatting and lint rules.
    pub linters: Vec<String>,
    /// Top-level directories and packages below them, with a trailing slash.
    pub directories: Vec<String>,
}

pub fn read(root: &Path, visited: &BTreeSet<PathBuf>) -> Project {
    let mut project = Project::default();
    let mut linters = BTreeSet::new();
    let at = |depth: usize| {
        visited.iter().filter(move |d| {
            d.components().count() == depth
                && !d.iter().any(|p| p.to_string_lossy().starts_with('.'))
        })
    };
    let root_dir = PathBuf::new();
    let bases: Vec<&PathBuf> = std::iter::once(&root_dir)
        .chain(at(1))
        .chain(at(2))
        .collect();
    for base in bases {
        let manifests = manifests(root, base, &mut linters);
        let package = !manifests.is_empty();
        if project.manifests.len() < MANIFESTS {
            project.manifests.extend(manifests);
        }
        let depth = base.components().count();
        if (depth == 1 || (depth == 2 && package)) && project.directories.len() < DIRECTORIES {
            project
                .directories
                .push(format!("{}/", base.to_string_lossy()));
        }
    }
    project.manifests.truncate(MANIFESTS);
    project.linters = linters.into_iter().collect();
    project
}

/// The manifests in one directory, recording the linters they configure.
fn manifests(root: &Path, base: &Path, linters: &mut BTreeSet<String>) -> Vec<Value> {
    let dir = root.join(base);
    let read = |name: &str| std::fs::read_to_string(dir.join(name)).ok();
    let path = |name: &str| base.join(name).to_string_lossy().into_owned();
    let mut found = Vec::new();
    for (file, tool) in CONFIG_FILES {
        if dir.join(file).is_file() {
            linters.insert((*tool).into());
        }
    }
    if let Some(table) = read("Cargo.toml").and_then(|t| t.parse::<toml::Table>().ok()) {
        let keys = |section: &str| {
            table
                .get(section)
                .and_then(|v| v.as_table())
                .map(|t| t.keys().cloned().collect::<Vec<_>>())
                .unwrap_or_default()
        };
        if table.get("lints").is_some()
            || table
                .get("workspace")
                .and_then(|w| w.get("lints"))
                .is_some()
        {
            linters.insert("cargo lints".into());
        }
        found.push(json!({
            "path": path("Cargo.toml"),
            "name": table.get("package").and_then(|p| p.get("name")).and_then(|n| n.as_str()),
            "dependencies": listed(keys("dependencies")),
            "dev_dependencies": listed(keys("dev-dependencies")),
        }));
    }
    if let Some(package) = read("package.json").and_then(|t| serde_json::from_str::<Value>(&t).ok())
    {
        let keys = |field: &str| {
            package[field]
                .as_object()
                .map(|o| o.keys().cloned().collect::<Vec<_>>())
                .unwrap_or_default()
        };
        let dev = keys("devDependencies");
        for (dependency, tool) in PACKAGE_LINTERS {
            if dev
                .iter()
                .chain(&keys("dependencies"))
                .any(|d| d == dependency)
            {
                linters.insert((*tool).into());
            }
        }
        let scripts: Vec<String> = package["scripts"]
            .as_object()
            .map(|o| {
                o.iter()
                    .map(|(name, command)| {
                        let command: String =
                            command.as_str().unwrap_or("").chars().take(80).collect();
                        format!("{name}: {command}")
                    })
                    .collect()
            })
            .unwrap_or_default();
        found.push(json!({
            "path": path("package.json"),
            "name": package["name"],
            "scripts": listed(scripts),
            "dependencies": listed(keys("dependencies")),
            "dev_dependencies": listed(dev),
        }));
    }
    if let Some(table) = read("pyproject.toml").and_then(|t| t.parse::<toml::Table>().ok()) {
        let project = table.get("project");
        let tools: Vec<String> = table
            .get("tool")
            .and_then(|t| t.as_table())
            .map(|t| t.keys().cloned().collect())
            .unwrap_or_default();
        for tool in &tools {
            if PYTHON_LINTERS.contains(&tool.as_str()) {
                linters.insert(tool.clone());
            }
        }
        let dependencies: Vec<String> = project
            .and_then(|p| p.get("dependencies"))
            .and_then(|d| d.as_array())
            .map(|d| {
                d.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        found.push(json!({
            "path": path("pyproject.toml"),
            "name": project.and_then(|p| p.get("name")).and_then(|n| n.as_str()),
            "dependencies": listed(dependencies),
            "tools": tools,
        }));
    }
    if let Some(text) = read("go.mod") {
        found.push(json!({
            "path": path("go.mod"),
            "name": text.lines().find_map(|l| l.strip_prefix("module ")).map(str::trim),
        }));
    }
    for name in ["Makefile", "justfile", "Justfile"] {
        if let Some(text) = read(name) {
            found.push(json!({"path": path(name), "targets": listed(targets(&text))}));
        }
    }
    found
}

/// The first `LISTED` names, then how many more there are.
fn listed(mut names: Vec<String>) -> Value {
    let extra = names.len().saturating_sub(LISTED);
    names.truncate(LISTED);
    if extra > 0 {
        names.push(format!("and {extra} more"));
    }
    json!(names)
}

/// Make and just recipe names: a name at the start of a line before `:`.
fn targets(text: &str) -> Vec<String> {
    text.lines()
        .filter(|line| !line.starts_with([' ', '\t', '#', '.']))
        .filter_map(|line| {
            let (name, rest) = line.split_once(':')?;
            let name = name.split_whitespace().next()?;
            (!rest.starts_with('=')
                && name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_')))
            .then(|| name.to_string())
        })
        .collect()
}

const CONFIG_FILES: &[(&str, &str)] = &[
    ("rustfmt.toml", "rustfmt"),
    (".rustfmt.toml", "rustfmt"),
    ("clippy.toml", "clippy"),
    (".clippy.toml", "clippy"),
    (".eslintrc", "eslint"),
    (".eslintrc.js", "eslint"),
    (".eslintrc.cjs", "eslint"),
    (".eslintrc.json", "eslint"),
    (".eslintrc.yml", "eslint"),
    ("eslint.config.js", "eslint"),
    ("eslint.config.mjs", "eslint"),
    ("eslint.config.ts", "eslint"),
    (".prettierrc", "prettier"),
    (".prettierrc.json", "prettier"),
    ("prettier.config.js", "prettier"),
    ("biome.json", "biome"),
    ("biome.jsonc", "biome"),
    ("ruff.toml", "ruff"),
    (".ruff.toml", "ruff"),
    (".flake8", "flake8"),
    (".golangci.yml", "golangci-lint"),
    (".golangci.yaml", "golangci-lint"),
    (".editorconfig", "editorconfig"),
    (".stylelintrc", "stylelint"),
    (".stylelintrc.json", "stylelint"),
    (".markdownlint.json", "markdownlint"),
    (".pre-commit-config.yaml", "pre-commit"),
    (".oxlintrc.json", "oxlint"),
    ("oxlint.json", "oxlint"),
    (".oxfmtrc.json", "oxfmt"),
    (".oxfmtrc.jsonc", "oxfmt"),
    ("dprint.json", "dprint"),
    (".dprint.json", "dprint"),
];

const PACKAGE_LINTERS: &[(&str, &str)] = &[
    ("eslint", "eslint"),
    ("prettier", "prettier"),
    ("@biomejs/biome", "biome"),
    ("stylelint", "stylelint"),
    ("oxlint", "oxlint"),
    ("oxfmt", "oxfmt"),
];

const PYTHON_LINTERS: &[&str] = &["ruff", "black", "isort", "flake8", "pylint"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifests_linters_and_directories() {
        let project = crate::tests::Project::new();
        project.write(
            "Cargo.toml",
            "[package]\nname = \"demo\"\n[dependencies]\nserde = \"1\"\n[lints.rust]\nunused = \"deny\"\n",
        );
        project.write(
            "web/package.json",
            r#"{"name":"web","scripts":{"test":"vitest"},"devDependencies":{"eslint":"9"}}"#,
        );
        project.write("Makefile", "build:\n\tcargo build\nX := 1\n.PHONY: build\n");
        project.write("rustfmt.toml", "");
        let visited: BTreeSet<PathBuf> = ["web", "src"].into_iter().map(PathBuf::from).collect();
        let read = read(&project.0, &visited);
        assert_eq!(read.linters, ["cargo lints", "eslint", "rustfmt"]);
        assert_eq!(read.directories, ["src/", "web/"]);
        assert_eq!(read.manifests[0]["dependencies"], json!(["serde"]));
        assert_eq!(read.manifests[1]["targets"], json!(["build"]));
        assert_eq!(read.manifests[2]["scripts"], json!(["test: vitest"]));
    }
}
