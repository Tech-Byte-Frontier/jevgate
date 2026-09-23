//! Paths and scripts a document names that the repository does not contain,
//! with what Git shows about them. These select sections for a staleness
//! check; they never decide one: outputs, local files and examples are named
//! too.
use super::history::History;
use std::{
    collections::BTreeSet,
    path::{Component, Path, PathBuf},
};

/// What the repository shows about a missing name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fate {
    Deleted,
    Renamed(PathBuf),
    /// Never tracked, or no Git history to tell.
    Absent,
    /// A script or target no manifest declares.
    NoScript,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Missing {
    pub name: String,
    pub fate: Fate,
}

impl Missing {
    /// The fact sent beside the section.
    pub fn status(&self) -> String {
        match &self.fate {
            Fate::Deleted => "deleted from the repository".into(),
            Fate::Renamed(to) => format!("renamed to {}", to.display()),
            Fate::Absent => "not in the repository".into(),
            Fate::NoScript => "no script or target with this name in the manifests".into(),
        }
    }
}

const EXTENSIONS: &[&str] = &[
    "md", "mdx", "ts", "tsx", "js", "jsx", "mjs", "cjs", "rs", "py", "json", "toml", "sh", "sql",
    "yml", "yaml", "css", "html", "lock", "go", "java", "kt", "rb",
];

/// Package-manager words that are commands, not scripts.
const BUILTIN: &[&str] = &[
    "install",
    "add",
    "remove",
    "exec",
    "dlx",
    "i",
    "ci",
    "run",
    "test",
    "build",
    "dev",
    "start",
    "update",
    "up",
    "why",
    "list",
    "ls",
    "outdated",
    "audit",
    "create",
    "init",
    "publish",
    "pack",
    "link",
    "store",
    "env",
    "setup",
    "rebuild",
    "prune",
    "fetch",
    "import",
    "dedupe",
    "patch",
    "config",
    "approve-builds",
    "version",
    "help",
    "info",
    "cache",
    "global",
    "workspace",
    "workspaces",
    "recursive",
];

/// Everything `text`, a section of `doc`, names that the repository lacks.
pub fn missing(
    root: &Path,
    doc: &Path,
    text: &str,
    history: &History,
    scripts: &BTreeSet<String>,
) -> Vec<Missing> {
    let base = doc.parent().unwrap_or(Path::new(""));
    let top: BTreeSet<&str> = history
        .tracked
        .iter()
        .filter_map(|p| p.iter().next()?.to_str())
        .collect();
    let mut found = BTreeSet::new();
    let mut out = Vec::new();
    for token in code_spans(text).into_iter().chain(link_targets(text)) {
        let Some(name) = path_like(&token, &top) else {
            continue;
        };
        if found.contains(&name) || present(root, base, &name, history) {
            continue;
        }
        found.insert(name.clone());
        let fate = [normal(Path::new(&name)), normal(&base.join(&name))]
            .iter()
            .find_map(|p| history.removed.get(p))
            .map_or(Fate::Absent, |to| match to {
                Some(to) => Fate::Renamed(to.clone()),
                None => Fate::Deleted,
            });
        out.push(Missing { name, fate });
    }
    if !scripts.is_empty() {
        for script in commands(text) {
            if !scripts.contains(&script) && found.insert(format!("script:{script}")) {
                out.push(Missing {
                    name: script,
                    fate: Fate::NoScript,
                });
            }
        }
    }
    out
}

/// A token naming a repository path: a file extension, or a first directory
/// the repository has. Routes, URLs, globs and placeholders are not.
fn path_like(token: &str, top: &BTreeSet<&str>) -> Option<String> {
    let mut t = token
        .trim()
        .trim_end_matches(['.', ',', ';', ':', ')'])
        .to_string();
    if let Some((path, line)) = t.rsplit_once(':')
        && line.chars().all(|c| c.is_ascii_digit() || c == '-')
    {
        t = path.to_string();
    }
    let excluded = ["http", "mailto:", "#", "$", "-", "@", "~", "/"]
        .iter()
        .any(|p| t.starts_with(p))
        || t.chars().any(|c| "*<>{}|=()[] ,'\"".contains(c))
        || t.is_empty();
    if excluded {
        return None;
    }
    let extension = Path::new(&t)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| EXTENSIONS.contains(&e));
    let first = t.split('/').next().unwrap_or("");
    (extension || (t.contains('/') && top.contains(first))).then_some(t)
}

fn present(root: &Path, base: &Path, name: &str, history: &History) -> bool {
    let trimmed = name.trim_end_matches('/');
    let candidates = [normal(Path::new(trimmed)), normal(&base.join(trimmed))];
    if candidates
        .iter()
        .any(|c| history.tracked.contains(c) || root.join(c).exists())
    {
        return true;
    }
    // A partial path such as `services/quota.ts` names a deeper file.
    let suffix = format!("/{trimmed}");
    history.tracked.iter().any(|p| {
        let p = p.to_string_lossy();
        p.ends_with(&suffix) || p.starts_with(&format!("{trimmed}/"))
    })
}

/// A relative path without `.` and `..` parts.
fn normal(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::ParentDir => {
                out.pop();
            }
            Component::Normal(p) => out.push(p),
            _ => {}
        }
    }
    out
}

/// Inline code spans without whitespace.
fn code_spans(text: &str) -> Vec<String> {
    text.split('`')
        .enumerate()
        .filter(|(i, part)| i % 2 == 1 && !part.contains(char::is_whitespace))
        .map(|(_, part)| part.to_string())
        .collect()
}

/// Relative Markdown link targets, without anchors.
fn link_targets(text: &str) -> Vec<String> {
    text.split("](")
        .skip(1)
        .filter_map(|rest| {
            let target = rest.split(')').next()?.split('#').next()?.trim();
            (!target.is_empty() && !target.contains(' ')).then(|| target.to_string())
        })
        .collect()
}

/// Scripts named after a package manager or task runner inside code.
fn commands(text: &str) -> Vec<String> {
    let code = code_text(text);
    let words: Vec<&str> = code.split_whitespace().collect();
    let mut out = Vec::new();
    for (i, word) in words.iter().enumerate() {
        let next = |n: usize| words.get(i + n).copied().unwrap_or("");
        let script = match *word {
            "pnpm" | "yarn" if next(1) == "run" => next(2),
            "npm" if next(1) == "run" => next(2),
            "pnpm" | "yarn" | "make" | "just" => next(1),
            _ => continue,
        };
        let name_like = script
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_lowercase())
            && script
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_:.".contains(c));
        if name_like && !BUILTIN.contains(&script) {
            out.push(script.to_string());
        }
    }
    out
}

/// Inline code spans and fenced code blocks, joined.
fn code_text(text: &str) -> String {
    let mut out = String::new();
    let mut fenced = false;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            out.push_str(line);
            out.push('\n');
        } else {
            for (i, part) in line.split('`').enumerate() {
                if i % 2 == 1 {
                    out.push_str(part);
                    out.push('\n');
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The missing names of `text`, a section of `docs/intro.md`, in a
    /// repository tracking a few files and with two removed.
    fn found(text: &str) -> Vec<(String, Fate)> {
        let project = crate::tests::Project::new();
        project.write("src/app.ts", "");
        let history = History {
            tracked: ["src/app.ts", "docs/guide.md", "src/services/quota.ts"]
                .into_iter()
                .map(PathBuf::from)
                .collect(),
            tags: BTreeSet::new(),
            removed: [
                (PathBuf::from("src/old.ts"), None),
                (
                    PathBuf::from("src/moved.ts"),
                    Some(PathBuf::from("src/new.ts")),
                ),
            ]
            .into(),
        };
        let scripts: BTreeSet<String> = ["dev".to_string(), "quality".to_string()].into();
        missing(
            &project.0,
            Path::new("docs/intro.md"),
            text,
            &history,
            &scripts,
        )
        .into_iter()
        .map(|m| (m.name, m.fate))
        .collect()
    }

    #[test]
    fn removed_paths_carry_what_git_shows() {
        assert_eq!(
            found("Edit `src/old.ts` and `src/moved.ts:12`; create `out.json`."),
            [
                ("src/old.ts".to_string(), Fate::Deleted),
                (
                    "src/moved.ts".into(),
                    Fate::Renamed(PathBuf::from("src/new.ts"))
                ),
                ("out.json".into(), Fate::Absent),
            ]
        );
    }

    #[test]
    fn present_partial_and_relative_paths_are_not_missing() {
        assert!(
            found("See `src/app.ts`, `services/quota.ts` and [the guide](guide.md).").is_empty()
        );
    }

    #[test]
    fn routes_branches_urls_and_globs_are_not_paths() {
        assert!(
            found(
                "Open `/login`, merge `origin/main`, fetch `https://x.io/a.ts`, match `src/*.ts`."
            )
            .is_empty()
        );
    }

    #[test]
    fn undeclared_scripts_in_code_are_missing() {
        let text =
            "```sh\npnpm quality\npnpm run lint\nnpm install\n```\nIn prose, make the build.";
        assert_eq!(found(text), [("lint".to_string(), Fate::NoScript)]);
    }
}
