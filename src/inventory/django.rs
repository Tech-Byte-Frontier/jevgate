//! Django evidence gathered while collecting files: the lines that select a
//! settings module, and the templates that write values without escaping.
use super::*;

/// Give each Python module that may be Django settings the lines of the
/// repository that select it (`DJANGO_SETTINGS_MODULE=…`), read only from
/// files the upload patterns permit. The repository is searched only when
/// such a module was collected.
pub(super) fn select_settings(context: &ConfigContext, boundary: &Boundary, inputs: &mut [Input]) {
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
pub(super) fn unescaped_templates(
    context: &ConfigContext,
    boundary: &Boundary,
    inputs: &mut [Input],
) {
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
