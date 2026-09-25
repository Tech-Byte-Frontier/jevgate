//! Finding documentation files. Agent instruction files are found even when
//! hidden or ignored, since harnesses load local-only files too; project docs
//! follow the ignore files like source does.
use crate::discovery::SKIPPED_DIRS;
use anyhow::{Context, Result};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

/// Instruction file names read wherever they appear.
pub const AGENT_NAMES: &[&str] = &[
    "AGENTS.md",
    "AGENTS.override.md",
    "CLAUDE.md",
    "CLAUDE.local.md",
    "GEMINI.md",
    ".cursorrules",
    ".windsurfrules",
    ".clinerules",
    ".roorules",
];

/// Hidden directories that hold instruction files.
const AGENT_DIRS: &[&str] = &[
    ".claude",
    ".cursor",
    ".github",
    ".windsurf",
    ".devin",
    ".clinerules",
    ".kiro",
    ".junie",
    ".roo",
];

/// Markdown that records history or legal terms rather than usage.
const RECORD_STEMS: &[&str] = &[
    "changelog",
    "changes",
    "history",
    "news",
    "releases",
    "release-notes",
    "release_notes",
    "releasenotes",
    "license",
    "licence",
    "notice",
    "code_of_conduct",
    "authors",
    "contributors",
    "copying",
];

#[derive(Debug, Default)]
pub struct Found {
    /// Agent instruction files, as regular files.
    pub agent: BTreeSet<PathBuf>,
    /// Agent instruction paths that are symbolic links, with their target
    /// inside the repository when it resolves there.
    pub links: Vec<(PathBuf, Option<PathBuf>)>,
    pub project: BTreeSet<PathBuf>,
    /// Directories visited under the ignore files, relative to the root.
    pub directories: BTreeSet<PathBuf>,
}

pub fn agent_file(path: &Path) -> bool {
    let name = file_name(path);
    let parts: Vec<&str> = path.iter().filter_map(|p| p.to_str()).collect();
    let under = |a: &str, b: &str| parts.windows(2).any(|w| w == [a, b]);
    let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    AGENT_NAMES.contains(&name)
        || (under(".claude", "rules") && extension == "md")
        || (under(".cursor", "rules") && matches!(extension, "mdc" | "md"))
        || (parts.len() >= 2
            && parts[parts.len() - 2] == ".github"
            && name == "copilot-instructions.md")
        || (under(".github", "instructions") && name.ends_with(".instructions.md"))
        || ((under(".windsurf", "rules") || under(".devin", "rules")) && extension == "md")
        || (parts.contains(&".clinerules") && matches!(extension, "md" | "txt"))
        || (under(".kiro", "steering") && extension == "md")
        || (parts.first() == Some(&".junie")
            && (matches!(name, "AGENTS.md" | "guidelines.md" | "playbook.md")
                || ((parts.get(1) == Some(&"rules") || parts.get(1) == Some(&"guidelines"))
                    && extension == "md")))
        || (parts.first() == Some(&".roo")
            && parts
                .get(1)
                .is_some_and(|f| *f == "rules" || f.starts_with("rules-"))
            && matches!(extension, "md" | "txt"))
        || (parts.len() == 1 && name.starts_with(".roorules-"))
}

/// Project documentation: Markdown, MDX, reStructuredText or AsciiDoc at
/// the root, README and CONTRIBUTING files anywhere, and those formats under
/// `docs/` or `doc/`, outside records such as changelogs and Sphinx output.
fn project_doc(path: &Path) -> bool {
    let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let lower = |p: &std::ffi::OsStr| p.to_string_lossy().to_ascii_lowercase();
    let stem = path.file_stem().map(lower).unwrap_or_default();
    let dirs: Vec<String> = path
        .parent()
        .into_iter()
        .flat_map(|p| p.iter())
        .map(lower)
        .collect();
    let hidden = path.iter().any(|p| p.to_string_lossy().starts_with('.'));
    let excluded = dirs.iter().any(|d| {
        RECORD_STEMS.contains(&d.as_str())
            || matches!(
                d.as_str(),
                "fixtures" | "__fixtures__" | "testdata" | "__snapshots__" | "archive" | "_build"
            )
    });
    let documentation = dirs.is_empty()
        || stem.starts_with("readme")
        || stem.starts_with("contributing")
        || dirs.iter().any(|d| DOC_DIRS.contains(&d.as_str()));
    super::format::EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str())
        && documentation
        && !agent_file(path)
        && !hidden
        && !excluded
        && !RECORD_STEMS.contains(&stem.as_str())
}

fn file_name(path: &Path) -> &str {
    path.file_name().and_then(|n| n.to_str()).unwrap_or("")
}

pub fn discover(root: &Path) -> Result<Found> {
    let mut found = Found::default();
    let visible = ignore::WalkBuilder::new(root)
        .standard_filters(true)
        .hidden(false)
        .parents(false)
        .require_git(false)
        .follow_links(false)
        .filter_entry(|e| {
            let name = e.file_name().to_str().unwrap_or_default();
            e.depth() == 0
                || !e.file_type().is_some_and(|t| t.is_dir())
                || !(SKIPPED_DIRS.contains(&name)
                    || (name.starts_with('.') && !AGENT_DIRS.contains(&name)))
        })
        .build();
    for entry in visible {
        let entry = entry.context("Failed while discovering documentation")?;
        let relative = crate::discovery::relative(entry.path(), root)?;
        if entry.file_type().is_some_and(|t| t.is_dir()) {
            found.directories.insert(relative);
        } else if agent_file(&relative) {
            add_agent(root, relative, &mut found);
        } else if project_doc(&relative) {
            found.project.insert(relative);
        }
    }
    // Ignored instruction files next to visible ones still load, so probe for
    // them by name and walk the agent directories without ignore files.
    for directory in found.directories.clone() {
        for name in AGENT_NAMES {
            let path = directory.join(name);
            if root
                .join(&path)
                .symlink_metadata()
                .is_ok_and(|m| !m.is_dir())
            {
                add_agent(root, path, &mut found);
            }
        }
        for name in AGENT_DIRS {
            let path = root.join(&directory).join(name);
            if path.is_dir() {
                walk_agent_dir(root, &path, &mut found)?;
            }
        }
        // Documentation folders kept out of Git are still read by agents on request.
        for name in DOC_DIRS {
            let path = directory.join(name);
            if root.join(&path).is_dir() {
                walk_doc_dir(root, &root.join(&path), &mut found)?;
            }
        }
    }
    Ok(found)
}

/// Documentation folders read even when ignored.
const DOC_DIRS: &[&str] = &["docs", "doc"];

fn walk_doc_dir(root: &Path, dir: &Path, found: &mut Found) -> Result<()> {
    let walk = ignore::WalkBuilder::new(dir)
        .standard_filters(false)
        .follow_links(false)
        .filter_entry(|e| {
            let name = e.file_name().to_str().unwrap_or_default();
            e.depth() == 0
                || !e.file_type().is_some_and(|t| t.is_dir())
                || !(SKIPPED_DIRS.contains(&name) || name.starts_with('.'))
        })
        .build();
    for entry in walk {
        let entry = entry.context("Failed while discovering documentation")?;
        let relative = crate::discovery::relative(entry.path(), root)?;
        if entry.file_type().is_some_and(|t| t.is_file()) && project_doc(&relative) {
            found.project.insert(relative);
        }
    }
    Ok(())
}

/// Rule folders nest a few levels at most; deeper trees are not rules.
const AGENT_DIR_DEPTH: usize = 6;

fn walk_agent_dir(root: &Path, dir: &Path, found: &mut Found) -> Result<()> {
    for entry in ignore::WalkBuilder::new(dir)
        .standard_filters(false)
        .follow_links(false)
        .max_depth(Some(AGENT_DIR_DEPTH))
        .build()
    {
        let entry = entry.context("Failed while discovering agent instructions")?;
        let relative = crate::discovery::relative(entry.path(), root)?;
        if !entry.file_type().is_some_and(|t| t.is_dir()) && agent_file(&relative) {
            add_agent(root, relative, found);
        }
    }
    Ok(())
}

fn add_agent(root: &Path, relative: PathBuf, found: &mut Found) {
    let path = root.join(&relative);
    let Ok(metadata) = path.symlink_metadata() else {
        return;
    };
    if metadata.file_type().is_symlink() {
        if found.links.iter().any(|(p, _)| *p == relative) {
            return;
        }
        let target = path
            .canonicalize()
            .ok()
            .and_then(|t| crate::discovery::relative(&t, root).ok());
        found.links.push((relative, target));
    } else if metadata.is_file() {
        found.agent.insert(relative);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_and_project_paths() {
        for path in [
            "AGENTS.md",
            "web/CLAUDE.md",
            ".claude/CLAUDE.md",
            ".claude/rules/testing.md",
            ".cursor/rules/style.mdc",
            ".github/copilot-instructions.md",
            ".github/instructions/api.instructions.md",
            ".windsurf/rules/a.md",
            ".clinerules/b.md",
            ".cursorrules",
            ".kiro/steering/tech.md",
            ".junie/guidelines.md",
            ".junie/rules/style.md",
            ".roo/rules/01-general.md",
            ".roo/rules-code/testing.txt",
            ".roorules",
            ".roorules-architect",
        ] {
            assert!(agent_file(Path::new(path)), "{path}");
        }
        assert!(!agent_file(Path::new(".github/workflows/ci.md")));
        assert!(!agent_file(Path::new(".kiro/specs/login/design.md")));
        assert!(!agent_file(Path::new(".roo/mcp.json")));
    }

    #[test]
    fn project_docs_are_root_readme_and_doc_folders_without_records() {
        for (path, expected) in [
            ("README.md", true),
            ("ROADMAP.md", true),
            ("web/README.md", true),
            ("docs/guide.md", true),
            ("doc/api/errors.mdx", true),
            ("CHANGELOG.md", false),
            ("release-notes.md", false),
            ("docs/releases/0.4.md", false),
            ("CLAUDE.md", false),
            ("src/NOTES.md", false),
            ("config/notes/SAUD3.md", false),
            (".github/ISSUE_TEMPLATE/bug.md", false),
            ("tests/fixtures/readme.md", false),
            ("docs/changelog/v1.md", false),
            ("docs/archive/plan.md", false),
        ] {
            assert_eq!(project_doc(Path::new(path)), expected, "{path}");
        }
    }

    #[test]
    fn ignored_and_hidden_agent_files_are_found() {
        let project = crate::tests::Project::new();
        for (path, text) in [
            (".gitignore", "AGENTS.md\n/.claude/\n/notes/\n/docs/*\n"),
            ("docs/plan.md", "# Plan\n"),
            ("AGENTS.md", "# A\n"),
            ("README.md", "# R\n"),
            (".claude/rules/x.md", "# X\n"),
            ("notes/n.md", "# N\n"),
            ("src/CLAUDE.md", "# C\n"),
            ("node_modules/p/README.md", "# P\n"),
        ] {
            project.write(path, text);
        }
        let found = discover(&project.0).unwrap();
        let agent: Vec<_> = found.agent.iter().map(|p| p.to_str().unwrap()).collect();
        assert_eq!(agent, [".claude/rules/x.md", "AGENTS.md", "src/CLAUDE.md"]);
        let docs: Vec<_> = found.project.iter().map(|p| p.to_str().unwrap()).collect();
        assert_eq!(docs, ["README.md", "docs/plan.md"]);
    }
}
