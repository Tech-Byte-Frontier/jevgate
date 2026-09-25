use super::{
    options::CheckArgs,
    schema::{FileResult, Status, hash},
};
use crate::{boundary::Boundary, config::ConfigContext, discovery};
use anyhow::{Context, Result, ensure};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

#[derive(Clone)]
pub struct Input {
    pub result: FileResult,
    pub source: Option<String>,
    pub context: Vec<super::context::ContextInput>,
    /// For an agent instruction file, the repository's documentation evidence.
    pub repository: Option<std::sync::Arc<crate::docs::Repository>>,
    /// With access control, for a SpacetimeDB module file (it imports
    /// `spacetimedb/server`), its package and framework version.
    pub framework: Option<Framework>,
    /// For application and test source, the package it belongs to.
    pub package: Option<crate::packages::Package>,
    /// For a Python module that may be Django settings, with unsafe settings
    /// judged: the lines elsewhere in the repository that select it as the
    /// settings to run with.
    pub settings_selected_by: Vec<crate::analysis::django::Selection>,
    /// For a Python file, with injection judged: the Django templates it
    /// names that write values without escaping them.
    pub templates: Vec<crate::analysis::django::Template>,
}

/// A SpacetimeDB module's package: the directory of the `package.json` that
/// declares `spacetimedb` (else the module file's directory), and the declared
/// version without range operators, empty when none is declared.
#[derive(Clone, Debug, PartialEq)]
pub struct Framework {
    pub root: PathBuf,
    pub version: String,
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

/// Files under `root`, honoring ignore files and skipping dependency and build directories.
pub(crate) fn walker(root: &std::path::Path) -> ignore::Walk {
    ignore::WalkBuilder::new(root)
        .standard_filters(true)
        .parents(false)
        .require_git(false)
        .follow_links(false)
        .filter_entry(|e| {
            e.depth() == 0
                || !e.file_type().is_some_and(|t| t.is_dir())
                || !discovery::SKIPPED_DIRS.contains(&e.file_name().to_str().unwrap_or_default())
        })
        .build()
}

pub fn collect(args: &CheckArgs, context: &ConfigContext, scope: &[PathBuf]) -> Result<Vec<Input>> {
    let changes = args
        .base
        .as_ref()
        .map(|b| crate::revision::Changes::load(&context.root, b))
        .transpose()?;
    let extra = super::context::collect(args, context)?;
    let boundary = Boundary::new(&context.config)?;
    let in_scope = |relative: &Path| {
        let path = context.root.join(relative);
        scope.is_empty() || scope.iter().any(|s| path.starts_with(s))
    };
    let changed = |relative: &Path| {
        changes
            .as_ref()
            .is_none_or(|c| c.paths.contains_key(relative))
    };
    let selected = |relative: &Path| in_scope(relative) && changed(relative);
    // Only the documentation or configuration rules selected: source files are not collected.
    let mut inputs: Vec<Input> = if args.code_rules() {
        source_paths(args, context, &boundary, &selected)?
            .into_iter()
            .map(|file| load(file, args, context, &extra))
            .collect::<Result<_>>()?
    } else {
        Vec::new()
    };
    keep_module_packages(args, &mut inputs);
    if args.enabled(crate::catalog::UNSAFE_SETTINGS) {
        select_settings(context, &boundary, &mut inputs);
    }
    if args.enabled(crate::catalog::INJECTION) {
        unescaped_templates(context, &boundary, &mut inputs);
    }
    if args.documentation() {
        add_documents(args, context, &boundary, &selected, &mut inputs)?;
    }
    // SQL is also a source extension, so the code rules' walk may have listed
    // the file already; the configuration rule's role replaces that entry.
    for (relative, role) in configuration_files(args, context, &in_scope, &changed)? {
        let input = bounded(&relative, role, args, context, &boundary)?;
        match inputs.iter_mut().find(|i| i.result.path == relative) {
            Some(existing) => *existing = input,
            None => inputs.push(input),
        }
    }
    Ok(inputs)
}

/// Give each Python module that may be Django settings the lines of the
/// repository that select it (`DJANGO_SETTINGS_MODULE=…`), read only from
/// files the upload patterns permit. The repository is searched only when
/// such a module was collected.
fn select_settings(context: &ConfigContext, boundary: &Boundary, inputs: &mut [Input]) {
    let candidate = |input: &Input| {
        let path = &input.result.path;
        path.extension().is_some_and(|e| e == "py")
            && (path.iter().any(|part| {
                part.to_str()
                    .is_some_and(|p| p.trim_end_matches(".py").contains("settings"))
            }) || input.source.as_deref().is_some_and(|source| {
                source.contains("INSTALLED_APPS") || source.contains(" import *")
            }))
    };
    if !inputs.iter().any(candidate) {
        return;
    }
    let mut selections = Vec::new();
    // `.github` is hidden, so the walker does not reach CI workflows.
    let workflows = std::fs::read_dir(context.root.join(".github/workflows"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path());
    let walked = walker(&context.root)
        .flatten()
        .filter(|e| e.file_type().is_some_and(|t| t.is_file()))
        .map(|e| e.path().to_path_buf());
    for path in walked.chain(workflows) {
        let Ok(relative) = path.strip_prefix(&context.root) else {
            continue;
        };
        if !crate::analysis::django::selection_file(relative)
            || !boundary.permits(relative)
            || std::fs::metadata(&path)
                .is_ok_and(|m| m.len() > crate::analysis::django::SELECTION_FILE_BYTES)
        {
            continue;
        }
        if let Ok(text) = std::fs::read_to_string(&path) {
            selections.extend(crate::analysis::django::selections_in(relative, &text));
        }
    }
    selections.sort_by(|a, b| (&a.file, a.line).cmp(&(&b.file, b.line)));
    for input in inputs.iter_mut().filter(|i| candidate(i)) {
        input.settings_selected_by =
            crate::analysis::django::selected_by(&input.result.path, &selections)
                .into_iter()
                .cloned()
                .collect();
    }
}

/// Give each Python file the Django templates it names that write values
/// without escaping, read only from files the upload patterns permit. The
/// repository is searched only when a Python file names a template.
fn unescaped_templates(context: &ConfigContext, boundary: &Boundary, inputs: &mut [Input]) {
    let candidate = |input: &Input| {
        input.result.path.extension().is_some_and(|e| e == "py")
            && input
                .source
                .as_deref()
                .is_some_and(|source| source.contains(".html"))
    };
    if !inputs.iter().any(candidate) {
        return;
    }
    let mut templates = Vec::new();
    for entry in walker(&context.root).flatten() {
        let path = entry.path();
        let Ok(relative) = path.strip_prefix(&context.root) else {
            continue;
        };
        let Some(name) = crate::analysis::django::template_name(relative) else {
            continue;
        };
        if !entry.file_type().is_some_and(|t| t.is_file())
            || !boundary.permits(relative)
            || std::fs::metadata(path)
                .is_ok_and(|m| m.len() > crate::analysis::django::SELECTION_FILE_BYTES)
        {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        let unescaped = crate::analysis::django::unescaped_lines(&text);
        if !unescaped.is_empty() {
            templates.push(crate::analysis::django::Template {
                name,
                path: relative.to_path_buf(),
                unescaped,
            });
        }
    }
    templates.sort_by(|a, b| a.path.cmp(&b.path));
    for input in inputs.iter_mut().filter(|i| candidate(i)) {
        let source = input.source.as_deref().unwrap_or("");
        input.templates = crate::analysis::django::rendered(source, &templates)
            .into_iter()
            .cloned()
            .collect();
    }
}

/// Application source and tests in scope, with their roles.
fn source_paths(
    args: &CheckArgs,
    context: &ConfigContext,
    boundary: &Boundary,
    selected: &dyn Fn(&Path) -> bool,
) -> Result<Vec<(PathBuf, String)>> {
    let classifier = discovery::Classifier::new(&context.config)?;
    let mut paths = Vec::new();
    for entry in walker(&context.root) {
        let entry = entry.context("Failed while discovering Jev scope")?;
        let path = entry.path();
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let relative = path.strip_prefix(&context.root)?;
        if discovery::source(relative, &args.source_extension)
            && selected(relative)
            && boundary.permits(relative)
            && super::context::ensure_visible_path(relative).is_ok()
        {
            paths.push((path.to_path_buf(), classifier.role(relative).to_string()));
        }
    }
    paths.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(paths)
}

/// SQL files for the access-control rule and workflows for the workflow rule.
/// Unchanged SQL is kept as context: earlier migrations decide the final
/// state of changed ones.
fn configuration_files(
    args: &CheckArgs,
    context: &ConfigContext,
    in_scope: &dyn Fn(&Path) -> bool,
    changed: &dyn Fn(&Path) -> bool,
) -> Result<Vec<(PathBuf, &'static str)>> {
    let mut files = Vec::new();
    if args.enabled(crate::catalog::ACCESS_CONTROL) {
        for entry in walker(&context.root).flatten() {
            let relative = entry.path().strip_prefix(&context.root)?;
            if entry.file_type().is_some_and(|t| t.is_file())
                && relative.extension().is_some_and(|e| e == "sql")
                && in_scope(relative)
            {
                let role = if changed(relative) { SQL } else { SQL_CONTEXT };
                files.push((relative.to_path_buf(), role));
            }
        }
    }
    if args.enabled(crate::catalog::WORKFLOWS) {
        // `.github` is hidden, so the walker does not reach it.
        let directory = context.root.join(".github/workflows");
        for entry in std::fs::read_dir(&directory)
            .into_iter()
            .flatten()
            .flatten()
        {
            let path = entry.path();
            let relative = path.strip_prefix(&context.root)?;
            if path.is_file()
                && path.extension().is_some_and(|e| e == "yml" || e == "yaml")
                && in_scope(relative)
                && changed(relative)
            {
                files.push((relative.to_path_buf(), WORKFLOW));
            }
        }
    }
    files.sort();
    Ok(files)
}

/// A file read whole, or listed as skipped when the upload patterns exclude
/// it, so an allow list that leaves it out is visible.
fn bounded(
    relative: &Path,
    role: &str,
    args: &CheckArgs,
    context: &ConfigContext,
    boundary: &Boundary,
) -> Result<Input> {
    if boundary.permits(relative) {
        return load_document(relative, role, args, &context.root.join(relative));
    }
    let mut result = pending_result(relative, role, args, &[]);
    result.status = Status::Skipped;
    result.error = Some("Outside upload_allow/upload_deny; not judged.".into());
    Ok(bare_input(result))
}

/// Agent instruction files and project docs the selected rules judge.
fn add_documents(
    args: &CheckArgs,
    context: &ConfigContext,
    boundary: &Boundary,
    selected: &dyn Fn(&Path) -> bool,
    inputs: &mut Vec<Input>,
) -> Result<()> {
    let repository = std::sync::Arc::new(crate::docs::scan(&context.root)?);
    let instructions = repository
        .readers
        .keys()
        .filter(|_| args.enabled(crate::catalog::AGENT_CONTEXT) || cross_document(args))
        .map(|p| (p, INSTRUCTIONS));
    let docs = repository
        .docs
        .iter()
        .filter(|_| args.enabled(crate::catalog::LARGE_DOCS) || cross_document(args))
        .map(|p| (p, DOCS));
    for (relative, role) in instructions.chain(docs) {
        let wanted = selected(relative)
            && (role == DOCS || repository.judged(relative))
            && !inputs.iter().any(|i| i.result.path == *relative);
        if wanted {
            let mut input = bounded(relative, role, args, context, boundary)?;
            if input.result.status != Status::Skipped {
                input.repository = Some(repository.clone());
            }
            inputs.push(input);
        }
    }
    Ok(())
}

/// A documentation file, read whole; hidden and ignored paths are allowed.
fn load_document(
    relative: &std::path::Path,
    role: &str,
    args: &CheckArgs,
    path: &std::path::Path,
) -> Result<Input> {
    let mut result = pending_result(relative, role, args, &[]);
    if std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.len() > args.max_file_bytes) {
        return over_read_cap(result, relative, path, args.max_file_bytes);
    }
    Ok(match read_source(path, args.max_file_bytes) {
        Ok(source) => {
            result.source_hash = hash(source.as_bytes());
            result.content_identity = result.source_hash.clone();
            Input {
                result,
                source: Some(source),
                context: Vec::new(),
                repository: None,
                framework: None,
                package: None,
                settings_selected_by: Vec::new(),
                templates: Vec::new(),
            }
        }
        Err(error) => error_input(result, error),
    })
}

/// Whether a rule that compares or checks every kind of document is selected.
fn cross_document(args: &CheckArgs) -> bool {
    args.enabled(crate::catalog::DOC_STALENESS) || args.enabled(crate::catalog::DOC_DUPLICATION)
}

/// The role of an agent instruction file.
pub const INSTRUCTIONS: &str = "instructions";
/// SQL judged by the access-control rule.
pub const SQL: &str = "sql";
/// Unchanged SQL read only for the final state of changed migrations.
pub const SQL_CONTEXT: &str = "sql-context";
/// A GitHub Actions workflow.
pub const WORKFLOW: &str = "workflow";
/// The role of project documentation such as a README or a docs page.
pub const DOCS: &str = "docs";

fn load(
    (path, role): (PathBuf, String),
    args: &CheckArgs,
    context: &ConfigContext,
    extra: &[super::context::ContextInput],
) -> Result<Input> {
    let relative = path
        .strip_prefix(&context.root)
        .context("Source outside root")?;
    let mut result = pending_result(relative, &role, args, extra);
    if !matches!(role.as_str(), "source" | "test") {
        return Ok(excluded(result, &role, relative));
    }
    if role == "source" && discovery::vendored(&path, None) {
        return Ok(recast(result, "vendored", relative));
    }
    if std::fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.len() > args.max_file_bytes) {
        // A copied library or build output is excluded whatever its size.
        let copied = read_source(&path, LOCAL_PARSE_MAX)
            .ok()
            .and_then(|source| not_written_here(&path, &role, &source));
        return Ok(match copied {
            Some(kind) => recast(result, kind, relative),
            None => over_read_cap(result, relative, &path, args.max_file_bytes)?,
        });
    }
    let source = read_source(&path, args.max_file_bytes);
    match source {
        Ok(source) => {
            if let Some(kind) = not_written_here(&path, &role, &source) {
                return Ok(recast(result, kind, relative));
            }
            result.source_hash = hash(source.as_bytes());
            result.content_identity = super::locations::identity(&path, &source);
            result.semantic_size = super::locations::semantic_size(&path, &source);
            result.symbols = super::locations::collect(&path, &source, &context.root)
                .ok()
                .map(|(_, s)| s.into_iter().map(|(name, _)| name).collect())
                .unwrap_or_default();
            let framework = (args.enabled(crate::catalog::ACCESS_CONTROL)
                && crate::units::spacetimedb_module(&source))
            .then(|| spacetimedb_package(&context.root, relative));
            Ok(Input {
                result,
                source: Some(source),

                context: extra
                    .iter()
                    .filter(|i| i.file.path != relative)
                    .cloned()
                    .collect(),
                repository: None,
                framework,
                package: crate::packages::package(&context.root, relative),
                settings_selected_by: Vec::new(),
                templates: Vec::new(),
            })
        }
        // Binary and non-UTF-8 files are reported and skipped; they never make a run incomplete.
        Err(error) if not_text(&error) => {
            result.status = Status::Skipped;
            result.error = Some(format!("{error}; this file was not judged."));
            Ok(bare_input(result))
        }
        Err(error) => Ok(error_input(result, error)),
    }
}

/// A pending result for one discovered file, before its source is read.
fn pending_result(
    relative: &std::path::Path,
    role: &str,
    args: &CheckArgs,
    extra: &[super::context::ContextInput],
) -> FileResult {
    FileResult {
        path: relative.into(),
        contains_tests: role == "test",
        role: role.into(),
        source_hash: String::new(),
        catalog_hash: hash(
            &serde_json::to_vec(&(
                crate::schema::RUBRIC,
                crate::schema::COMPOSITION,
                crate::units::questions::VERSION,
                crate::file_kind::VERSION,
                args.include_tests,
                args.model(),
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
        judgments: Vec::new(),
        findings: Vec::new(),
        error: None,
        classification: None,
    }
}

/// Build output, or a library copied into the repository, found from a
/// file's content: the role it takes instead of the one its path gave.
fn not_written_here(path: &std::path::Path, role: &str, source: &str) -> Option<&'static str> {
    if discovery::generated_source(source) {
        Some("generated")
    } else if role == "source" && discovery::vendored(path, Some(source)) {
        Some("vendored")
    } else {
        None
    }
}

/// A file found to be build output or a copied library once read.
fn recast(mut result: FileResult, role: &str, relative: &std::path::Path) -> Input {
    result.role = role.into();
    result.contains_tests = false;
    excluded(result, role, relative)
}

/// A file outside the reviewed roles, skipped with its reason.
fn excluded(mut result: FileResult, role: &str, relative: &std::path::Path) -> Input {
    result.status = Status::Skipped;
    result.error = Some(crate::file_kind::excluded_reason(role).into());
    result.classification = Some(crate::file_kind::excluded(role, relative));
    bare_input(result)
}

fn not_text(error: &anyhow::Error) -> bool {
    error.downcast_ref::<std::string::FromUtf8Error>().is_some()
        || error.to_string().contains("NUL bytes")
}

fn bare_input(result: FileResult) -> Input {
    Input {
        result,
        source: None,
        context: Vec::new(),
        repository: None,
        framework: None,
        package: None,
        settings_selected_by: Vec::new(),
        templates: Vec::new(),
    }
}

/// Access control reads application source only for SpacetimeDB modules:
/// alone among the code rules, it keeps just the files of their packages.
fn keep_module_packages(args: &CheckArgs, inputs: &mut Vec<Input>) {
    let only_access = args.rules.iter().all(|rule| {
        rule == crate::catalog::ACCESS_CONTROL
            || !crate::catalog::find(rule).is_some_and(|r| args.code_rules_include(r.key))
    });
    if !only_access {
        return;
    }
    let roots: Vec<PathBuf> = inputs
        .iter()
        .filter_map(|i| Some(i.framework.as_ref()?.root.clone()))
        .collect();
    inputs.retain(|input| roots.iter().any(|root| input.result.path.starts_with(root)));
}

/// The package of a SpacetimeDB module file: the nearest `package.json` above
/// it that declares `spacetimedb`. Only the version is read from it, never
/// the manifest's other text.
fn spacetimedb_package(root: &Path, relative: &Path) -> Framework {
    for directory in relative.ancestors().skip(1) {
        if let Ok(text) = read_source(&root.join(directory).join("Cargo.toml"), LOCAL_PARSE_MAX)
            && let Ok(table) = text.parse::<toml::Table>()
            && let Some(dependency) = table.get("dependencies").and_then(|d| d.get("spacetimedb"))
        {
            let version = dependency
                .as_str()
                .or_else(|| dependency.get("version")?.as_str())
                .unwrap_or("");
            return Framework {
                root: directory.to_path_buf(),
                version: version.trim_start_matches(['^', '~', '=', 'v', ' ']).into(),
            };
        }
        let manifest = root.join(directory).join("package.json");
        let Ok(text) = read_source(&manifest, LOCAL_PARSE_MAX) else {
            continue;
        };
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };
        for section in ["dependencies", "devDependencies", "peerDependencies"] {
            if let Some(version) = json[section]["spacetimedb"].as_str() {
                return Framework {
                    root: directory.to_path_buf(),
                    version: version.trim_start_matches(['^', '~', '=', 'v', ' ']).into(),
                };
            }
        }
    }
    Framework {
        root: relative.parent().unwrap_or(Path::new("")).to_path_buf(),
        version: String::new(),
    }
}

fn error_input(mut result: FileResult, error: impl ToString) -> Input {
    result.status = Status::Error;
    result.error = Some(error.to_string());
    bare_input(result)
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
                return Ok(error_input(result, message));
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
    Ok(bare_input(result))
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
