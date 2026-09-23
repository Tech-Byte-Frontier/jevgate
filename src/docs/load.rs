//! When each harness loads each instruction file, following its documented
//! discovery rules, and what a session started at the repository root costs.
//! These are facts about loading, reported as evidence; they never decide a
//! finding.
use super::markdown::{self, Markdown};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

/// When a harness adds a file to its context.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "when", content = "scope")]
pub enum Load {
    /// At the start of every session in the repository.
    Always,
    /// When the agent works in this directory.
    Directory(String),
    /// When the agent works on files matching these patterns.
    Files(String),
    /// When the agent decides the rule's description is relevant.
    Requested,
    /// Only when a person names it.
    Manual,
}

impl Load {
    pub fn phrase(&self) -> String {
        match self {
            Self::Always => "at the start of every session".into(),
            Self::Directory(dir) => format!("when the agent works in `{dir}/`"),
            Self::Files(globs) => format!("when the agent works on files matching `{globs}`"),
            Self::Requested => "when the agent decides its description is relevant".into(),
            Self::Manual => "only when a person names it".into(),
        }
    }
}

/// One harness reading one file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reader {
    pub harness: String,
    #[serde(flatten)]
    pub load: Load,
}

/// A loading fact with its fix, such as a copy or an import that does not resolve.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fact {
    pub path: PathBuf,
    pub line: usize,
    pub message: String,
}

/// What one harness loads at the start of a session at the repository root.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HarnessLoad {
    pub harness: String,
    pub files: Vec<PathBuf>,
    /// Estimated at four bytes per token, including imported files.
    pub estimated_tokens: usize,
    /// Files loaded later, on demand.
    pub on_demand_files: usize,
    pub on_demand_tokens: usize,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ContextLoad {
    pub harnesses: Vec<HarnessLoad>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub facts: Vec<Fact>,
}

pub const CLAUDE: &str = "Claude Code";
const CODEX: &str = "Codex";
const GEMINI: &str = "Gemini CLI";
const COPILOT: &str = "GitHub Copilot";
const CURSOR: &str = "Cursor";
const WINDSURF: &str = "Windsurf";
const CLINE: &str = "Cline";

/// Codex stops adding project instructions at this size (`project_doc_max_bytes`).
const CODEX_MAX_BYTES: usize = 32 * 1024;
/// Windsurf drops workspace rule text past this many characters.
const WINDSURF_MAX_CHARS: usize = 12_000;
/// Claude Code follows imports this many hops deep.
const IMPORT_DEPTH: usize = 4;

/// One instruction file: its text as read and who reads it when.
pub struct File {
    pub path: PathBuf,
    pub source: String,
    pub readers: Vec<Reader>,
}

/// Every harness that reads `path`, and when.
pub fn readers(path: &Path, markdown: &Markdown, present: &dyn Fn(&str) -> bool) -> Vec<Reader> {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let dir = path.parent().unwrap_or(Path::new(""));
    let parts: Vec<&str> = path.iter().filter_map(|p| p.to_str()).collect();
    let under = |a: &str, b: &str| parts.windows(2).any(|w| w == [a, b]);
    let nested = |base: &Path| match base.to_str() {
        Some("") | None => Load::Always,
        Some(dir) => Load::Directory(dir.to_string()),
    };
    let front = |key: &str| markdown.frontmatter.get(key).filter(|v| !v.is_empty());
    let scoped = |key: &str| match front(key) {
        Some(globs) if !matches!(globs.as_str(), "**" | "**/*") => Load::Files(globs.clone()),
        _ => Load::Always,
    };
    let at = |base: &Path, file: &str| present(&base.join(file).to_string_lossy());
    let read = |harnesses: &[&str], load: Load| {
        harnesses
            .iter()
            .map(|h| Reader {
                harness: (*h).into(),
                load: load.clone(),
            })
            .collect::<Vec<_>>()
    };
    let claude_dir = dir.file_name().is_some_and(|n| n == ".claude");
    match name {
        "AGENTS.md" => {
            let mut readers = read(&[CODEX, COPILOT, CURSOR, WINDSURF, CLINE], nested(dir));
            // Claude Code reads AGENTS.md only where no CLAUDE.md exists.
            if !["CLAUDE.md", "CLAUDE.local.md", ".claude/CLAUDE.md"]
                .iter()
                .any(|f| at(dir, f))
            {
                readers.extend(read(&[CLAUDE], nested(dir)));
            }
            readers
        }
        "AGENTS.override.md" => read(&[CODEX], nested(dir)),
        "CLAUDE.md" if claude_dir => read(&[CLAUDE], nested(dir.parent().unwrap_or(dir))),
        "CLAUDE.md" | "CLAUDE.local.md" => read(&[CLAUDE], nested(dir)),
        "GEMINI.md" => read(&[GEMINI], nested(dir)),
        ".cursorrules" => read(&[CURSOR, CLINE], Load::Always),
        ".windsurfrules" => read(&[WINDSURF, CLINE], Load::Always),
        ".clinerules" => read(&[CLINE], Load::Always),
        "copilot-instructions.md" => read(&[COPILOT], Load::Always),
        _ if under(".claude", "rules") => read(&[CLAUDE], scoped("paths")),
        _ if under(".github", "instructions") => match front("applyTo") {
            Some(_) => read(&[COPILOT], scoped("applyTo")),
            None => read(&[COPILOT], Load::Manual),
        },
        _ if under(".cursor", "rules") => {
            if !name.ends_with(".mdc") {
                Vec::new()
            } else if front("alwaysApply").is_some_and(|v| v == "true") {
                read(&[CURSOR], Load::Always)
            } else if let Some(globs) = front("globs") {
                read(&[CURSOR], Load::Files(globs.clone()))
            } else if front("description").is_some() {
                read(&[CURSOR], Load::Requested)
            } else {
                read(&[CURSOR], Load::Manual)
            }
        }
        _ if under(".windsurf", "rules") || under(".devin", "rules") => {
            match front("trigger").map(String::as_str) {
                Some("always_on") => read(&[WINDSURF], Load::Always),
                Some("glob") => read(&[WINDSURF], scoped("globs")),
                Some("manual") => read(&[WINDSURF], Load::Manual),
                _ => read(&[WINDSURF], Load::Requested),
            }
        }
        _ if parts.contains(&".clinerules") => read(&[CLINE], scoped("paths")),
        _ => Vec::new(),
    }
}

/// Parse each file and decide who reads it; `sources` maps each regular
/// instruction file to its text.
pub fn files(
    sources: BTreeMap<PathBuf, String>,
    links: &[(PathBuf, Option<PathBuf>)],
) -> Vec<File> {
    let mut present: Vec<String> = sources
        .keys()
        .chain(links.iter().map(|(p, _)| p))
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    present.sort();
    let exists = |p: &str| present.binary_search_by(|x| x.as_str().cmp(p)).is_ok();
    let mut files: Vec<File> = sources
        .iter()
        .map(|(path, source)| File {
            readers: readers(path, &markdown::parse(source), &exists),
            path: path.clone(),
            source: source.clone(),
        })
        .collect();
    // A link loads its target's text under its own name.
    for (link, target) in links {
        let Some(source) = target.as_ref().and_then(|t| sources.get(t)) else {
            continue;
        };
        files.push(File {
            readers: readers(link, &markdown::parse(source), &exists),
            path: link.clone(),
            source: source.clone(),
        });
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    files
}

/// Session-start cost per harness and the loading facts.
pub fn context_load(
    files: &[File],
    links: &[(PathBuf, Option<PathBuf>)],
    root: &Path,
) -> ContextLoad {
    let by_path: BTreeMap<&Path, &File> = files.iter().map(|f| (f.path.as_path(), f)).collect();
    let mut harnesses = BTreeMap::<String, HarnessLoad>::new();
    for file in files {
        for reader in &file.readers {
            let entry = harnesses
                .entry(reader.harness.clone())
                .or_insert_with(|| HarnessLoad {
                    harness: reader.harness.clone(),
                    files: Vec::new(),
                    estimated_tokens: 0,
                    on_demand_files: 0,
                    on_demand_tokens: 0,
                });
            let tokens = loaded_tokens(file, &reader.harness, root);
            match reader.load {
                Load::Always => {
                    entry.files.push(file.path.clone());
                    entry.estimated_tokens += tokens;
                }
                Load::Manual => {}
                _ => {
                    entry.on_demand_files += 1;
                    entry.on_demand_tokens += tokens;
                }
            }
        }
    }
    let mut facts = Vec::new();
    copies(files, links, &mut facts);
    for file in files {
        file_facts(file, &by_path, root, &mut facts);
    }
    codex_cap(files, &mut facts);
    cline_duplicates(files, &mut facts);
    ContextLoad {
        harnesses: harnesses.into_values().collect(),
        facts,
    }
}

/// Tokens one harness spends on a file: Claude Code drops HTML comments and
/// adds the files it imports.
fn loaded_tokens(file: &File, harness: &str, root: &Path) -> usize {
    if harness != CLAUDE {
        return markdown::tokens(&file.source);
    }
    let mut total = 0;
    let mut pending = vec![(root.join(&file.path), file.source.clone(), 0)];
    let mut seen = std::collections::BTreeSet::new();
    while let Some((path, source, depth)) = pending.pop() {
        total += markdown::tokens(&markdown::strip_comments(&source));
        if depth == IMPORT_DEPTH {
            continue;
        }
        for (_, import) in markdown::imports(&source) {
            let Some(target) = resolve(&path, &import, root) else {
                continue;
            };
            if seen.insert(target.clone())
                && let Ok(text) = std::fs::read_to_string(&target)
            {
                pending.push((target, text, depth + 1));
            }
        }
    }
    total
}

/// An import relative to the importing file, inside the repository.
fn resolve(from: &Path, import: &str, root: &Path) -> Option<PathBuf> {
    if import.starts_with('~') || import.starts_with('/') {
        return None;
    }
    let target = from.parent()?.join(import).canonicalize().ok()?;
    let root = root.canonicalize().ok()?;
    (target.starts_with(&root) && target.is_file()).then_some(target)
}

/// Byte-identical instruction files that are not links to each other.
fn copies(files: &[File], links: &[(PathBuf, Option<PathBuf>)], facts: &mut Vec<Fact>) {
    let linked = |path: &Path| links.iter().any(|(l, _)| l == path);
    let mut groups = BTreeMap::<&str, Vec<&Path>>::new();
    for file in files.iter().filter(|f| !linked(&f.path)) {
        groups
            .entry(file.source.trim())
            .or_default()
            .push(&file.path);
    }
    for paths in groups.values().filter(|p| p.len() > 1) {
        let names: Vec<String> = paths.iter().map(|p| format!("`{}`", p.display())).collect();
        facts.push(Fact {
            path: paths[0].to_path_buf(),
            line: 1,
            message: format!(
                "{} are identical copies. Keep one, and import it (`@{}`) or link to it from the others, so the copies cannot drift apart.",
                names.join(" and "),
                paths[0].display()
            ),
        });
    }
}

fn file_facts(file: &File, by_path: &BTreeMap<&Path, &File>, root: &Path, facts: &mut Vec<Fact>) {
    let path = &file.path;
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let in_cursor_rules = path
        .iter()
        .collect::<Vec<_>>()
        .windows(2)
        .any(|w| w[0] == ".cursor" && w[1] == "rules");
    if in_cursor_rules && !name.ends_with(".mdc") {
        facts.push(Fact {
            path: path.clone(),
            line: 1,
            message: "Cursor reads only `.mdc` files in `.cursor/rules`, so this file is never loaded. Rename it to `.mdc` with frontmatter, or remove it.".into(),
        });
    }
    if file.readers.iter().any(|r| r.harness == CLAUDE) {
        for (line, import) in markdown::imports(&file.source) {
            if !import.starts_with('~') && resolve(&root.join(path), &import, root).is_none() {
                facts.push(Fact {
                    path: path.clone(),
                    line,
                    message: format!(
                        "The import `@{import}` does not resolve to a file in the repository."
                    ),
                });
            }
        }
    }
    if name == "CLAUDE.md" {
        let dir = path.parent().unwrap_or(Path::new(""));
        let agents = dir.join("AGENTS.md");
        let imported = markdown::imports(&file.source)
            .iter()
            .any(|(_, i)| i.trim_start_matches("./") == "AGENTS.md");
        // A link or an identical copy already gives Claude the same text.
        let same = by_path
            .get(agents.as_path())
            .is_some_and(|other| other.source.trim() == file.source.trim());
        if by_path.contains_key(agents.as_path()) && !imported && !same {
            facts.push(Fact {
                path: agents,
                line: 1,
                message: format!(
                    "Claude Code reads `{}` here and skips this AGENTS.md. Add `@AGENTS.md` to it if Claude should follow these instructions too.",
                    path.display()
                ),
            });
        }
    }
    let windsurf = file.readers.iter().any(|r| r.harness == WINDSURF) && name != "AGENTS.md";
    let chars = file.source.chars().count();
    if windsurf && chars > WINDSURF_MAX_CHARS {
        facts.push(Fact {
            path: path.clone(),
            line: 1,
            message: format!(
                "Windsurf reads the first {WINDSURF_MAX_CHARS} characters of a workspace rule; {} of this file's {chars} are dropped.",
                chars - WINDSURF_MAX_CHARS
            ),
        });
    }
}

/// Codex joins the root instruction files and stops at its byte limit.
fn codex_cap(files: &[File], facts: &mut Vec<Fact>) {
    let root: Vec<&File> = files
        .iter()
        .filter(|f| {
            f.readers
                .iter()
                .any(|r| r.harness == CODEX && r.load == Load::Always)
        })
        .collect();
    let bytes: usize = root.iter().map(|f| f.source.len()).sum();
    if let Some(first) = root.first()
        && bytes > CODEX_MAX_BYTES
    {
        facts.push(Fact {
            path: first.path.clone(),
            line: 1,
            message: format!(
                "Codex stops reading project instructions at {CODEX_MAX_BYTES} bytes by default (`project_doc_max_bytes`); {} of these {bytes} bytes are not loaded.",
                bytes - CODEX_MAX_BYTES
            ),
        });
    }
}

/// Cline loads every rule file it recognizes, so each one adds to the session.
fn cline_duplicates(files: &[File], facts: &mut Vec<Fact>) {
    let always: Vec<&File> = files
        .iter()
        .filter(|f| {
            f.path.parent() == Some(Path::new(""))
                && f.readers
                    .iter()
                    .any(|r| r.harness == CLINE && r.load == Load::Always)
        })
        .collect();
    if always.len() > 1 {
        let names: Vec<String> = always
            .iter()
            .map(|f| format!("`{}`", f.path.display()))
            .collect();
        facts.push(Fact {
            path: always[0].path.clone(),
            line: 1,
            message: format!(
                "Cline loads each of {} at the start of every session.",
                names.join(", ")
            ),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loads(path: &str, source: &str, present: &[&str]) -> Vec<(String, Load)> {
        readers(Path::new(path), &markdown::parse(source), &|p| {
            present.contains(&p)
        })
        .into_iter()
        .map(|r| (r.harness, r.load))
        .collect()
    }

    #[test]
    fn harness_readers_follow_their_discovery_rules() {
        let agents = loads("AGENTS.md", "", &["AGENTS.md", "CLAUDE.md"]);
        assert!(
            agents
                .iter()
                .all(|(h, l)| h != CLAUDE && *l == Load::Always)
        );
        assert!(
            loads("AGENTS.md", "", &["AGENTS.md"])
                .iter()
                .any(|(h, _)| h == CLAUDE)
        );
        assert_eq!(
            loads("web/CLAUDE.md", "", &[]),
            [(CLAUDE.to_string(), Load::Directory("web".into()))]
        );
        assert_eq!(
            loads(".claude/CLAUDE.md", "", &[]),
            [(CLAUDE.to_string(), Load::Always)]
        );
        assert_eq!(
            loads(".claude/rules/api.md", "---\npaths: src/api/**\n---\n", &[]),
            [(CLAUDE.to_string(), Load::Files("src/api/**".into()))]
        );
        assert_eq!(
            loads(".cursor/rules/a.mdc", "---\nalwaysApply: true\n---\n", &[]),
            [(CURSOR.to_string(), Load::Always)]
        );
        assert_eq!(
            loads(".cursor/rules/a.mdc", "---\ndescription: API\n---\n", &[]),
            [(CURSOR.to_string(), Load::Requested)]
        );
        assert!(loads(".cursor/rules/a.md", "", &[]).is_empty());
        assert_eq!(
            loads(
                ".github/instructions/a.instructions.md",
                "---\napplyTo: \"**/*.ts\"\n---\n",
                &[]
            ),
            [(COPILOT.to_string(), Load::Files("**/*.ts".into()))]
        );
    }

    #[test]
    fn session_cost_and_loading_facts() {
        let project = crate::tests::Project::new();
        let big = "x".repeat(CODEX_MAX_BYTES);
        let sources: BTreeMap<PathBuf, String> = [
            ("AGENTS.md", big.as_str()),
            (
                "CLAUDE.md",
                "# Rules\nSee @docs/missing.md\n<!-- note for people -->\n",
            ),
            (
                "GEMINI.md",
                "# Rules\nSee @docs/missing.md\n<!-- note for people -->\n",
            ),
            (".cursor/rules/style.md", "# Style\n"),
        ]
        .into_iter()
        .map(|(p, s)| {
            project.write(p, s);
            (PathBuf::from(p), s.to_string())
        })
        .collect();
        let files = files(sources, &[]);
        let load = context_load(&files, &[], &project.0);
        let codex = load.harnesses.iter().find(|h| h.harness == CODEX).unwrap();
        assert_eq!(codex.estimated_tokens, CODEX_MAX_BYTES / 4);
        let claude = load.harnesses.iter().find(|h| h.harness == CLAUDE).unwrap();
        assert_eq!(claude.files, [PathBuf::from("CLAUDE.md")]);
        assert_eq!(
            claude.estimated_tokens,
            markdown::tokens("# Rules\nSee @docs/missing.md\n\n")
        );
        let messages: Vec<&str> = load.facts.iter().map(|f| f.message.as_str()).collect();
        assert!(messages[0].contains("`CLAUDE.md` and `GEMINI.md` are identical copies"));
        assert!(
            messages
                .iter()
                .any(|m| m.contains("`@docs/missing.md` does not resolve"))
        );
        assert!(
            messages
                .iter()
                .any(|m| m.contains("Cursor reads only `.mdc`"))
        );
        assert!(messages.iter().any(|m| m.contains("skips this AGENTS.md")));
        assert!(!messages.iter().any(|m| m.contains("Codex stops reading")));
        let bigger = format!("{big}y");
        let files = super::files([(PathBuf::from("AGENTS.md"), bigger)].into(), &[]);
        let load = context_load(&files, &[], &project.0);
        assert!(
            load.facts[0]
                .message
                .contains("1 of these 32769 bytes are not loaded")
        );
    }

    #[test]
    fn a_linked_claude_file_reads_the_same_instructions() {
        let project = crate::tests::Project::new();
        let text = "# Build\nRun `make`.\n";
        project.write("AGENTS.md", text);
        let links = [(PathBuf::from("CLAUDE.md"), Some(PathBuf::from("AGENTS.md")))];
        let files = files(
            [(PathBuf::from("AGENTS.md"), text.to_string())].into(),
            &links,
        );
        let claude: Vec<_> = files
            .iter()
            .flat_map(|f| f.readers.iter().map(move |r| (&f.path, r)))
            .filter(|(_, r)| r.harness == CLAUDE)
            .map(|(p, _)| p.to_str().unwrap())
            .collect();
        assert_eq!(claude, ["CLAUDE.md"]);
        assert!(context_load(&files, &links, &project.0).facts.is_empty());
    }
}
