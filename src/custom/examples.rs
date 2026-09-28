//! A custom question's examples: code that breaks its rule (`failing`) and
//! code that keeps it (`passing`), which `jevgate rules test` asks it about.
//! An example is a file's text at a path, written inline or read from a
//! file of the repository. A file example is uploaded, so it is held to what
//! a check uploads.
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use std::path::{Component, Path, PathBuf};

/// An example of a custom question: a file's text that breaks or keeps its rule, written inline or read from a file.
#[derive(Deserialize)]
#[cfg_attr(
    test,
    derive(schemars::JsonSchema),
    schemars(rename = "QuestionExample")
)]
#[serde(deny_unknown_fields)]
pub struct ExampleSpec {
    /// The example's text: a file's content, or for a `hunk` question the lines of a diff (`+` added, `-` removed, a space unchanged). Use `code` or `file`.
    #[serde(default)]
    pub code: Option<String>,
    /// A file holding the example, relative to the repository root. It is uploaded as a checked file is: inside the repository, not hidden (except under .jevgate/questions/), and within upload_allow and upload_deny. Use `code` or `file`.
    #[serde(default)]
    pub file: Option<String>,
    /// The file the example stands for, relative to the repository root: its language, and the path Jev reads. It must match the question's `paths`. Required with `code`; default: `file`.
    #[serde(default)]
    pub path: Option<String>,
}

/// Whether a check must find an example or clear it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Expected {
    /// Code that breaks the rule: its answer must reach the threshold.
    Failing,
    /// Code that keeps the rule: its answer must stay below the threshold.
    Passing,
}

impl Expected {
    pub fn name(self) -> &'static str {
        match self {
            Self::Failing => "failing",
            Self::Passing => "passing",
        }
    }
}

/// Where an example's text is.
#[derive(Debug)]
pub enum Text {
    Inline(String),
    /// A file, relative to the repository root.
    File(PathBuf),
}

/// A validated example.
#[derive(Debug)]
pub struct Example {
    pub expected: Expected,
    /// Its place among the question's examples of its kind, from 1.
    pub number: usize,
    pub text: Text,
    /// The file it stands for, relative to the repository root.
    pub path: PathBuf,
}

impl Example {
    /// Its text: inline, or read from its file as a checked file is read,
    /// once the upload boundary permits the file.
    pub fn read(
        &self,
        root: &Path,
        boundary: &crate::boundary::Boundary,
        limit: u64,
    ) -> Result<String> {
        let file = match &self.text {
            Text::Inline(code) => return Ok(code.clone()),
            Text::File(file) => file,
        };
        ensure!(
            boundary.permits(file),
            "{} is outside upload_allow or inside upload_deny; allow it, or write the example as `code`",
            file.display()
        );
        // Joined a component at a time: a canonical Windows root is a `\\?\`
        // path, where `/` is not a separator.
        let path = file
            .components()
            .fold(root.to_path_buf(), |path, part| path.join(part));
        crate::inventory::read_source(&path, limit)
            .with_context(|| format!("Cannot read {}", file.display()))
    }
}

/// A question's examples, failing then passing, each checked: exactly one
/// of `code` and `file`, a `path` with code, relative paths that stay in
/// the repository, a file that may be uploaded, and a path the question
/// applies to (`applies`), since a check never asks it elsewhere.
pub(super) fn validate(
    (failing, passing): (&[ExampleSpec], &[ExampleSpec]),
    applies: impl Fn(&Path) -> bool,
) -> Result<Vec<Example>, String> {
    let mut examples = Vec::new();
    for (expected, specs) in [(Expected::Failing, failing), (Expected::Passing, passing)] {
        for (at, spec) in specs.iter().enumerate() {
            let label = format!("{} example {}", expected.name(), at + 1);
            let example = checked(expected, at + 1, spec)
                .and_then(|example| {
                    if applies(&example.path) {
                        Ok(example)
                    } else {
                        Err(format!(
                            "its path {} is outside the question's paths; set `path` to a file the question applies to",
                            example.path.display()
                        ))
                    }
                })
                .map_err(|problem| format!("{label}: {problem}"))?;
            examples.push(example);
        }
    }
    Ok(examples)
}

/// One example's fields, checked.
fn checked(expected: Expected, number: usize, spec: &ExampleSpec) -> Result<Example, String> {
    let (text, path) = match (&spec.code, &spec.file) {
        (Some(code), None) if code.trim().is_empty() => return Err("`code` is empty".into()),
        (Some(code), None) => {
            let path = spec
                .path
                .clone()
                .ok_or("`path` is required with `code`: the file the example stands for")?;
            (Text::Inline(code.clone()), path)
        }
        (None, Some(file)) => {
            let path = spec.path.clone().unwrap_or_else(|| file.clone());
            let file = relative(file, "file")?;
            readable(&file)?;
            (Text::File(file), path)
        }
        _ => return Err("give either `code` or `file`".into()),
    };
    Ok(Example {
        expected,
        number,
        text,
        path: relative(&path, "path")?,
    })
}

/// A path relative to the repository root that stays inside it, written
/// with `/` so a configuration reads the same on every system.
fn relative(value: &str, field: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(value);
    let inside = !value.is_empty()
        && !value.contains('\\')
        && path.components().all(|c| matches!(c, Component::Normal(_)));
    if inside {
        Ok(path)
    } else {
        Err(format!(
            "`{field}` {value:?} must be a path relative to the repository root, with `/` between its parts and no `..`"
        ))
    }
}

/// Whether an example file may be uploaded: no hidden part outside the
/// question directory, no dependency or build directory and no credential
/// name, as for `--context` files. A pull request that adds a question
/// file naming `.git/config`, where CI checkouts keep their token, would
/// otherwise send it.
fn readable(file: &Path) -> Result<(), String> {
    let own = file.strip_prefix(super::DIRECTORY).unwrap_or(file);
    crate::context::ensure_visible_path(own).map_err(|_| {
        format!(
            "{} is hidden, in a dependency or build directory, or a credential; keep example files elsewhere, or under {}/",
            file.display(),
            super::DIRECTORY
        )
    })
}
