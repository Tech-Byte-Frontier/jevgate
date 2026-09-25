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
    /// Not in the repository, but the one file of the same name near the
    /// document is, such as `kasada-server.tsx` for `kasada-server.ts`.
    Nearby(PathBuf),
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
            Fate::Nearby(path) => format!(
                "not in the repository; `{}` has the same name",
                path.display()
            ),
            Fate::NoScript => {
                "no script, target or dependency with this name in the manifests".into()
            }
        }
    }
}

const EXTENSIONS: &[&str] = &[
    "md", "mdx", "rst", "adoc", "ts", "tsx", "js", "jsx", "mjs", "cjs", "rs", "py", "json", "toml",
    "sh", "sql", "yml", "yaml", "css", "html", "lock", "go", "java", "kt", "rb",
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
    let shown = shown(text, &top);
    let mut found = BTreeSet::new();
    let mut out = Vec::new();
    for token in code_spans(text).into_iter().chain(link_targets(text)) {
        let Some(name) = path_like(&token, &top) else {
            continue;
        };
        // A relative link that climbs above the repository, such as a
        // README's `../../actions/workflows/ci.yml/badge.svg`, is a route of
        // the site that hosts it.
        if name.starts_with("../") && escapes(base, &name) {
            continue;
        }
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
        // A path Git never had is the reader's when the section writes it
        // out, and local when the ignore files cover it.
        if fate == Fate::Absent && shown.contains(&name) {
            continue;
        }
        out.push(Missing { name, fate });
    }
    // A name ending in `/`, or without an extension, may be a directory,
    // which `dir/` patterns match: `backend/app/frontend` is a build output
    // that `backend/app/frontend/` ignores.
    let forms = |name: &str| {
        let directory = name.ends_with('/');
        let name = name.trim_end_matches('/');
        let bare = Path::new(name).extension().is_none();
        [normal(Path::new(name)), normal(&base.join(name))]
            .into_iter()
            .flat_map(|p| {
                let dir = PathBuf::from(format!("{}/", p.display()));
                match (directory, bare) {
                    (true, _) => vec![dir],
                    (false, true) => vec![p, dir],
                    (false, false) => vec![p],
                }
            })
            .collect::<Vec<PathBuf>>()
    };
    let absent: Vec<PathBuf> = out
        .iter()
        .filter(|m| m.fate == Fate::Absent)
        .flat_map(|m| forms(&m.name))
        .collect();
    let ignored = super::history::ignored(root, &absent);
    out.retain(|m| m.fate != Fate::Absent || !forms(&m.name).iter().any(|p| ignored.contains(p)));
    for m in &mut out {
        if m.fate == Fate::Absent
            && let Some(path) = nearby(base, &m.name, history)
        {
            m.fate = Fate::Nearby(path);
        }
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

/// Extensions a file keeps its role under, such as `.ts` renamed to `.tsx`.
const SIBLING_EXTENSIONS: &[&[&str]] = &[
    &["ts", "tsx", "js", "jsx", "mjs", "cjs", "mts", "cts"],
    &["yml", "yaml"],
    &["md", "mdx", "markdown"],
];

/// The one tracked file under the document's directory with the file name
/// of `name`, or its stem and a sibling extension: the file a stale name
/// most likely means, moved or renamed. None when several match.
fn nearby(base: &Path, name: &str, history: &History) -> Option<PathBuf> {
    let name = Path::new(name.trim_end_matches('/'));
    let extension = name.extension()?.to_str()?;
    let stem = name.file_stem()?;
    let siblings = SIBLING_EXTENSIONS
        .iter()
        .find(|group| group.contains(&extension))
        .copied()
        .unwrap_or(&[]);
    let same = |p: &&PathBuf| {
        p.starts_with(base)
            && p.file_stem() == Some(stem)
            && p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e == extension || siblings.contains(&e))
    };
    let mut found = history.tracked.iter().filter(same);
    let first = found.next()?;
    found.next().is_none().then(|| first.clone())
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
/// Whether `name`, relative to the document's directory `base`, climbs
/// above the repository root.
fn escapes(base: &Path, name: &str) -> bool {
    let mut depth = base.components().count() as isize;
    for part in Path::new(name).components() {
        match part {
            Component::ParentDir => depth -= 1,
            Component::Normal(_) => depth += 1,
            _ => {}
        }
        if depth < 0 {
            return true;
        }
    }
    false
}

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

/// Interpreted-text roles whose target is a file: reStructuredText's
/// `:file:` and `:download:`, and the same roles written the MyST way.
const PATH_ROLES: &[&str] = &["file", "download", "doc"];

/// The lines outside fenced code blocks, and the fenced lines, of `text`.
fn split_fences(text: &str) -> (Vec<&str>, Vec<&str>) {
    let (mut prose, mut code) = (Vec::new(), Vec::new());
    let mut fence: Option<&str> = None;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if let Some(open) = fence {
            if trimmed.starts_with(open) {
                fence = None;
            } else {
                code.push(line);
            }
        } else if let Some(open) = ["```", "~~~"].into_iter().find(|f| trimmed.starts_with(f)) {
            fence = Some(open);
        } else {
            prose.push(line);
        }
    }
    (prose, code)
}

/// Inline code spans of one line, each with the text before it.
fn line_spans(line: &str) -> Vec<(&str, &str)> {
    let mut out = Vec::new();
    let mut rest = line;
    let mut offset = 0;
    while let Some(open) = rest.find('`') {
        let run = rest[open..].chars().take_while(|&c| c == '`').count();
        let body = &rest[open + run..];
        let fence = "`".repeat(run);
        // The closing run has the same length as the opening one.
        let close = body.match_indices(&fence).find(|(i, _)| {
            !body[i + run..].starts_with('`') && (*i == 0 || !body[..*i].ends_with('`'))
        });
        let Some((close, _)) = close else { break };
        out.push((&line[..offset + open], &body[..close]));
        let consumed = open + run + close + run;
        offset += consumed;
        rest = &rest[consumed..];
    }
    out
}

/// The role an interpreted-text span is written with, such as `attr` in
/// `:py:attr:` or `{file}`, from the text before the span.
fn role(before: &str) -> Option<&str> {
    if let Some(inner) = before.strip_suffix('}') {
        let start = inner.rfind('{')?;
        return Some(&inner[start + 1..]);
    }
    let inner = before.strip_suffix(':')?;
    let start = inner
        .rfind(|c: char| !(c.is_ascii_alphanumeric() || c == ':' || c == '-' || c == '_'))
        .map_or(0, |i| i + 1);
    let name = inner[start..].strip_prefix(':')?;
    (!name.is_empty()).then(|| name.rsplit(':').next().unwrap_or(name))
}

/// Inline code spans without whitespace, outside code blocks. A span written
/// with a role names what the role says: `:attr:` and `:class:` name code,
/// only `:file:` and its kind name files.
fn code_spans(text: &str) -> Vec<String> {
    split_fences(text)
        .0
        .into_iter()
        .flat_map(line_spans)
        .filter(|(before, _)| role(before).is_none_or(|r| PATH_ROLES.contains(&r)))
        .map(|(_, span)| span.trim())
        .filter(|span| !span.is_empty() && !span.contains(char::is_whitespace))
        .map(str::to_string)
        .collect()
}

/// Paths the section writes out: the file a code block is titled with, such
/// as `filename="app/api/chat.ts"`, or the one path named by the paragraph
/// that introduces a code block. A tutorial shows the reader's own files this
/// way; they are not the repository's.
fn shown(text: &str, top: &BTreeSet<&str>) -> BTreeSet<String> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = BTreeSet::new();
    let mut fenced = false;
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        if !["```", "~~~"].iter().any(|f| trimmed.starts_with(f)) {
            continue;
        }
        fenced = !fenced;
        if !fenced {
            continue;
        }
        for key in ["filename=", "title=", "file="] {
            if let Some(value) = trimmed.split(key).nth(1) {
                let value = value.trim_start_matches(['"', '\'']);
                let end = value.find(['"', '\'', ' ']).unwrap_or(value.len());
                out.extend(path_like(&value[..end], top));
            }
        }
        // The paragraph before, past blank lines and directive lines.
        let mut j = i;
        while j > 0 && (blank_or_directive(lines[j - 1])) {
            j -= 1;
        }
        let mut named = BTreeSet::new();
        while j > 0 && !lines[j - 1].trim().is_empty() && !is_fence(lines[j - 1]) {
            j -= 1;
            named.extend(
                line_spans(lines[j])
                    .into_iter()
                    .filter(|(before, _)| role(before).is_none_or(|r| PATH_ROLES.contains(&r)))
                    .filter_map(|(_, span)| path_like(span, top)),
            );
        }
        if named.len() == 1 {
            out.extend(named);
        }
    }
    out
}

fn is_fence(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with("```") || trimmed.starts_with("~~~")
}

fn blank_or_directive(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.is_empty() || (trimmed.starts_with(".. ") && trimmed.contains("::"))
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

/// Scripts named after a package manager or task runner where a command
/// starts: at the start of a code line or an inline code span, after a
/// prompt, or after `&&`, `||`, `;` or `|`. The same word inside a comment
/// or a sentence, such as "make sure", is not a command.
fn commands(text: &str) -> Vec<String> {
    let (prose, code) = split_fences(text);
    let spans = prose.into_iter().flat_map(line_spans).map(|(_, span)| span);
    let mut out = Vec::new();
    for line in code.into_iter().chain(spans) {
        let line = line.replace("&&", ";").replace("||", ";");
        for command in line.split([';', '|']) {
            let words: Vec<&str> = command
                .split_whitespace()
                .skip_while(|w| matches!(*w, "$" | ">" | "%") || assignment(w))
                .collect();
            let next = |n: usize| words.get(n).copied().unwrap_or("");
            let script = match next(0) {
                "pnpm" | "yarn" | "npm" if next(1) == "run" => next(2),
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
    }
    out
}

/// A shell variable assignment such as `NODE_ENV=production`.
fn assignment(word: &str) -> bool {
    word.split_once('=').is_some_and(|(name, _)| {
        !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
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
    fn links_above_the_repository_are_routes_of_its_host() {
        let text = "[![CI](../../actions/workflows/ci.yml/badge.svg)](../../actions/workflows/ci.yml) and [gone](../src/gone.ts)";
        assert_eq!(
            found(text),
            [("../src/gone.ts".to_string(), Fate::Absent)],
            "a link inside the repository is still checked"
        );
    }

    #[test]
    fn undeclared_scripts_in_code_are_missing() {
        let text =
            "```sh\npnpm quality\npnpm run lint\nnpm install\n```\nIn prose, make the build.";
        assert_eq!(found(text), [("lint".to_string(), Fate::NoScript)]);
    }

    #[test]
    fn scripts_are_read_where_a_command_starts() {
        let text = "```sh\n$ CI=1 pnpm dev && make release\n# make sure the server runs\nprint('just not yet')\n```\nUse `yarn` or `pnpm` with `just check-all`, or the ``python\nyourapp.py`` will not work. Let's just say.";
        assert_eq!(
            found(text),
            [
                ("release".to_string(), Fate::NoScript),
                ("check-all".to_string(), Fate::NoScript)
            ]
        );
    }

    #[test]
    fn spans_with_a_code_role_are_not_paths() {
        let text = "Assign :attr:`flask.Flask.json` or :py:mod:`flask.json`; edit :file:`conf/app.json` and {download}`data/seed.json`.";
        let names: Vec<String> = found(text).into_iter().map(|(n, _)| n).collect();
        assert_eq!(names, ["conf/app.json", "data/seed.json"]);
    }

    #[test]
    fn a_file_the_section_writes_out_is_the_readers() {
        let text = "Create a route handler, `app/api/chat.ts`:\n\n```ts filename=\"app/api/chat.ts\"\nexport {}\n```\n\nHere is the example :file:`database.py` module::\n```\n    engine = None\n```\n.. code-block:: python\n\nThen update both `kasada-server.ts` and `kasada-client.ts`, like this:\n\n```\nhttps://example.com/api\n```\n";
        let names: Vec<String> = found(text).into_iter().map(|(n, _)| n).collect();
        assert_eq!(
            names,
            ["kasada-server.ts", "kasada-client.ts"],
            "a paragraph naming two files does not say which the code is"
        );
    }

    #[test]
    fn a_missing_name_points_at_the_file_it_most_likely_means() {
        let history = History {
            tracked: [
                "docs/kasada/kasada-server.tsx",
                "docs/a/index.ts",
                "docs/b/index.ts",
                "src/app.ts",
            ]
            .into_iter()
            .map(PathBuf::from)
            .collect(),
            ..Default::default()
        };
        assert_eq!(
            nearby(Path::new("docs"), "kasada-server.ts", &history),
            Some(PathBuf::from("docs/kasada/kasada-server.tsx"))
        );
        assert_eq!(nearby(Path::new("docs"), "index.js", &history), None);
        assert_eq!(
            nearby(Path::new("docs"), "kasada-server.md", &history),
            None,
            "another kind of file"
        );
        assert_eq!(nearby(Path::new("docs"), "app.ts", &history), None);
    }

    #[test]
    fn ignored_paths_are_local_files() {
        let project = crate::tests::Project::new();
        project.write(".gitignore", "out/\n*.local\nweb/dist/\n");
        let git = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .current_dir(&*project.0)
                .output()
                .unwrap()
        };
        assert!(git(&["init", "-q"]).status.success());
        // `web/` is tracked, so its missing entries are candidates.
        let history = History {
            tracked: [PathBuf::from("web/index.html")].into(),
            ..Default::default()
        };
        let names: Vec<String> = missing(
            &project.0,
            Path::new("docs/intro.md"),
            "Open `out/report.json`, `settings.json.local`, `web/dist`, `web/old` and `config/app.json`.",
            &history,
            &BTreeSet::new(),
        )
        .into_iter()
        .map(|m| m.name)
        .collect();
        assert_eq!(
            names,
            ["web/old", "config/app.json"],
            "a bare name an ignored directory covers is local"
        );
    }
}
