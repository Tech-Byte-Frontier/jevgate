//! Agent instruction files and project documentation, read whole whether or
//! not they are hidden or ignored.
use super::*;

/// Agent instruction files and project docs the selected rules judge.
pub(super) fn add_documents(
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
pub(super) fn load_document(
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
