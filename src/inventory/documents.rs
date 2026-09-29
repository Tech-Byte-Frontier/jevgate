//! Agent instruction files and project documentation, read whole whether or
//! not they are hidden or ignored.
use super::*;

/// Which documents a check reads: those `selected`, and with `broken` the
/// others in its scope that name one of the paths a change removed.
pub(super) type Selection<'a> = (
    &'a dyn Fn(&Path) -> bool,
    Option<(&'a dyn Fn(&Path) -> bool, &'a Arc<BTreeSet<PathBuf>>)>,
);

/// Agent instruction files and project docs the selected rules judge.
pub(super) fn add_documents(
    args: &CheckArgs,
    context: &ConfigContext,
    boundary: &Boundary,
    (selected, broken): Selection<'_>,
    inputs: &mut Vec<Input>,
) -> Result<()> {
    let repository = Arc::new(crate::docs::scan(&context.root)?);
    let every = cross_document(args) || sections(args);
    let instructions = repository
        .readers
        .keys()
        .filter(|_| args.enabled(crate::catalog::AGENT_CONTEXT) || every)
        .map(|p| (p, INSTRUCTIONS));
    let docs = repository
        .docs
        .iter()
        .filter(|_| args.enabled(crate::catalog::LARGE_DOCS) || every)
        .map(|p| (p, DOCS));
    for (relative, role) in instructions.chain(docs) {
        let judged = (role == DOCS || repository.judged(relative))
            && !inputs.iter().any(|i| i.result.path == *relative);
        if !judged {
            continue;
        }
        let change = match broken {
            _ if selected(relative) => None,
            Some((in_scope, removed)) if in_scope(relative) => {
                Some(crate::revision::FileChange::unchanged(removed.clone()))
            }
            _ => continue,
        };
        let mut input = bounded(relative, role, args, context, boundary)?;
        if let Some(change) = change {
            let text = input.source.as_deref().unwrap_or("");
            if !names_removed(relative, text, &change.removed) {
                continue;
            }
            input.changed = Some(change);
        }
        if input.result.status != Status::Skipped {
            input.repository = Some(repository.clone());
        }
        inputs.push(input);
    }
    Ok(())
}

/// Whether `text`, the document at `doc`, may name one of `removed`: the
/// path from the repository root, or from a directory holding the document,
/// as a relative link climbing out of it ends. Only the top of each removed
/// tree is looked for, since a name inside it holds the tree's own path: a
/// deleted directory of vendored files is one search. The staleness rule
/// then decides which sections name a removed path. Both paths are written
/// with `/`, as Git and `discovery::relative` write them.
fn names_removed(doc: &Path, text: &str, removed: &BTreeSet<PathBuf>) -> bool {
    let folders: Vec<&Path> = doc
        .ancestors()
        .skip(1)
        .filter(|dir| !dir.as_os_str().is_empty())
        .collect();
    removed
        .iter()
        .filter(|path| path.parent().is_none_or(|dir| !removed.contains(dir)))
        .any(|path| {
            let below = folders.iter().filter_map(|dir| path.strip_prefix(dir).ok());
            std::iter::once(path.as_path())
                .chain(below)
                .any(|name| text.contains(name.to_string_lossy().as_ref()))
        })
}

/// A documentation file, read whole; hidden and ignored paths are allowed.
pub(super) fn load_document(
    relative: &std::path::Path,
    role: &str,
    args: &CheckArgs,
    path: &std::path::Path,
) -> Result<Input> {
    let mut result = pending_result(relative, role, args, &[]);
    if args
        .size(path)
        .is_some_and(|size| size > args.max_file_bytes)
    {
        return over_read_cap(result, relative, path, args);
    }
    Ok(match args.read(path, args.max_file_bytes) {
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
                changed: None,
            }
        }
        // dvja's docs hold a Markdown file with NUL bytes, which made the run incomplete.
        Err(error) => unread(result, error),
    })
}

/// Whether a custom question asks about documentation sections.
pub(super) fn sections(args: &CheckArgs) -> bool {
    args.custom()
        .any(|q| q.unit == crate::custom::Kind::Section)
}

/// Whether a rule that compares or checks every kind of document is selected.
fn cross_document(args: &CheckArgs) -> bool {
    args.enabled(crate::catalog::DOC_STALENESS) || args.enabled(crate::catalog::DOC_DUPLICATION)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_document_may_name_a_removed_path_from_the_root_or_a_folder_holding_it() {
        let removed: BTreeSet<PathBuf> = [
            "docs/api/old.md",
            "vendor/lib",
            "vendor/lib/a.js",
            "vendor/lib/b.js",
        ]
        .map(PathBuf::from)
        .into();
        let names = |text: &str| names_removed(Path::new("docs/guide/intro.md"), text, &removed);
        assert!(names("See [the API](../api/old.md)."));
        assert!(names("Built from `vendor/lib/a.js`."));
        assert!(!names("See old.md and lib."));
    }
}
