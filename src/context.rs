use super::{
    inventory::read_source,
    options::CheckArgs,
    schema::{ContextFile, hash},
};
use crate::{boundary::Boundary, config::ConfigContext};
use anyhow::{Context, Result, ensure};
use std::path::{Component, Path};

#[derive(Clone)]
pub struct ContextInput {
    pub file: ContextFile,
    pub source: String,
}

pub(crate) fn package(path: &Path, source: String, reason: &str) -> ContextInput {
    let lines = source.lines().count().max(1);
    ContextInput {
        file: ContextFile {
            path: path.into(),
            source_hash: hash(source.as_bytes()),
            start_line: 1,
            end_line: lines,
            reason: reason.into(),
            dependent_rules: super::catalog::rules()
                .iter()
                .map(|r| r.id.into())
                .collect(),
            complete: true,
            line_ranges: vec![crate::schema::SourceRange {
                start_line: 1,
                end_line: lines,
            }],
            selection: "whole-file".into(),
        },
        source,
    }
}

pub fn collect(args: &CheckArgs, context: &ConfigContext) -> Result<Vec<ContextInput>> {
    let boundary = Boundary::new(&context.config)?;
    let mut inputs = Vec::new();
    let mut bytes = 0;
    for name in &args.context {
        let path = context.input_path(name);
        let relative = &crate::discovery::relative(&path, &context.root)
            .context("Context must be inside the repository root")?;
        ensure_visible_path(relative)?;
        ensure!(
            boundary.permits(relative),
            "Context is outside the configured upload boundary"
        );
        let canonical = path.canonicalize().context("Cannot resolve context file")?;
        ensure!(
            canonical == path,
            "Context paths must not contain symlinks or parent traversal"
        );
        if inputs
            .iter()
            .any(|i: &ContextInput| i.file.path == *relative)
        {
            continue;
        }
        ensure!(
            inputs.len() < 16,
            "At most 16 explicit context files are allowed"
        );
        let source = read_source(&path, args.max_context_bytes)?;
        bytes += source.len() as u64;
        ensure!(
            bytes <= args.max_context_bytes,
            "Context exceeds --max-context-bytes; nothing was truncated or sent"
        );
        inputs.push(package(relative, source, "explicit contract/context"));
    }
    inputs.sort_by(|a, b| a.file.path.cmp(&b.file.path));
    Ok(inputs)
}

pub fn ensure_visible_path(path: &Path) -> Result<()> {
    for component in path.components() {
        let Component::Normal(name) = component else {
            anyhow::bail!("Context must use a normal path inside the root");
        };
        let name = name.to_string_lossy();
        ensure!(
            !name.starts_with('.') && !crate::discovery::SKIPPED_DIRS.contains(&name.as_ref()),
            "Hidden, dependency and build-output paths cannot be context"
        );
        ensure!(
            !name.ends_with(".pem") && !name.ends_with(".key") && name != "credentials",
            "Credential paths cannot be context"
        );
    }
    Ok(())
}
