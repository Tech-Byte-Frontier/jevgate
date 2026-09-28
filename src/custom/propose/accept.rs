//! `jevgate rules accept`: a proposal a person read and edited, checked as
//! a question file and moved into `.jevgate/questions/`. A question file
//! that does not load stops every command, so each one is checked before
//! any is moved.
use super::PROPOSALS;
use crate::{
    config::{Config, ConfigContext},
    custom::{self, Kind, Question},
    schema::Strength,
};
use anyhow::{Result, bail, ensure};
use std::path::{Path, PathBuf};

pub fn accept(ids: &[String], context: &ConfigContext) -> Result<u8> {
    let root = &context.root;
    let mut moves: Vec<(Question, PathBuf, PathBuf)> = Vec::new();
    for id in ids {
        ensure!(
            custom::valid_id(id),
            "Invalid proposal {id:?}: name it by its file in {PROPOSALS}/, without .toml"
        );
        ensure!(
            !moves.iter().any(|(question, ..)| question.id() == id),
            "{id} is named twice"
        );
        let from = custom::within(root, PROPOSALS).join(format!("{id}.toml"));
        ensure!(
            from.symlink_metadata().is_ok(),
            "No proposal {id} in {PROPOSALS}/; `jevgate rules propose` writes them"
        );
        let question = custom::read_file(&from, PathBuf::from(format!("{PROPOSALS}/{id}.toml")))?;
        if let Some(defined) = context.questions.iter().find(|q| q.rule == question.rule) {
            bail!(
                "{} is already defined in {}; rename the proposal's file to accept it",
                question.rule,
                defined.source.display()
            );
        }
        let to = custom::directory(root).join(format!("{id}.toml"));
        ensure!(
            !to.exists() && !to.is_symlink(),
            "{}/{id}.toml already exists",
            custom::DIRECTORY
        );
        moves.push((question, from, to));
    }
    crate::storage::real_directory(
        &custom::directory(root),
        "The questions directory must be a real directory",
    )?;
    for (question, from, to) in moves {
        std::fs::rename(&from, &to)?;
        say!("{}", accepted(&question));
        let file = Path::new(custom::DIRECTORY).join(format!("{}.toml", question.id()));
        if let Some(warning) = custom::ignored(root, &file) {
            note!("{warning}");
        }
        if context.config.leaves_out(&question.rule) {
            note!("{}", Config::unlisted_note(&question.rule));
        }
    }
    Ok(0)
}

/// What was accepted, and what it takes to be asked and to fail the gate.
/// A rule quoted as written can answer close to its threshold (ky's, at
/// 0.78 against 0.80 on a change that broke it, and 0.89 with one line of
/// guidance), so a question is told to get guidance and examples before it
/// enforces anything, and one that already fails the gate what it lacks.
pub(super) fn accepted(question: &Question) -> String {
    let mut lines = vec![format!(
        "Accepted {} into {}/{}.toml; commit it.",
        question.rule,
        custom::DIRECTORY,
        question.id()
    )];
    let (guided, exampled) = (question.guidance.is_some(), !question.examples.is_empty());
    let test = format!("run `jevgate rules test --rule {}`", question.rule);
    if question.level == Strength::Note {
        let add = match (guided, exampled) {
            (true, true) => String::new(),
            (true, false) => "add a [[failing]] and a [[passing]] example, ".into(),
            (false, true) => "add guidance, ".into(),
            (false, false) => "add guidance and a [[failing]] and a [[passing]] example, ".into(),
        };
        lines.push(format!(
            "  It is a note, which never fails the gate: {add}{test}, then set level = \"review\" to enforce it."
        ));
    } else if !(guided && exampled) {
        let lacks = match (guided, exampled) {
            (true, _) => "examples",
            (_, true) => "guidance",
            _ => "guidance or examples",
        };
        lines.push(format!(
            "  It fails the gate on its {}s without {lacks}, and a rule quoted alone can answer close to its threshold: add them and {test}.",
            crate::output::label(&question.level)
        ));
    }
    match question.unit {
        Kind::Hunk => lines.push("  It is asked only with --base.".into()),
        Kind::Test => lines.push("  It is asked only with --include-tests.".into()),
        _ => {}
    }
    lines.join("\n")
}
