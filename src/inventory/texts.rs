//! Text files a custom `file` or `hunk` question's `paths` name that no
//! built-in rule reads: a shell script, a Terraform module, a Kotlin file.
use super::*;

/// Add each text file a custom question's `paths` name that is not read
/// already, or that its role set aside unread (a script, a fixture, a
/// migration, type declarations): with `--base`, among the changed files;
/// otherwise, over the repository. Generated files, hidden and dependency
/// paths, credential names, files outside the upload patterns and files
/// Git does not track are never read.
pub(super) fn add_texts(
    args: &CheckArgs,
    context: &ConfigContext,
    (boundary, selected): (&Boundary, &dyn Fn(&Path) -> bool),
    changes: Option<&crate::revision::Changes>,
    inputs: &mut Vec<Input>,
) -> Result<()> {
    // A hunk question asks nothing without `--base`, so it reads no file.
    let naming: Vec<_> = args
        .custom()
        .filter(|q| q.names_files() && (q.unit == crate::custom::Kind::File || args.base.is_some()))
        .collect();
    if naming.is_empty() {
        return Ok(());
    }
    let classifier = discovery::Classifier::new(&context.config)?;
    for relative in candidates(context, changes)? {
        let path = context.root.join(&relative);
        let wanted = naming.iter().any(|q| q.applies_to(&relative))
            && selected(&relative)
            && boundary.permits(&relative)
            && crate::context::ensure_visible_path(&relative).is_ok()
            && classifier.role(&relative) != "generated"
            && args.regular_file(&path);
        let existing = inputs.iter().position(|i| i.result.path == relative);
        if !wanted || existing.is_some_and(|at| !set_aside(&inputs[at])) {
            continue;
        }
        let input = load_document(&relative, TEXT, args, &path)?;
        if input
            .source
            .as_deref()
            .is_some_and(discovery::generated_source)
        {
            continue;
        }
        match existing {
            Some(at) => inputs[at] = input,
            None => inputs.push(input),
        }
    }
    Ok(())
}

/// Whether a file was set aside unread by its role, and so may be read for
/// a question that names it; generated and vendored code never is.
fn set_aside(input: &Input) -> bool {
    input.source.is_none()
        && input.result.status == Status::Skipped
        && matches!(
            input.result.role.as_str(),
            "script" | "fixture" | "migration" | "declarations"
        )
}

/// The files that may be named: the changed ones with `--base`, else every
/// file the walk finds, in path order; in a Git repository, only those it
/// tracks. Such a file is uploaded only because a question's glob matched
/// it, and an untracked one in a CI workspace can be a credential another
/// step wrote there, such as `google-github-actions/auth`'s
/// `gha-creds-*.json`, which a `*.json` glob would send.
fn candidates(
    context: &ConfigContext,
    changes: Option<&crate::revision::Changes>,
) -> Result<Vec<PathBuf>> {
    let untracked: BTreeSet<PathBuf> = crate::revision::untracked(&context.root)
        .unwrap_or_default()
        .into_iter()
        .collect();
    let mut found = Vec::new();
    match changes {
        Some(changes) => found.extend(changes.paths.keys().cloned()),
        None => {
            for entry in walker(&context.root) {
                let entry = entry.context("Failed while discovering Jev scope")?;
                if entry.file_type().is_some_and(|t| t.is_file()) {
                    found.push(discovery::relative(entry.path(), &context.root)?);
                }
            }
            found.sort();
        }
    }
    found.retain(|path| !untracked.contains(path));
    Ok(found)
}
