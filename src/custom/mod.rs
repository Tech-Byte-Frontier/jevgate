//! Custom questions: a team's conventions written as yes/no questions. Each
//! is the rule `custom/<id>`, asked as a Noul of every unit it names, with
//! yes a violation at its own threshold and level. They come from
//! `[[question]]` tables of the configuration and, with the repository's own
//! configuration, from one file per question in `.jevgate/questions/`. Its
//! examples (`examples`) are what `jevgate rules test` asks it about.
use crate::{catalog, schema::Strength};
use anyhow::{Context, Result, anyhow, ensure};
use serde::Deserialize;
use std::path::{Path, PathBuf};

mod examples;
pub mod gallery;
mod ignored;
pub mod propose;

pub use examples::{Example, ExampleSpec, Expected, Text};
pub use ignored::ignored;

/// Question files, one per question, relative to the repository root.
pub const DIRECTORY: &str = ".jevgate/questions";

/// The question directory of the repository at `root`.
pub fn directory(root: &Path) -> PathBuf {
    within(root, DIRECTORY)
}

/// `relative`, a slash path, under `root`, joined a component at a time: a
/// canonical Windows root is a `\\?\` path, where `/` is not a separator.
pub fn within(root: &Path, relative: &str) -> PathBuf {
    relative
        .split('/')
        .fold(root.to_path_buf(), |path, part| path.join(part))
}

/// A question as a configuration writes it.
#[derive(Deserialize)]
#[cfg_attr(test, derive(schemars::JsonSchema), schemars(rename = "Question"))]
#[serde(deny_unknown_fields)]
pub struct Spec {
    /// Names the rule: `custom/` and the id, of lowercase letters, digits and single hyphens, starting with a letter. Required in jevgate.toml; a question file's name gives it.
    #[serde(default)]
    pub id: Option<String>,
    /// One yes/no question about each unit, ending in `?`; yes is a violation.
    pub question: String,
    /// Why the rule exists; sent with the question.
    #[serde(default)]
    pub background: Option<String>,
    /// How to decide: what counts as a violation and what does not; sent with the question.
    #[serde(default)]
    pub guidance: Option<String>,
    /// What the question is asked about.
    pub unit: Kind,
    /// Globs of the files it applies to. Default: every file its unit applies to. For `file` and `hunk`, they can also name text files in languages JevGate does not parse.
    #[serde(default)]
    pub paths: Vec<String>,
    /// The probability of yes at or above which a unit breaks the rule (0.5 to 0.99); at or below one minus it, the unit is clear. Default: 0.8.
    #[serde(default)]
    pub threshold: Option<f64>,
    /// The level of its findings, and the level at which it fails the gate unless `fail_on` or `--fail-on` says otherwise. Default: review.
    #[serde(default)]
    pub level: Option<Level>,
    /// The next step a finding shows. Default: fix it, or accept it with a `jevgate: allow` comment and a reason.
    #[serde(default)]
    pub next_step: Option<String>,
    /// Examples of code that breaks the rule: `jevgate rules test` fails when the answer about one stays below the threshold.
    #[serde(default)]
    pub failing: Vec<ExampleSpec>,
    /// Examples of code that keeps the rule: `jevgate rules test` fails when the answer about one reaches the threshold.
    #[serde(default)]
    pub passing: Vec<ExampleSpec>,
}

/// What a custom question is asked about.
#[derive(Clone, Copy, Debug, Deserialize, serde::Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(test, derive(schemars::JsonSchema), schemars(rename = "QuestionUnit"))]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// A function or method of application code, outside tests.
    Function,
    /// A whole file.
    File,
    /// A test case; needs include_tests, as the test rules do.
    Test,
    /// A heading section of an agent instruction file or project documentation.
    Section,
    /// A comment or docstring of application code, with the code it is about.
    Comment,
    /// A changed hunk since --base, with three lines of context.
    Hunk,
}

impl Kind {
    /// The unit in the plural-ready form a dimension counts it by.
    pub fn noun(self) -> &'static str {
        match self {
            Self::Function => "function",
            Self::File => "file",
            Self::Test => "test",
            Self::Section => "section",
            Self::Comment => "comment",
            Self::Hunk => "changed hunk",
        }
    }

    /// What one request states about the unit, for `jevgate rules`.
    fn evidence(self) -> &'static str {
        match self {
            Self::Function => "one function's source",
            Self::File => "one file's source",
            Self::Test => "one test's source",
            Self::Section => "one documentation section with its heading",
            Self::Comment => "one comment with the code it is about",
            Self::Hunk => "one changed hunk with three lines of context",
        }
    }

    /// The files a question of this kind reads without `paths`.
    fn default_scope(self) -> &'static str {
        match self {
            Self::Function | Self::Comment => "application code",
            Self::File | Self::Hunk => "application and test code",
            Self::Test => "tests, with --include-tests",
            Self::Section => "agent instruction files and project documentation",
        }
    }
}

/// The level of a custom question's findings.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[cfg_attr(test, derive(schemars::JsonSchema), schemars(rename = "QuestionLevel"))]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Review,
    Consider,
    Note,
}

/// The threshold of the built-in policy, and so of a question without one.
const DEFAULT_THRESHOLD: f64 = crate::policy::REVIEW_PROBABILITY;
/// Below one half, a unit Jev leans away from would break the rule.
const THRESHOLDS: std::ops::RangeInclusive<f64> = 0.5..=0.99;
/// Built-in questions stay under 200 characters: one short question, with
/// the detail in background and guidance.
pub(crate) const QUESTION_CHARS: usize = 300;
pub(crate) const TEXT_CHARS: usize = 2_000;
const NEXT_STEP_CHARS: usize = 300;
const ID_CHARS: usize = 48;
/// A question file larger than this is refused: one question with its
/// background and guidance takes a few kilobytes.
pub(crate) const FILE_BYTES: u64 = 65_536;
/// The version of a question: enough of a hash to tell edits apart.
const VERSION_CHARS: usize = 12;

/// A validated custom question.
#[derive(Debug)]
pub struct Question {
    /// `custom/<id>`: the rule's key and ID.
    pub rule: String,
    pub question: String,
    pub background: Option<String>,
    pub guidance: Option<String>,
    pub unit: Kind,
    pub paths: Vec<String>,
    matcher: globset::GlobSet,
    pub threshold: f64,
    pub level: Strength,
    pub next_step: String,
    /// The file that defines it, relative to the repository root when inside it.
    pub source: PathBuf,
    /// A hash of what it asks and how its answer is read.
    pub version: String,
    /// Its failing examples, then its passing ones.
    pub examples: Vec<Example>,
    /// The files it reads, for `jevgate rules`.
    scope: String,
    /// Where it is defined, for `jevgate rules`.
    provenance: String,
}

impl Question {
    /// The id after `custom/`.
    pub fn id(&self) -> &str {
        &self.rule[catalog::CUSTOM_GROUP.len() + 1..]
    }

    /// Whether it applies to the file at `path`, relative to the root.
    pub fn applies_to(&self, path: &Path) -> bool {
        self.paths.is_empty() || self.matcher.is_match(path)
    }

    /// Whether its `paths` name files, so a `file` or `hunk` question also
    /// reads text files JevGate does not otherwise select.
    pub fn names_files(&self) -> bool {
        !self.paths.is_empty() && matches!(self.unit, Kind::File | Kind::Hunk)
    }

    /// The levels at which it fails the default gate, which `mature` stands
    /// for: its own, none for a note. JevGate cannot measure a team's
    /// question on projects it was never tuned on, as it measures its own
    /// rules; whoever wrote and committed it chose where it blocks, and its
    /// examples test it (`jevgate rules test`).
    pub fn blocks(&self) -> Vec<Strength> {
        [self.level]
            .into_iter()
            .filter(|level| *level != Strength::Note)
            .collect()
    }

    /// Its catalog entry, beside the built-in rules.
    pub fn rule(&'static self) -> catalog::Rule {
        catalog::Rule {
            id: &self.rule,
            key: &self.rule,
            group: catalog::CUSTOM_GROUP,
            default_enabled: true,
            version: &self.version,
            scope: &self.scope,
            unit: self.unit.evidence(),
            inspection: &self.question,
            acceptable_example: self.guidance.as_deref().unwrap_or(""),
            requires_tests: self.unit == Kind::Test,
        }
    }

    /// Where it is defined, as `jevgate rules --format json` names the
    /// source of a rule's labels: a custom question has none.
    pub fn provenance(&self) -> &str {
        &self.provenance
    }

    /// What it is asked about and when it fails, for the rules table:
    /// `function, review at 0.80, src/api/**`.
    pub fn summary(&self) -> String {
        let mut parts = vec![self.asked()];
        parts.extend(self.paths.iter().cloned());
        parts.join(", ")
    }

    /// Its unit, and the level and threshold of its findings:
    /// `function, review at 0.80`.
    pub fn asked(&self) -> String {
        let level = crate::output::label(&self.level);
        format!(
            "{}, {level} at {:.2}",
            crate::output::label(&self.unit),
            self.threshold
        )
    }

    /// The definition `jevgate rules --format json` adds to its entry.
    pub fn describe(&self) -> serde_json::Value {
        let count = |expected| {
            self.examples
                .iter()
                .filter(|e| e.expected == expected)
                .count()
        };
        serde_json::json!({
            "question": self.question,
            "background": self.background,
            "guidance": self.guidance,
            "unit": self.unit,
            "paths": self.paths,
            "threshold": self.threshold,
            "level": self.level,
            "next_step": self.next_step,
            "source": self.source,
            "examples": {
                "failing": count(Expected::Failing),
                "passing": count(Expected::Passing),
            },
        })
    }
}

/// The questions of a configuration: the `[[question]]` tables of `file`
/// and, when `directory` is given, one per `.toml` file directly in it. An
/// id defined twice is an error naming both places.
pub fn load(
    root: &Path,
    (file, specs): (&Path, &[Spec]),
    directory: Option<&Path>,
) -> Result<Vec<Question>> {
    let shown =
        |path: &Path| crate::discovery::relative(path, root).unwrap_or_else(|_| path.to_path_buf());
    let mut questions = Vec::new();
    for (index, spec) in specs.iter().enumerate() {
        let id = spec.id.as_deref().ok_or_else(|| {
            anyhow!(
                "Question {} in {} needs an id",
                index + 1,
                shown(file).display()
            )
        })?;
        questions.push(validate(spec, id, shown(file))?);
    }
    for path in directory
        .map(question_files)
        .transpose()?
        .unwrap_or_default()
    {
        questions.push(read_file(&path, shown(&path))?);
    }
    for (at, question) in questions.iter().enumerate() {
        if let Some(earlier) = questions[..at].iter().find(|q| q.rule == question.rule) {
            anyhow::bail!(
                "Question {} is defined twice: in {} and in {}",
                question.rule,
                earlier.source.display(),
                question.source.display()
            );
        }
    }
    Ok(questions)
}

/// The `.toml` files directly in `directory`, by name; none when it does
/// not exist. Hidden files are left out, as editors keep backups there.
pub(crate) fn question_files(directory: &Path) -> Result<Vec<PathBuf>> {
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(error).with_context(|| format!("Cannot read {}", directory.display()));
        }
    };
    let mut files = Vec::new();
    for entry in entries {
        let path = entry?.path();
        let visible = path
            .file_name()
            .is_some_and(|name| !name.to_string_lossy().starts_with('.'));
        if visible && path.extension().is_some_and(|e| e == "toml") && path.is_file() {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

/// One question file, whose name is its id. It is read as sources are, so a
/// question file linked to another file is refused rather than followed:
/// a parse error would show a line of that file, such as a key, in a log.
pub(crate) fn read_file(path: &Path, shown: PathBuf) -> Result<Question> {
    let text = crate::inventory::read_source(path, FILE_BYTES)
        .with_context(|| format!("Cannot read {}", shown.display()))?;
    let spec: Spec =
        toml::from_str(&text).with_context(|| format!("Invalid {}", shown.display()))?;
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    if let Some(id) = &spec.id {
        ensure!(
            *id == stem,
            "Question id {id:?} in {} does not match its file name; name the file {id}.toml or remove the id",
            shown.display()
        );
    }
    validate(&spec, &stem, shown)
}

/// A question checked field by field; every error names it and its file.
fn validate(spec: &Spec, id: &str, source: PathBuf) -> Result<Question> {
    ensure!(
        valid_id(id),
        "Invalid question id {id:?} in {}: use lowercase letters, digits and single hyphens, starting with a letter, at most {ID_CHARS} characters",
        source.display()
    );
    let rule = format!("{}/{id}", catalog::CUSTOM_GROUP);
    let checked = checked(spec)
        .map_err(|problem| anyhow!("Question {rule} in {}: {problem}", source.display()))?;
    let level = match spec.level.unwrap_or(Level::Review) {
        Level::Review => Strength::Review,
        Level::Consider => Strength::Consider,
        Level::Note => Strength::Note,
    };
    let version = version(&checked, spec.unit, level);
    let scope = if spec.paths.is_empty() {
        spec.unit.default_scope().to_string()
    } else {
        format!("files matching {}", spec.paths.join(", "))
    };
    Ok(Question {
        next_step: checked.next_step.unwrap_or_else(|| {
            format!("Fix it, or accept it with `jevgate: allow({rule}) reason` on its line")
        }),
        provenance: format!("custom question in {}", source.display()),
        rule,
        question: checked.question,
        background: checked.background,
        guidance: checked.guidance,
        unit: spec.unit,
        paths: spec.paths.clone(),
        matcher: checked.matcher,
        threshold: checked.threshold,
        level,
        source,
        version,
        examples: checked.examples,
        scope,
    })
}

/// A question's texts, threshold, paths and examples once checked.
struct Checked {
    question: String,
    background: Option<String>,
    guidance: Option<String>,
    next_step: Option<String>,
    threshold: f64,
    matcher: globset::GlobSet,
    examples: Vec<Example>,
}

/// The fields of `spec` a person writes freely, checked; the first problem
/// otherwise.
fn checked(spec: &Spec) -> Result<Checked, String> {
    let question = spec.question.trim().to_string();
    if !question.ends_with('?') {
        return Err("`question` must be one yes/no question ending in `?`".into());
    }
    text(&Some(question.clone()), "question", QUESTION_CHARS)
        .map_err(|problem| format!("{problem}; move detail to background or guidance"))?;
    let threshold = spec.threshold.unwrap_or(DEFAULT_THRESHOLD);
    if !THRESHOLDS.contains(&threshold) {
        return Err(format!(
            "threshold {threshold} is outside {} to {}",
            THRESHOLDS.start(),
            THRESHOLDS.end()
        ));
    }
    let matcher = crate::boundary::globs(&spec.paths)
        .map_err(|error| format!("invalid paths {:?}: {error}", spec.paths))?;
    let applies = |path: &Path| spec.paths.is_empty() || matcher.is_match(path);
    Ok(Checked {
        background: text(&spec.background, "background", TEXT_CHARS)?,
        guidance: text(&spec.guidance, "guidance", TEXT_CHARS)?,
        next_step: text(&spec.next_step, "next_step", NEXT_STEP_CHARS)?,
        examples: examples::validate((&spec.failing, &spec.passing), applies)?,
        matcher,
        question,
        threshold,
    })
}

/// Lowercase letters, digits and single hyphens, starting with a letter:
/// safe in a question key, a rule name and an allow comment.
pub(crate) fn valid_id(id: &str) -> bool {
    id.len() <= ID_CHARS
        && id.starts_with(|c: char| c.is_ascii_lowercase())
        && !id.ends_with('-')
        && !id.contains("--")
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// An optional text trimmed, none when empty, and at most `limit` characters.
fn text(value: &Option<String>, field: &str, limit: usize) -> Result<Option<String>, String> {
    let value = value
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string);
    match value {
        Some(v) if v.chars().count() > limit => Err(format!(
            "`{field}` is {} characters; keep it within {limit}",
            v.chars().count()
        )),
        other => Ok(other),
    }
}

/// A hash of everything that changes what is asked or how the answer is
/// read, so a finding records which wording of the question raised it.
fn version(checked: &Checked, unit: Kind, level: Strength) -> String {
    let fields = serde_json::json!([
        checked.question,
        checked.background,
        checked.guidance,
        unit,
        checked.threshold,
        level,
    ]);
    let mut hash = crate::schema::hash(fields.to_string().as_bytes());
    hash.truncate(VERSION_CHARS);
    hash
}

/// A custom question or group, as a rule name: `custom`, `custom/<id>`.
#[cfg(test)]
pub const NAMES: &str = "^custom(/[a-z][a-z0-9]*(-[a-z0-9]+)*)?$";

/// The limits validation holds a question and its examples to, in the JSON
/// schema's `definitions`.
#[cfg(test)]
pub fn schema(definitions: &mut serde_json::Value) {
    use serde_json::json;
    let example = &mut definitions["QuestionExample"];
    example["oneOf"] = json!([{"required": ["code"]}, {"required": ["file"]}]);
    example["dependentRequired"] = json!({"code": ["path"]});
    let fields = &mut definitions["Question"]["properties"];
    fields["id"]["pattern"] = json!("^[a-z][a-z0-9]*(-[a-z0-9]+)*$");
    fields["id"]["maxLength"] = json!(ID_CHARS);
    fields["question"]["pattern"] = json!("\\?\\s*$");
    fields["question"]["maxLength"] = json!(QUESTION_CHARS);
    for text in ["background", "guidance"] {
        fields[text]["maxLength"] = json!(TEXT_CHARS);
    }
    fields["next_step"]["maxLength"] = json!(NEXT_STEP_CHARS);
    fields["threshold"]["minimum"] = json!(THRESHOLDS.start());
    fields["threshold"]["maximum"] = json!(THRESHOLDS.end());
}

/// The `[[question]]` tables of configuration text, for tests.
#[cfg(test)]
pub fn parse(text: &str) -> Result<&'static [Question]> {
    let config: crate::config::Config = toml::from_str(text)?;
    let questions = load(
        Path::new("."),
        (Path::new("jevgate.toml"), &config.question),
        None,
    )?;
    Ok(Box::leak(questions.into_boxed_slice()))
}

#[cfg(test)]
mod tests;
