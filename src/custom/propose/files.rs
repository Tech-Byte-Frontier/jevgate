//! The files `jevgate rules propose` reads, their candidate lines, and the
//! files their rules apply to.
use super::lines::{self, Line};
use crate::{
    boundary::Boundary,
    config::ConfigContext,
    docs::{
        Instructions,
        load::{Load, Reader},
    },
};
use anyhow::{Context, Result, anyhow};
use serde::Serialize;
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

/// Why a file the upload patterns exclude is not read.
const OUTSIDE: &str = "Outside upload_allow/upload_deny; not read.";

/// Why a translated instruction file is not read unless named: its rules
/// are its original's.
pub const TRANSLATION: &str = "A translation under a locale directory; name it to read it.";

/// One instruction file as it is asked about.
pub struct File {
    /// Relative to the repository root, with `/` between its parts.
    pub path: PathBuf,
    pub source_hash: String,
    pub lines: Vec<Line>,
    /// Globs of the files its rules apply to; none for every file.
    pub scope: Vec<String>,
}

/// A file named or found but not read, and why.
#[derive(Serialize)]
pub struct Skipped {
    pub path: PathBuf,
    pub reason: String,
}

/// The files to ask about and the ones left out: every agent instruction
/// file the agent-context rule judges, or those `paths` name.
pub fn read(
    paths: &[PathBuf],
    context: &ConfigContext,
    max_bytes: u64,
) -> Result<(Vec<File>, Vec<Skipped>)> {
    let instructions = crate::docs::instructions(&context.root)?;
    let boundary = Boundary::new(&context.config)?;
    let mut files = Vec::new();
    let (chosen, translations) = selected(paths, context, &instructions)?;
    let mut skipped: Vec<Skipped> = translations
        .into_iter()
        .map(|path| Skipped {
            path,
            reason: TRANSLATION.into(),
        })
        .collect();
    for path in chosen {
        if !boundary.permits(&path) {
            skipped.push(Skipped {
                path,
                reason: OUTSIDE.into(),
            });
            continue;
        }
        let absolute = context.root.join(path.components().collect::<PathBuf>());
        match crate::inventory::read_source(&absolute, max_bytes) {
            Ok(source) => files.push(File {
                source_hash: crate::schema::hash(source.as_bytes()),
                lines: lines::lines(&source),
                scope: scope(instructions.judged.get(&path).map_or(&[], Vec::as_slice)),
                path,
            }),
            Err(error) => skipped.push(Skipped {
                path,
                reason: format!("{error:#}"),
            }),
        }
    }
    Ok((files, skipped))
}

/// The files to read, relative to the root: each file `paths` names, the
/// instruction files under each directory it names, or, without paths,
/// every instruction file; and the translations those directories hold,
/// which are read only when named.
fn selected(
    paths: &[PathBuf],
    context: &ConfigContext,
    instructions: &Instructions,
) -> Result<(BTreeSet<PathBuf>, BTreeSet<PathBuf>)> {
    let mut chosen = BTreeSet::new();
    let mut directories = Vec::new();
    if paths.is_empty() {
        directories.push(PathBuf::new());
    }
    for named in paths {
        let path = context
            .input_path(named)
            .canonicalize()
            .with_context(|| format!("Cannot read {}", named.display()))?;
        let relative = crate::discovery::relative(&path, &context.root)
            .map_err(|_| anyhow!("{} is outside the repository", named.display()))?;
        if path.is_dir() {
            directories.push(relative);
        } else {
            chosen.insert(relative);
        }
    }
    let mut translations = BTreeSet::new();
    for directory in &directories {
        for (file, translated) in under(instructions, directory) {
            if translated {
                translations.insert(file.clone());
            } else {
                chosen.insert(file.clone());
            }
        }
    }
    translations.retain(|file| !chosen.contains(file));
    Ok((chosen, translations))
}

/// The instruction files under `directory`, those that could not be read
/// included, each with whether it is a translation outside it:
/// `docs/i18n/ja/CLAUDE.md` is one under the root, and none under
/// `docs/i18n/ja`.
fn under<'a>(
    instructions: &'a Instructions,
    directory: &'a Path,
) -> impl Iterator<Item = (&'a PathBuf, bool)> + 'a {
    let within = crate::docs::discover::translation(directory);
    let found = instructions.judged.keys().chain(&instructions.unread);
    found
        .filter(move |file| file.starts_with(directory))
        .map(move |file| {
            let translated = file
                .parent()
                .is_some_and(crate::docs::discover::translation);
            (file, translated && !within)
        })
}

/// Where a file's rules apply, as globs: when every harness that reads it
/// loads it for one directory (a nested `web/CLAUDE.md`) or for some files
/// (a Cursor rule's `globs`, a Copilot `applyTo`), those; otherwise every
/// file. A pattern that is no valid glob is left out.
fn scope(readers: &[Reader]) -> Vec<String> {
    let Some(first) = readers.first() else {
        return Vec::new();
    };
    if readers.iter().any(|reader| reader.load != first.load) {
        return Vec::new();
    }
    match &first.load {
        Load::Directory(directory) => vec![format!("{}/**", literal(directory))],
        Load::Files(globs) => globs
            .split(',')
            .map(str::trim)
            .filter(|glob| !glob.is_empty() && globset::Glob::new(glob).is_ok())
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

/// A directory as a glob that matches it literally, with `/` between its
/// parts: a Next.js route directory such as `app/[locale]` holds brackets.
fn literal(directory: &str) -> String {
    Path::new(directory)
        .iter()
        .map(|part| {
            part.to_string_lossy()
                .chars()
                .map(|c| match c {
                    '*' | '?' | '[' | '{' | '}' => format!("[{c}]"),
                    c => c.to_string(),
                })
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("/")
}
