//! The question gallery: custom questions JevGate measured on real projects,
//! for conventions linters cannot check. Their files are compiled in, so
//! `jevgate rules add` works offline and writes the wording this version
//! measured; the docs' question gallery page gives each one's numbers.
use super::{DIRECTORY, Kind, Question, Spec};
use crate::{catalog, config::Config, output};
use anyhow::{Context, Result, anyhow, bail};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

/// A gallery question: its name, which is its file name and its id, the
/// file, and how it measured.
pub struct Entry {
    pub name: &'static str,
    text: &'static str,
    measured: Measured,
}

/// How often a gallery question was right when it fired, as the docs page
/// gives it: its right findings among those labeled, and on how many
/// projects it was asked.
struct Measured {
    right: usize,
    findings: usize,
    projects: usize,
}

macro_rules! entry {
    ($name:literal, $right:literal of $findings:literal on $projects:literal) => {
        Entry {
            name: $name,
            text: include_str!(concat!("../../gallery/", $name, ".toml")),
            measured: Measured {
                right: $right,
                findings: $findings,
                projects: $projects,
            },
        }
    };
}

/// Every gallery question, in the order the docs page lists them, with its
/// right findings of those labeled and its projects.
pub const ENTRIES: &[Entry] = &[
    entry!("todo-without-owner", 31 of 31 on 7),
    entry!("swallowed-errors", 14 of 17 on 6),
    entry!("resource-leak", 12 of 14 on 6),
    entry!("thin-handlers", 12 of 13 on 12),
    entry!("n-plus-one", 5 of 7 on 11),
];

impl Entry {
    /// What it catches: the first line of its file, `# NAME: summary`.
    pub fn summary(&self) -> &'static str {
        self.text
            .lines()
            .next()
            .and_then(|line| line.strip_prefix("# "))
            .and_then(|line| line.strip_prefix(self.name))
            .and_then(|line| line.strip_prefix(": "))
            .unwrap_or_default()
    }

    /// The question its file defines, validated as a question file is.
    pub fn question(&self) -> Result<Question> {
        let shown = self.path();
        let spec: Spec = toml::from_str(self.text)
            .with_context(|| format!("Invalid gallery question {}", self.name))?;
        super::validate(&spec, self.name, shown)
    }

    /// Where `rules add` writes it, relative to the repository root and
    /// written with `/`, as reports name files on every platform.
    fn path(&self) -> PathBuf {
        PathBuf::from(format!("{DIRECTORY}/{}.toml", self.name))
    }

    /// How it measured and whether it fails the gate at its level:
    /// `14 of 17 findings right on 6 projects; it fails the gate on its reviews`.
    fn standing(&self, question: &Question) -> String {
        let Measured {
            right,
            findings,
            projects,
        } = self.measured;
        let gate = match question.blocks().first() {
            Some(level) => format!("it fails the gate on its {}s", output::label(level)),
            None => "a note, it never fails the gate".into(),
        };
        format!("{right} of {findings} findings right on {projects} projects; {gate}")
    }
}

/// `jevgate rules add`: write the gallery questions `names` into the
/// questions directory of the repository at `root` and say what each asks
/// and how to try it. Only `jevgate.toml` is read, for its own questions:
/// the question files are not loaded, so one that no longer loads can be
/// replaced.
pub fn run(root: &Path, names: &[String], force: bool) -> Result<u8> {
    let config = Config::read(&root.join(crate::init::CONFIG_FILE), false)?;
    let added = add(root, &config.question, names, force)?;
    for (question, written) in &added {
        let needs = match question.unit {
            Kind::Test => ", asked with --include-tests",
            Kind::Hunk => ", asked with --base",
            _ => "",
        };
        let state = if *written { "Added" } else { "Already added:" };
        let entry = ENTRIES.iter().find(|entry| entry.name == question.id());
        say!(
            "{state} {} ({}{needs}) in {}: {}.",
            question.rule,
            question.asked(),
            question.source.display(),
            entry.map_or_else(String::new, |entry| entry.standing(question))
        );
    }
    if let Some(warning) = added
        .first()
        .and_then(|(question, _)| super::ignored(root, &question.source))
    {
        note!("{warning}");
    }
    for (question, _) in added.iter().filter(|(q, _)| config.leaves_out(&q.rule)) {
        note!("{}", Config::unlisted_note(&question.rule));
    }
    say!(
        "Commit {DIRECTORY}/. `--fail-on custom=report` reports every question without failing the gate while you try them."
    );
    say!("Next: jevgate check --rule custom --dry-run, then jevgate check");
    Ok(0)
}

/// Write the gallery questions `names` to the questions directory of the
/// repository at `root`: each with whether it was written, or already there
/// with the same text. A name among the `configured` questions of
/// `jevgate.toml` is refused, and one whose file holds other text unless
/// `force` replaces it; every name is checked before any file is written.
pub fn add(
    root: &Path,
    configured: &[Spec],
    names: &[String],
    force: bool,
) -> Result<Vec<(Question, bool)>> {
    let directory = super::directory(root);
    let mut chosen: Vec<(&Entry, bool)> = Vec::new();
    for name in names {
        let entry = ENTRIES
            .iter()
            .find(|entry| entry.name == name)
            .ok_or_else(|| {
                anyhow!("No gallery question {name}; `jevgate rules add --help` lists them")
            })?;
        if chosen.iter().any(|(earlier, _)| earlier.name == entry.name) {
            continue;
        }
        if configured
            .iter()
            .any(|spec| spec.id.as_deref() == Some(entry.name))
        {
            bail!(
                "{}/{} is already defined in {}; remove it there to add the gallery's",
                catalog::CUSTOM_GROUP,
                entry.name,
                crate::init::CONFIG_FILE
            );
        }
        let path = directory.join(format!("{name}.toml"));
        let present = path.exists() || path.is_symlink();
        let same = present && unchanged(&path, entry);
        if present && !same && !force {
            bail!(
                "{} already exists and differs from the gallery's; --force replaces it",
                entry.path().display()
            );
        }
        chosen.push((entry, !same));
    }
    crate::storage::state_directory(root)?;
    crate::storage::real_directory(
        &directory,
        "The questions directory must be a real directory",
    )?;
    let mut added = Vec::new();
    for (entry, write) in chosen {
        if write {
            replace(&directory.join(format!("{}.toml", entry.name)), entry.text)
                .with_context(|| format!("Cannot write {}", entry.path().display()))?;
        }
        added.push((entry.question()?, write));
    }
    Ok(added)
}

/// Whether the file at `path` holds exactly the gallery's text. A link or
/// anything else that is not a readable regular file differs.
fn unchanged(path: &Path, entry: &Entry) -> bool {
    crate::inventory::read_source(path, super::FILE_BYTES).is_ok_and(|text| text == entry.text)
}

/// Write `text` as a new file at `path`, removing what is there first: a
/// link is removed, never followed.
fn replace(path: &Path, text: &str) -> Result<()> {
    if path.exists() || path.is_symlink() {
        fs::remove_file(path)?;
    }
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(text.as_bytes())?;
    Ok(())
}

#[cfg(test)]
mod tests;
