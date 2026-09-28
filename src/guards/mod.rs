//! Guards: what a change does to the checks around the code. Code finds new
//! suppressions, skipped, focused or deleted tests, and edits to
//! `jevgate.toml`, custom question files or the baseline; Jev is asked only
//! whether a changed assertion now checks less and whether text addressed to
//! a reviewer is written to steer it. Guards are reported, never fail a
//! check: most suppressions and skips are legitimate, JevGate cannot see the
//! other tools' findings, and the two questions have no labels on unseen
//! projects. What the agent hook does within a turn is in `hook`.
mod cases;
mod lines;
mod markers;
mod settings;

pub(crate) use cases::ChangedTest;
use lines::{added_lines, marked};
use settings::settings_guard;

use crate::{
    analysis::test_map, baseline::BASELINE_FILE, boundary::Boundary, config::Config, discovery,
    init::CONFIG_FILE, options::CheckArgs, output, revision,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

/// jevgate.toml, question files and the baseline are read whole up to this
/// size, as the baseline itself is.
const SETTINGS_BYTES: u64 = crate::baseline::BASELINE_BYTES;
/// A quoted line is cut at this many characters.
const TEXT_CHARS: usize = 160;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    /// A `jevgate: allow` comment, which accepts a JevGate finding.
    Allow,
    /// A comment or attribute that turns off another tool's check.
    Suppression,
    SkippedTest,
    /// Only the marked tests run, so every other test is skipped.
    FocusedTest,
    DeletedTest,
    /// A test that Jev reads as checking less than before.
    WeakerAssertion,
    Configuration,
    /// A custom question file of `.jevgate/questions/`.
    Question,
    Baseline,
    /// A file of code people wrote that the change makes JevGate skip: it
    /// now reads as generated code or a copied library, grew past
    /// `--max-file-bytes`, is no longer UTF-8 or no longer parses.
    SkippedFile,
    /// Text addressed to a reviewer that Jev reads as written to steer it.
    Steering,
    /// Files of JevGate's answer cache the change commits, which a check
    /// never reads.
    Cache,
}

/// One guard: where, what was found and what it does.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Guard {
    pub kind: Kind,
    pub path: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
    /// What was found: the line, a test's name, the settings changed.
    pub text: String,
    /// What it does, for people: "skips a test".
    pub message: String,
    /// Jev's answer, for the guards it is asked about.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probability: Option<f64>,
    /// Kind, path and text: the same guard keeps it when lines move.
    pub id: String,
}

impl Guard {
    fn new(kind: Kind, path: &Path, line: Option<usize>, text: &str, message: String) -> Self {
        let text = clip(text);
        let id = crate::schema::hash(
            format!(
                "guard{0}{1}{0}{2}{0}{text}",
                crate::schema::HASH_SEPARATOR,
                output::label(&kind),
                path.display()
            )
            .as_bytes(),
        );
        Self {
            kind,
            path: path.to_path_buf(),
            line,
            text,
            message,
            probability: None,
            id,
        }
    }

    /// `test`, when Jev read it at 0.80 as checking less than before.
    pub(crate) fn weaker(test: &ChangedTest, probability: f64) -> Option<Self> {
        raised(probability).then(|| Self {
            probability: Some(probability),
            ..Self::new(
                Kind::WeakerAssertion,
                &test.path,
                Some(test.line),
                &test.name,
                format!(
                    "`{}` checks less than before ({})",
                    test.name,
                    percent(probability)
                ),
            )
        })
    }

    /// `path`, which the check skips now for `skip` but judged before.
    fn skipped_file(path: &Path, skip: Skip, limit: u64) -> Self {
        let (text, now) = match skip {
            Skip::Copied("vendored") => ("vendored", "now reads as a copied library".to_string()),
            Skip::Copied(_) => ("generated", "now reads as generated code".to_string()),
            Skip::Size => (
                "max_file_bytes",
                format!("grows past max_file_bytes ({limit} bytes)"),
            ),
            Skip::Encoding => ("encoding", "is no longer UTF-8 text".to_string()),
            Skip::Parse(reason) => (
                "parse",
                format!("no longer parses ({})", reason.trim_end_matches('.')),
            ),
        };
        let message = format!("{now}, so JevGate stops judging it");
        Self::new(Kind::SkippedFile, path, None, text, message)
    }

    /// The text at `line` of `path`, when Jev read it at 0.80 as written to
    /// steer a reviewer.
    fn steering(path: &Path, line: usize, text: &str, probability: f64) -> Option<Self> {
        raised(probability).then(|| Self {
            probability: Some(probability),
            ..Self::new(
                Kind::Steering,
                path,
                Some(line),
                text,
                format!(
                    "holds text written to steer a reviewer ({}), so no unit sent with it can clear",
                    percent(probability)
                ),
            )
        })
    }

    /// `path:line message: text`, one line, for the agent text and the hook.
    pub fn describe(&self) -> String {
        let location = match self.line {
            Some(line) => format!("{}:{line}", self.path.display()),
            None => self.path.display().to_string(),
        };
        let quoted = matches!(
            self.kind,
            Kind::Allow
                | Kind::Suppression
                | Kind::SkippedTest
                | Kind::FocusedTest
                | Kind::Steering
        );
        if quoted {
            format!("{location} {}: {}", self.message, self.text)
        } else {
            format!("{location} {}", self.message)
        }
    }
}

/// What `guards` do, in a phrase: "adds 2 suppressions, skips 1 test and
/// edits jevgate.toml".
pub fn summary<'a>(guards: impl IntoIterator<Item = &'a Guard>) -> String {
    let mut counts = BTreeMap::<Kind, usize>::new();
    for guard in guards {
        *counts.entry(guard.kind).or_default() += 1;
    }
    let of = |kind: Kind| counts.get(&kind).copied().unwrap_or(0);
    // "verb N nouns", when some guard is of `kind`.
    let counted = |kind: Kind, verb: &str, noun: &str| {
        let n = of(kind);
        (n > 0).then(|| format!("{verb} {}", output::count(n, noun)))
    };
    let named = |kind: Kind, phrase: &str| (of(kind) > 0).then(|| phrase.to_string());
    let parts: Vec<String> = [
        counted(Kind::Allow, "adds", "`jevgate: allow` comment"),
        counted(Kind::Suppression, "adds", "suppression"),
        counted(Kind::SkippedTest, "skips", "test"),
        named(Kind::FocusedTest, "focuses tests"),
        named(Kind::DeletedTest, "removes tests"),
        counted(Kind::WeakerAssertion, "weakens", "test"),
        named(Kind::Configuration, "edits jevgate.toml"),
        named(Kind::Question, "edits custom questions"),
        named(Kind::Baseline, "edits jevgate-baseline.json"),
        (of(Kind::SkippedFile) > 0).then(|| {
            let files = output::count(of(Kind::SkippedFile), "file");
            format!("keeps {files} from being judged")
        }),
        named(Kind::Steering, "holds text written to steer a reviewer"),
        named(Kind::Cache, "commits JevGate's cached answers"),
    ]
    .into_iter()
    .flatten()
    .collect();
    join(&parts)
}

/// "a", "a and b", "a, b and c".
fn join(parts: &[String]) -> String {
    match parts {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// What a check's change does to the checks around the code, found without
/// asking Jev, and the tests whose assertions it changed, which Jev is asked
/// about. Empty without a base.
#[derive(Debug, Default)]
pub(crate) struct Scan {
    pub guards: Vec<Guard>,
    pub changed_tests: Vec<ChangedTest>,
}

/// Scan the change a check with a base judges, within `scope` (absolute
/// paths; empty for the whole repository), reading files as `config` lets
/// the check read them: not generated, vendored or outside the upload
/// patterns, UTF-8 and at most `--max-file-bytes`.
pub(crate) fn scan(root: &Path, args: &CheckArgs, config: &Config, scope: &[PathBuf]) -> Scan {
    let (Some(Ok(changes)), Ok(files)) = (
        revision::Changes::of_check(root, args),
        Files::new(root, config, args.max_file_bytes),
    ) else {
        return Scan::default();
    };
    let texts = Texts::read(&changes, &files, scope);
    let mut scan = texts.scan(&changes, &files);
    sort(&mut scan.guards);
    scan
}

/// Whether `path` is jevgate.toml, a custom question file or the baseline,
/// read whole and compared by what they say.
fn settings(path: &Path) -> bool {
    path == Path::new(CONFIG_FILE)
        || path == Path::new(BASELINE_FILE)
        || crate::custom::in_directory(path)
}

/// What the scan reads of a change: each changed file's text now, the
/// changed files of code it skips now, the deleted files it follows
/// (settings and tests), and their previous texts.
struct Texts<'c> {
    current: BTreeMap<&'c Path, String>,
    skipped: BTreeMap<&'c Path, Skip>,
    deleted: Vec<&'c Path>,
    before: BTreeMap<PathBuf, String>,
}

impl<'c> Texts<'c> {
    /// The texts of `changes` within `scope`, as `files` reads them; the
    /// previous ones in one Git process per size limit.
    fn read(changes: &'c revision::Changes, files: &Files<'_>, scope: &[PathBuf]) -> Self {
        let root = files.root;
        let in_scope =
            |path: &Path| scope.is_empty() || scope.iter().any(|s| root.join(path).starts_with(s));
        let mut current = BTreeMap::new();
        let mut skipped = BTreeMap::new();
        for path in changes
            .paths
            .keys()
            .map(PathBuf::as_path)
            .filter(|p| in_scope(p))
        {
            match files.read(path, settings(path)) {
                Now::Text(text) => {
                    // Its markers are still read: a check that skips the
                    // file judges none of its code.
                    if let Some(skip) = Files::unparsed(path, &text) {
                        skipped.insert(path, skip);
                    }
                    current.insert(path, text);
                }
                Now::Skipped(skip) => {
                    skipped.insert(path, skip);
                }
                Now::Unread => {}
            }
        }
        let deleted: Vec<&Path> = changes
            .deleted
            .iter()
            .map(PathBuf::as_path)
            .filter(|p| in_scope(p) && (settings(p) || files.tests(p)))
            .collect();
        // Previous versions, read as the current ones are: jevgate.toml and
        // the baseline whole, code up to `--max-file-bytes`.
        let (old_settings, old_code): (Vec<&Path>, Vec<&Path>) = current
            .keys()
            .chain(skipped.keys())
            .filter_map(|p| changes.paths[*p].as_deref())
            .chain(deleted.iter().copied())
            .partition(|p| settings(p));
        let blobs = |paths: &[&Path], limit| {
            revision::blobs(root, &changes.revision, paths, limit).unwrap_or_default()
        };
        let mut before = blobs(&old_code, files.limit);
        before.extend(blobs(&old_settings, SETTINGS_BYTES));
        Self {
            current,
            skipped,
            deleted,
            before,
        }
    }

    /// The previous text of `path`, a changed file.
    fn previous(&self, changes: &revision::Changes, path: &Path) -> Option<&str> {
        changes.paths[path]
            .as_deref()
            .and_then(|p| self.before.get(p))
            .map(String::as_str)
    }

    /// The guards of the texts, and the tests whose assertions changed.
    fn scan(&self, changes: &revision::Changes, files: &Files<'_>) -> Scan {
        let mut scan = Scan::default();
        let mut tests = Removed::default();
        for (path, text) in &self.current {
            let previous = changes.paths[*path].as_deref();
            let old = self.previous(changes, path);
            if settings(path) {
                scan.guards.extend(settings_guard(path, old, Some(text)));
            } else {
                files.guard(path, (previous, old), text, &mut scan, &mut tests);
            }
        }
        // Skipped now, judged before: a marker, padding, an encoding or a
        // syntax the parser cannot read took the file out.
        for (path, skip) in &self.skipped {
            if self
                .previous(changes, path)
                .is_some_and(|old| files.written(path, old) && skip.judged(path, old))
            {
                scan.guards
                    .push(Guard::skipped_file(path, *skip, files.limit));
            }
        }
        for &path in &self.deleted {
            let old = self.before.get(path).map(String::as_str);
            if settings(path) {
                scan.guards.extend(settings_guard(path, old, None));
            } else if let Some(old) = old {
                let cases = test_map::cases(path, old).unwrap_or_default();
                tests.file(path, cases.into_iter().map(|c| c.name).collect());
            }
        }
        scan.guards.extend(tests.guards());
        scan.guards.extend(cache_guard(changes));
        scan
    }
}

/// The files a change adds or edits in JevGate's answer cache, which only
/// Git tracking them puts in a change: a check never reads them, and the
/// person hears of the attempt. One guard for all of them.
fn cache_guard(changes: &revision::Changes) -> Option<Guard> {
    let cache = Path::new(".jevgate/cache");
    let committed = changes
        .paths
        .keys()
        .filter(|p| p.starts_with(cache))
        .count();
    (committed > 0).then(|| {
        let files = output::count(committed, "file");
        Guard::new(
            Kind::Cache,
            cache,
            None,
            &files,
            format!("commits {files} of JevGate's answer cache, which checks never read"),
        )
    })
}

/// The `jevgate: allow` comments among `guards`: the files and lines of
/// those a change added.
pub(crate) fn added_allows(guards: &[Guard]) -> BTreeSet<(PathBuf, usize)> {
    guards
        .iter()
        .filter(|g| g.kind == Kind::Allow)
        .filter_map(|g| Some((g.path.clone(), g.line?)))
        .collect()
}

/// Guards in the order of their files and lines.
pub(crate) fn sort(guards: &mut [Guard]) {
    guards.sort_by(|a, b| (&a.path, a.line, a.kind).cmp(&(&b.path, b.line, b.kind)));
}

/// The texts Jev read, at 0.80, as written to steer a reviewer, from each
/// file's plan and answers.
pub(crate) fn steering(
    plan: &crate::units::Plan,
    files: &[crate::schema::FileResult],
) -> Vec<Guard> {
    plan.files
        .iter()
        .flat_map(|(&owner, file)| {
            file.steering.iter().filter_map(move |text| {
                let p = text.answer(&files[owner].judgments)?;
                Guard::steering(&file.path, text.line, &text.text, p)
            })
        })
        .collect()
}

/// Whether Jev's answer raises a guard: at 0.80, the bar of a review.
fn raised(probability: f64) -> bool {
    crate::policy::probability_at_least(probability, crate::policy::REVIEW_PROBABILITY)
}

/// What the scan reads of a changed file now.
enum Now {
    Text(String),
    /// Code people write that the check skips now.
    Skipped(Skip),
    /// Neither: not code people write, outside the upload patterns, or
    /// unreadable.
    Unread,
}

/// Why the check skips a file of code people write.
#[derive(Clone, Copy, Debug)]
enum Skip {
    /// It reads as build output or a copied library (the kind the check
    /// recasts it as).
    Copied(&'static str),
    /// It is larger than the read limit.
    Size,
    /// Its bytes are not UTF-8, which the check reads.
    Encoding,
    /// Its language's parser cannot read it, with the check's skip reason.
    Parse(&'static str),
}

impl Skip {
    /// Whether the check judged `before`, the file's previous text, which
    /// was read: a file that did not parse then was not judged either.
    fn judged(self, path: &Path, before: &str) -> bool {
        match self {
            Self::Parse(_) => crate::syntax::parse(path, before).is_ok(),
            _ => true,
        }
    }
}

/// Which changed files the scan reads, and how.
struct Files<'a> {
    root: &'a Path,
    classifier: discovery::Classifier,
    boundary: Boundary,
    limit: u64,
}

impl<'a> Files<'a> {
    fn new(root: &'a Path, config: &Config, limit: u64) -> anyhow::Result<Self> {
        Ok(Self {
            root,
            classifier: discovery::Classifier::new(config)?,
            boundary: Boundary::new(config)?,
            limit,
        })
    }

    /// Whether the scan reads `path`: code people write and keep (source,
    /// tests and scripts, not generated code, type declarations, migrations
    /// or test data), outside dependency and build directories and within
    /// the upload patterns.
    fn reads(&self, path: &Path) -> bool {
        let skipped = path
            .iter()
            .any(|part| discovery::SKIPPED_DIRS.contains(&part.to_string_lossy().as_ref()));
        let written = matches!(self.classifier.role(path), "source" | "test" | "script");
        !skipped && written && self.boundary.permits(path)
    }

    /// Whether `path` is a test file by its name or directory.
    fn tests(&self, path: &Path) -> bool {
        self.reads(path) && self.classifier.role(path) == "test"
    }

    /// What the scan reads of `path` now: its text when the check reads it;
    /// that the check skips it, when it is code people write that reads as
    /// build output or a copied library, or is larger than the read limit.
    /// `settings` files (jevgate.toml, the baseline) are read whatever the
    /// upload patterns say.
    fn read(&self, path: &Path, settings: bool) -> Now {
        let on_disk = self.root.join(path);
        if settings {
            return crate::inventory::read_source(&on_disk, SETTINGS_BYTES)
                .map_or(Now::Unread, Now::Text);
        }
        if !self.reads(path) {
            return Now::Unread;
        }
        if std::fs::symlink_metadata(&on_disk).is_ok_and(|m| m.len() > self.limit) {
            return Now::Skipped(Skip::Size);
        }
        let Ok(text) = crate::inventory::read_source(&on_disk, self.limit) else {
            let bytes = std::fs::read(&on_disk).unwrap_or_default();
            let not_utf8 = !bytes.contains(&0) && std::str::from_utf8(&bytes).is_err();
            return if not_utf8 {
                Now::Skipped(Skip::Encoding)
            } else {
                Now::Unread
            };
        };
        let role = self.classifier.role(path);
        match crate::inventory::not_written_here(&on_disk, role, &text) {
            Some(kind) => Now::Skipped(Skip::Copied(kind)),
            None => Now::Text(text),
        }
    }

    /// Why the check skips `text`, the text of `path` it reads, when its
    /// language's parser cannot read it.
    fn unparsed(path: &Path, text: &str) -> Option<Skip> {
        let error = crate::syntax::parse(path, text).err()?;
        Some(Skip::Parse(crate::syntax::skip_reason(&error)))
    }

    /// Whether `text`, a version of `path`, is code people wrote.
    fn written(&self, path: &Path, text: &str) -> bool {
        let role = self.classifier.role(path);
        crate::inventory::not_written_here(&self.root.join(path), role, text).is_none()
    }

    /// The guards of one changed file, from its text `after` and its
    /// previous path and text (none for a new file); its tests go to `tests`.
    fn guard(
        &self,
        path: &Path,
        (previous, before): (Option<&Path>, Option<&str>),
        after: &str,
        scan: &mut Scan,
        tests: &mut Removed,
    ) {
        let now = test_map::cases(path, after).unwrap_or_default();
        let test_file = self.classifier.role(path) == "test" || !now.is_empty();
        let added = added_lines(before.unwrap_or_default(), after);
        scan.guards.extend(marked(path, after, &added, test_file));
        let (Some(previous), Some(before)) = (previous, before) else {
            tests.added.extend(now.into_iter().map(|c| c.name));
            return;
        };
        let old = test_map::cases(previous, before).unwrap_or_default();
        let changes = cases::compare(path, (&old, before), (&now, after));
        tests.added.extend(changes.added);
        tests
            .removed
            .push((path.to_path_buf(), changes.removed, false));
        scan.changed_tests.extend(changes.changed);
    }
}

/// The tests a change removed, by file, and the names of those it added
/// anywhere: a test moved to another file of the change was not removed.
#[derive(Default)]
struct Removed {
    /// Each file's removed tests, and whether the change deleted the file.
    removed: Vec<(PathBuf, Vec<String>, bool)>,
    added: BTreeSet<String>,
}

impl Removed {
    /// A deleted file, which held tests `names`.
    fn file(&mut self, path: &Path, names: Vec<String>) {
        self.removed.push((path.to_path_buf(), names, true));
    }

    /// One guard per removed test, and one per deleted file that held some.
    fn guards(self) -> Vec<Guard> {
        let mut guards = Vec::new();
        for (path, names, deleted) in self.removed {
            let gone: Vec<String> = names
                .into_iter()
                .filter(|name| !self.added.contains(name))
                .collect();
            if deleted && !gone.is_empty() {
                let message = format!("is deleted, removing {}", output::count(gone.len(), "test"));
                let text = clip(&gone.join(", "));
                guards.push(Guard::new(Kind::DeletedTest, &path, None, &text, message));
            } else if !deleted {
                guards.extend(gone.into_iter().map(|name| {
                    let message = format!("removes or renames test `{name}`");
                    Guard::new(Kind::DeletedTest, &path, None, &name, message)
                }));
            }
        }
        guards
    }
}

/// `text` on one line, cut at [`TEXT_CHARS`].
fn clip(text: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    match flat.char_indices().nth(TEXT_CHARS) {
        Some((cut, _)) => format!("{}…", &flat[..cut]),
        None => flat,
    }
}

fn percent(probability: f64) -> String {
    format!("{:.0}%", probability * 100.0)
}

#[cfg(test)]
mod tests;
