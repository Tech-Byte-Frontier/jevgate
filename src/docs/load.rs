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
    /// When the agent works in this mode, such as Roo Code's `code` mode.
    Mode(String),
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
            Self::Mode(mode) => format!("when the agent works in its `{mode}` mode"),
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
const KIRO: &str = "Kiro";
const JUNIE: &str = "Junie";
const ROO: &str = "Roo Code";

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
    let claude_dir = dir.file_name().is_some_and(|n| n == ".claude");
    match name {
        "AGENTS.md" => agents_readers(dir, present),
        "AGENTS.override.md" => read(&[CODEX], nested(dir)),
        "CLAUDE.md" if claude_dir => read(&[CLAUDE], nested(dir.parent().unwrap_or(dir))),
        "CLAUDE.md" | "CLAUDE.local.md" => read(&[CLAUDE], nested(dir)),
        "GEMINI.md" => read(&[GEMINI], nested(dir)),
        ".cursorrules" => read(&[CURSOR, CLINE], Load::Always),
        ".windsurfrules" => read(&[WINDSURF, CLINE], Load::Always),
        ".clinerules" => read(&[CLINE], Load::Always),
        "copilot-instructions.md" => read(&[COPILOT], Load::Always),
        _ if under(".claude", "rules") => read(&[CLAUDE], scoped(markdown, "paths")),
        _ if under(".github", "instructions") => match front(markdown, "applyTo") {
            Some(_) => read(&[COPILOT], scoped(markdown, "applyTo")),
            None => read(&[COPILOT], Load::Manual),
        },
        _ if under(".cursor", "rules") => cursor_readers(name, markdown),
        _ if under(".windsurf", "rules") || under(".devin", "rules") => windsurf_readers(markdown),
        _ if parts.contains(&".clinerules") => read(&[CLINE], scoped(markdown, "paths")),
        _ if under(".kiro", "steering") => kiro_readers(markdown),
        _ if parts.first() == Some(&".junie") => junie_readers(&parts, present),
        _ if parts.first() == Some(&".roo") || name.starts_with(".roorules") => {
            roo_readers(&parts, name, present)
        }
        _ => Vec::new(),
    }
}

/// Kiro steering files load by their `inclusion` frontmatter, always by default.
fn kiro_readers(markdown: &Markdown) -> Vec<Reader> {
    match front(markdown, "inclusion").map(String::as_str) {
        Some("fileMatch") => read(&[KIRO], scoped(markdown, "fileMatchPattern")),
        Some("manual") => read(&[KIRO], Load::Manual),
        Some("auto") => read(&[KIRO], Load::Requested),
        _ => read(&[KIRO], Load::Always),
    }
}

/// Junie reads the first of `.junie/AGENTS.md`; the root `AGENTS.md` with
/// `.junie/playbook.md` and `.junie/rules/*.md`; or its legacy
/// `.junie/guidelines.md` or `.junie/guidelines/`.
fn junie_readers(parts: &[&str], present: &dyn Fn(&str) -> bool) -> Vec<Reader> {
    let own = present(".junie/AGENTS.md");
    let chosen = match parts {
        [_, "AGENTS.md"] => true,
        [_, "playbook.md"] | [_, "rules", ..] => !own,
        [_, "guidelines.md"] | [_, "guidelines", ..] => !own && !present("AGENTS.md"),
        _ => false,
    };
    if chosen {
        read(&[JUNIE], Load::Always)
    } else {
        Vec::new()
    }
}

/// Roo Code reads every file under `.roo/rules/`, and `.roo/rules-{mode}/`
/// in that mode; a root `.roorules` or `.roorules-{mode}` file only when the
/// matching folder is missing or empty.
fn roo_readers(parts: &[&str], name: &str, present: &dyn Fn(&str) -> bool) -> Vec<Reader> {
    let load = |folder: &str| match folder.strip_prefix("rules-") {
        Some(mode) => Load::Mode(mode.to_string()),
        None => Load::Always,
    };
    match parts {
        [".roo", folder, _, ..] if *folder == "rules" || folder.starts_with("rules-") => {
            read(&[ROO], load(folder))
        }
        [_] => {
            let folder = name.trim_start_matches(".roo");
            if present(&format!(".roo/{folder}/")) {
                Vec::new()
            } else {
                read(&[ROO], load(folder))
            }
        }
        _ => Vec::new(),
    }
}

/// Every harness reading one load.
fn read(harnesses: &[&str], load: Load) -> Vec<Reader> {
    harnesses
        .iter()
        .map(|h| Reader {
            harness: (*h).into(),
            load: load.clone(),
        })
        .collect()
}

/// Loaded at session start at the root, else when working in the directory.
fn nested(dir: &Path) -> Load {
    match dir.to_str() {
        Some("") | None => Load::Always,
        Some(dir) => Load::Directory(dir.to_string()),
    }
}

fn front<'a>(markdown: &'a Markdown, key: &str) -> Option<&'a String> {
    markdown.frontmatter.get(key).filter(|v| !v.is_empty())
}

/// Loaded for the files a frontmatter key's patterns match, or always.
fn scoped(markdown: &Markdown, key: &str) -> Load {
    match front(markdown, key) {
        Some(globs) if !matches!(globs.as_str(), "**" | "**/*") => Load::Files(globs.clone()),
        _ => Load::Always,
    }
}

/// Every AGENTS.md reader; Claude Code reads it only where no CLAUDE.md exists.
fn agents_readers(dir: &Path, present: &dyn Fn(&str) -> bool) -> Vec<Reader> {
    let mut readers = read(&[CODEX, COPILOT, CURSOR, WINDSURF, CLINE], nested(dir));
    let claude_file = ["CLAUDE.md", "CLAUDE.local.md", ".claude/CLAUDE.md"]
        .iter()
        .any(|f| present(&dir.join(f).to_string_lossy()));
    if !claude_file {
        readers.extend(read(&[CLAUDE], nested(dir)));
    }
    readers
}

/// Cursor reads only `.mdc` rules, by their frontmatter.
fn cursor_readers(name: &str, markdown: &Markdown) -> Vec<Reader> {
    if !name.ends_with(".mdc") {
        Vec::new()
    } else if front(markdown, "alwaysApply").is_some_and(|v| v == "true") {
        read(&[CURSOR], Load::Always)
    } else if let Some(globs) = front(markdown, "globs") {
        read(&[CURSOR], Load::Files(globs.clone()))
    } else if front(markdown, "description").is_some() {
        read(&[CURSOR], Load::Requested)
    } else {
        read(&[CURSOR], Load::Manual)
    }
}

fn windsurf_readers(markdown: &Markdown) -> Vec<Reader> {
    match front(markdown, "trigger").map(String::as_str) {
        Some("always_on") => read(&[WINDSURF], Load::Always),
        Some("glob") => read(&[WINDSURF], scoped(markdown, "globs")),
        Some("manual") => read(&[WINDSURF], Load::Manual),
        _ => read(&[WINDSURF], Load::Requested),
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
    // A path ending in `/` asks whether any file is inside that folder.
    let exists = |p: &str| match p.ends_with('/') {
        true => present.iter().any(|x| x.starts_with(p)),
        false => present.binary_search_by(|x| x.as_str().cmp(p)).is_ok(),
    };
    let mut files: Vec<File> = sources
        .iter()
        .map(|(path, source)| File {
            readers: readers(path, &markdown::parse(source), &exists),
            path: path.clone(),
            source: source.clone(),
        })
        .collect();
    // A link loads its target's text under its own name. A target that no
    // harness reads by its own name, such as refined-github's `agents.md`
    // behind its `CLAUDE.md`, is read through the link instead: it takes the
    // link's readers, so its findings name the file itself and its text is
    // counted once.
    for (link, target) in links {
        let Some((target, source)) = target.as_ref().and_then(|t| sources.get_key_value(t)) else {
            continue;
        };
        let linked = readers(link, &markdown::parse(source), &exists);
        if let Some(file) = files
            .iter_mut()
            .find(|f| f.path == *target && f.readers.is_empty())
        {
            file.readers = linked;
            continue;
        }
        files.push(File {
            readers: linked,
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
    facts.extend(ignored_cursor_rule(file));
    facts.extend(unresolved_imports(file, root));
    facts.extend(skipped_agents(file, by_path));
    facts.extend(windsurf_truncated(file));
}

fn fact(path: &Path, line: usize, message: String) -> Fact {
    Fact {
        path: path.to_path_buf(),
        line,
        message,
    }
}

/// A plain `.md` file in `.cursor/rules`, which Cursor never loads.
fn ignored_cursor_rule(file: &File) -> Option<Fact> {
    let parts: Vec<_> = file.path.iter().collect();
    let in_rules = parts
        .windows(2)
        .any(|w| w[0] == ".cursor" && w[1] == "rules");
    let mdc = file.path.extension().is_some_and(|e| e == "mdc");
    (in_rules && !mdc).then(|| {
        fact(
            &file.path,
            1,
            "Cursor reads only `.mdc` files in `.cursor/rules`, so this file is never loaded. Rename it to `.mdc` with frontmatter, or remove it.".into(),
        )
    })
}

/// Claude Code imports that do not resolve inside the repository.
fn unresolved_imports(file: &File, root: &Path) -> Vec<Fact> {
    if !file.readers.iter().any(|r| r.harness == CLAUDE) {
        return Vec::new();
    }
    markdown::imports(&file.source)
        .into_iter()
        .filter(|(_, import)| {
            !import.starts_with('~') && resolve(&root.join(&file.path), import, root).is_none()
        })
        .map(|(line, import)| {
            fact(
                &file.path,
                line,
                format!("The import `@{import}` does not resolve to a file in the repository."),
            )
        })
        .collect()
}

/// An AGENTS.md beside a CLAUDE.md that neither imports nor copies it.
fn skipped_agents(file: &File, by_path: &BTreeMap<&Path, &File>) -> Option<Fact> {
    if file.path.file_name().is_none_or(|n| n != "CLAUDE.md") {
        return None;
    }
    let agents = file
        .path
        .parent()
        .unwrap_or(Path::new(""))
        .join("AGENTS.md");
    let other = by_path.get(agents.as_path())?;
    let imported = markdown::imports(&file.source)
        .iter()
        .any(|(_, i)| i.trim_start_matches("./") == "AGENTS.md");
    // A link or an identical copy already gives Claude the same text.
    let same = other.source.trim() == file.source.trim();
    (!imported && !same).then(|| {
        fact(
            &agents,
            1,
            format!(
                "Claude Code reads `{}` here and skips this AGENTS.md. Add `@AGENTS.md` to it if Claude should follow these instructions too.",
                file.path.display()
            ),
        )
    })
}

/// A Windsurf rule past its character cap, whose tail is dropped.
fn windsurf_truncated(file: &File) -> Option<Fact> {
    let windsurf = file.readers.iter().any(|r| r.harness == WINDSURF)
        && file.path.file_name().is_some_and(|n| n != "AGENTS.md");
    let chars = file.source.chars().count();
    (windsurf && chars > WINDSURF_MAX_CHARS).then(|| {
        fact(
            &file.path,
            1,
            format!(
                "Windsurf reads the first {WINDSURF_MAX_CHARS} characters of a workspace rule; {} of this file's {chars} are dropped.",
                chars - WINDSURF_MAX_CHARS
            ),
        )
    })
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
            present
                .iter()
                .any(|x| *x == p || (p.ends_with('/') && x.starts_with(p)))
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
    fn kiro_junie_and_roo_files_load_by_their_own_rules() {
        let one = |harness: &str, load: Load| vec![(harness.to_string(), load)];
        assert_eq!(
            loads(".kiro/steering/tech.md", "", &[]),
            one(KIRO, Load::Always)
        );
        assert_eq!(
            loads(
                ".kiro/steering/api.md",
                "---\ninclusion: fileMatch\nfileMatchPattern: \"app/api/**/*\"\n---\n",
                &[]
            ),
            one(KIRO, Load::Files("app/api/**/*".into()))
        );
        assert_eq!(
            loads(".kiro/steering/x.md", "---\ninclusion: manual\n---\n", &[]),
            one(KIRO, Load::Manual)
        );
        assert_eq!(
            loads(".junie/guidelines.md", "", &[]),
            one(JUNIE, Load::Always)
        );
        assert!(loads(".junie/guidelines.md", "", &["AGENTS.md"]).is_empty());
        assert_eq!(
            loads(".junie/rules/style.md", "", &["AGENTS.md"]),
            one(JUNIE, Load::Always)
        );
        assert!(loads(".junie/rules/style.md", "", &[".junie/AGENTS.md"]).is_empty());
        assert_eq!(
            loads(".roo/rules/01-general.md", "", &[]),
            one(ROO, Load::Always)
        );
        assert_eq!(
            loads(".roo/rules-code/testing.txt", "", &[]),
            one(ROO, Load::Mode("code".into()))
        );
        assert_eq!(loads(".roorules", "", &[]), one(ROO, Load::Always));
        assert!(loads(".roorules", "", &[".roo/rules/a.md"]).is_empty());
        assert_eq!(
            loads(".roorules-architect", "", &[".roo/rules/a.md"]),
            one(ROO, Load::Mode("architect".into()))
        );
    }

    /// The context load of instruction files written to a fresh project.
    fn load_of(sources: &[(&str, &str)]) -> ContextLoad {
        let project = crate::tests::Project::new();
        let sources: BTreeMap<PathBuf, String> = sources
            .iter()
            .map(|(p, s)| {
                project.write(p, s);
                (PathBuf::from(p), s.to_string())
            })
            .collect();
        context_load(&files(sources, &[]), &[], &project.0)
    }

    fn has_fact(load: &ContextLoad, text: &str) -> bool {
        load.facts.iter().any(|f| f.message.contains(text))
    }

    #[test]
    fn claude_session_cost_leaves_out_html_comments() {
        let load = load_of(&[(
            "CLAUDE.md",
            "# Rules\nUse pnpm.\n<!-- note for people -->\n",
        )]);
        let claude = load.harnesses.iter().find(|h| h.harness == CLAUDE).unwrap();
        assert_eq!(claude.files, [PathBuf::from("CLAUDE.md")]);
        assert_eq!(
            claude.estimated_tokens,
            markdown::tokens("# Rules\nUse pnpm.\n\n")
        );
    }

    #[test]
    fn identical_copies_are_a_fact() {
        let text = "# Rules\nUse pnpm.\n";
        let load = load_of(&[("CLAUDE.md", text), ("GEMINI.md", text)]);
        assert!(has_fact(
            &load,
            "`CLAUDE.md` and `GEMINI.md` are identical copies"
        ));
    }

    #[test]
    fn an_import_that_does_not_resolve_is_a_fact() {
        let load = load_of(&[("CLAUDE.md", "See @docs/missing.md\n")]);
        assert!(has_fact(&load, "`@docs/missing.md` does not resolve"));
    }

    #[test]
    fn a_plain_markdown_cursor_rule_is_a_fact() {
        let load = load_of(&[(".cursor/rules/style.md", "# Style\n")]);
        assert!(has_fact(&load, "Cursor reads only `.mdc`"));
    }

    #[test]
    fn an_agents_file_claude_skips_is_a_fact() {
        let load = load_of(&[
            ("AGENTS.md", "# A\nUse pnpm.\n"),
            ("CLAUDE.md", "# C\nUse npm.\n"),
        ]);
        assert!(has_fact(&load, "skips this AGENTS.md"));
    }

    #[test]
    fn codex_truncation_is_a_fact_only_past_its_limit() {
        let at_limit = "x".repeat(CODEX_MAX_BYTES);
        assert!(!has_fact(
            &load_of(&[("AGENTS.md", &at_limit)]),
            "Codex stops reading"
        ));
        let over = format!("{at_limit}y");
        assert!(has_fact(
            &load_of(&[("AGENTS.md", &over)]),
            "1 of these 32769 bytes are not loaded"
        ));
    }

    /// `CLAUDE.md` linked to `target`: the files Claude Code reads, and
    /// the load facts.
    fn linked_claude(target: &str) -> (Vec<String>, usize) {
        let project = crate::tests::Project::new();
        let text = "# Build\nRun `make`.\n";
        project.write(target, text);
        let links = [(PathBuf::from("CLAUDE.md"), Some(PathBuf::from(target)))];
        let files = files([(PathBuf::from(target), text.to_string())].into(), &links);
        let claude = files
            .iter()
            .filter(|f| f.readers.iter().any(|r| r.harness == CLAUDE))
            .map(|f| f.path.to_string_lossy().into_owned())
            .collect();
        (claude, context_load(&files, &links, &project.0).facts.len())
    }

    #[test]
    fn a_linked_claude_file_reads_the_same_instructions() {
        assert_eq!(
            linked_claude("AGENTS.md"),
            (vec!["CLAUDE.md".to_string()], 0)
        );
        // A target no harness names is read once, under its own name.
        assert_eq!(
            linked_claude("agents.md"),
            (vec!["agents.md".to_string()], 0)
        );
    }
}
