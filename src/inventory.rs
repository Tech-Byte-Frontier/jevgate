use super::{
    options::CheckArgs,
    schema::{FileResult, Status, hash},
};
use crate::{
    config::{Boundary, ConfigContext},
    discovery,
};
use anyhow::{Context, Result, ensure};
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Clone)]
pub struct Input {
    pub result: FileResult,
    pub source: Option<String>,
    pub context: Vec<super::context::ContextInput>,
}

pub fn scope(args: &CheckArgs, context: &ConfigContext) -> Result<Vec<PathBuf>> {
    args.paths
        .iter()
        .map(|p| {
            let path = context
                .input_path(p)
                .canonicalize()
                .with_context(|| format!("Cannot resolve scope {}", p.display()))?;
            ensure!(
                path.starts_with(&context.root),
                "JevGate scope must be inside the repository root"
            );
            Ok(path)
        })
        .collect()
}

pub fn collect(args: &CheckArgs, context: &ConfigContext, scope: &[PathBuf]) -> Result<Vec<Input>> {
    let changes = args
        .base
        .as_ref()
        .map(|b| crate::revision::Changes::load(&context.root, b))
        .transpose()?;
    let extra = super::context::collect(args, context)?;
    let classifier = discovery::Classifier::new(&context.config)?;
    let boundary = Boundary::new(&context.config)?;
    let walker = ignore::WalkBuilder::new(&context.root)
        .standard_filters(true)
        .parents(false)
        .require_git(false)
        .follow_links(false)
        .filter_entry(|e| {
            e.depth() == 0
                || !e.file_type().is_some_and(|t| t.is_dir())
                || !discovery::SKIPPED_DIRS.contains(&e.file_name().to_str().unwrap_or_default())
        })
        .build();
    let mut paths = Vec::new();
    for entry in walker {
        let entry = entry.context("Failed while discovering Jev scope")?;
        let path = entry.path();
        if !entry.file_type().is_some_and(|t| t.is_file())
            || !(scope.is_empty() || scope.iter().any(|s| path.starts_with(s)))
        {
            continue;
        }
        let relative = path.strip_prefix(&context.root)?;
        if discovery::source(relative, &args.source_extension)
            && changes
                .as_ref()
                .is_none_or(|c| c.paths.contains_key(relative))
            && boundary.permits(relative)
            && super::context::ensure_visible_path(relative).is_ok()
        {
            paths.push((path.to_path_buf(), classifier.role(relative).to_string()));
        }
    }
    paths.sort_by(|a, b| a.0.cmp(&b.0));
    let inputs: Vec<Input> = paths
        .into_iter()
        .map(|file| load(file, args, context, &extra))
        .collect::<Result<_>>()?;
    Ok(inputs)
}

fn load(
    (path, role): (PathBuf, String),
    args: &CheckArgs,
    context: &ConfigContext,
    extra: &[super::context::ContextInput],
) -> Result<Input> {
    let relative = path
        .strip_prefix(&context.root)
        .context("Source outside root")?;
    let mut result = FileResult {
        role_assessment: None,
        path: relative.into(),
        contains_tests: role == "test",
        role: role.clone(),
        source_hash: String::new(),
        catalog_hash: hash(
            &serde_json::to_vec(&(
                crate::schema::RUBRIC,
                crate::schema::COMPOSITION,
                crate::roles::VERSION,
                crate::cascade::VERSION,
                crate::file_kind::VERSION,
                args.roles_only,
                args.include_tests,
                &args.model,
                &args.rules,
            ))
            .unwrap(),
        ),
        context_complete: true,
        context_expanded: false,

        context_limitations: Vec::new(),
        context_requests: Vec::new(),
        content_identity: String::new(),
        symbols: Vec::new(),
        semantic_size: 0,
        input_tokens: 0,
        output_tokens: 0,
        context_files: extra
            .iter()
            .filter(|i| i.file.path != relative)
            .map(|i| i.file.clone())
            .collect(),
        syntax_checked: false,
        status: Status::Pending,
        cached: false,
        evaluated_at: None,
        model: None,
        elapsed_ms: 0,
        dimensions: BTreeMap::new(),
        file_dimensions: BTreeMap::new(),

        findings: Vec::new(),
        error: None,
        classification: None,
    };
    if !matches!(role.as_str(), "source" | "test") {
        result.status = Status::Skipped;
        result.error = Some(crate::file_kind::excluded_reason(&role).into());
        result.classification = Some(crate::file_kind::excluded(&role, relative));
        return Ok(Input {
            result,
            source: None,

            context: Vec::new(),
        });
    }
    if std::fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.len() > args.max_file_bytes) {
        return over_read_cap(result, relative, &path, args.max_file_bytes);
    }
    let source = read_source(&path, args.max_file_bytes);
    match source {
        Ok(source) => {
            result.source_hash = hash(source.as_bytes());
            result.content_identity = super::locations::identity(&path, &source);
            result.semantic_size = super::locations::semantic_size(&path, &source);
            result.symbols = super::locations::collect(&path, &source, &context.root)
                .ok()
                .map(|(_, s)| s.into_iter().map(|(name, _)| name).collect())
                .unwrap_or_default();
            Ok(Input {
                result,
                source: Some(source),

                context: extra
                    .iter()
                    .filter(|i| i.file.path != relative)
                    .cloned()
                    .collect(),
            })
        }
        Err(error) => {
            result.status = Status::Error;
            result.error = Some(error.to_string());
            Ok(Input {
                result,
                source: None,

                context: Vec::new(),
            })
        }
    }
}

const LOCAL_PARSE_MAX: u64 = 1_048_576;

fn over_read_cap(
    mut result: FileResult,
    relative: &std::path::Path,
    path: &std::path::Path,
    cap: u64,
) -> Result<Input> {
    let parsed = match read_source(path, LOCAL_PARSE_MAX) {
        Ok(source) => Some(source),
        Err(error) => {
            let message = error.to_string();
            if !message.contains("exceeds") && !message.contains("grew beyond") {
                result.status = Status::Error;
                result.error = Some(message);
                return Ok(Input {
                    result,
                    source: None,
                    context: Vec::new(),
                });
            }
            None
        }
    };
    let len = parsed.as_ref().map(String::len).unwrap_or_else(|| {
        std::fs::symlink_metadata(path)
            .map(|metadata| metadata.len() as usize)
            .unwrap_or(0)
    });
    if let Some(source) = &parsed {
        result.source_hash = hash(source.as_bytes());
        result.content_identity = super::locations::identity(relative, source);
        result.semantic_size = super::locations::semantic_size(relative, source);
    }
    result.status = Status::NeedsContext;
    result.classification = Some(super::file_kind::unsent(
        relative,
        parsed.as_deref().unwrap_or(""),
        &format!(
            "{len} bytes exceeds the {cap}-byte read cap, so the complete source was not sent."
        ),
    ));
    Ok(Input {
        result,
        source: None,
        context: Vec::new(),
    })
}

pub(super) fn read_source(path: &std::path::Path, limit: u64) -> Result<String> {
    use std::io::Read;
    let metadata = std::fs::symlink_metadata(path)?;
    ensure!(
        path.canonicalize()? == path,
        "Source paths must not contain symlinks"
    );
    ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "Source symlinks and special files are not uploaded"
    );
    ensure!(
        metadata.len() <= limit,
        "File exceeds --max-file-bytes; no source was truncated or sent"
    );
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(limit + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= limit,
        "File grew beyond --max-file-bytes"
    );
    ensure!(!bytes.contains(&0), "Source contains binary NUL bytes");
    String::from_utf8(bytes).context("Source is not UTF-8")
}

pub fn fingerprint(inputs: &[Input]) -> String {
    let values: Vec<_> = inputs
        .iter()
        .map(|i| {
            (
                &i.result.path,
                &i.result.role,
                &i.result.source_hash,
                &i.result.catalog_hash,
                &i.result.context_files,
                &i.result.context_complete,
                &i.result.context_limitations,
                &i.result.error,
            )
        })
        .collect();
    hash(&serde_json::to_vec(&values).expect("serializable inventory"))
}
