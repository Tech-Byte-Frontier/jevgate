//! Lines outside a settings module that select it to run with.
use std::path::Path;

/// A line outside a settings module that names it as the settings to run
/// with, such as `ENV DJANGO_SETTINGS_MODULE=site.settings.production` in a
/// Dockerfile or `os.environ.setdefault(…, "site.settings.dev")` in
/// `manage.py`: whether a module is deployed or only local shows there.
#[derive(Clone, Debug, PartialEq)]
pub struct Selection {
    pub file: std::path::PathBuf,
    pub line: usize,
    pub text: String,
}

/// Longest line text kept for a selection.
const SELECTION_TEXT: usize = 160;
/// Files larger than this are not searched for selections.
pub const SELECTION_FILE_BYTES: u64 = 65_536;
/// Selections shown per settings module, at most.
pub const SELECTIONS: usize = 8;

/// Whether a file may name the settings module a process runs with:
/// container, process, CI, test runner and shell configuration, and the
/// Python entry points Django projects keep beside their settings. Files
/// named `.env…` are left out, since their other lines hold secrets.
pub fn selection_file(relative: &Path) -> bool {
    let name = relative
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    if name.starts_with(".env") {
        return false;
    }
    let extension = relative
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default();
    name.starts_with("Dockerfile")
        || matches!(
            name,
            "Procfile" | "Makefile" | "manage.py" | "wsgi.py" | "asgi.py" | "conftest.py"
        )
        || matches!(
            extension,
            "yml" | "yaml" | "toml" | "cfg" | "ini" | "sh" | "json"
        )
}

/// The lines of one file that set the settings module to run with.
pub fn selections_in(relative: &Path, text: &str) -> Vec<Selection> {
    text.lines()
        .enumerate()
        .filter(|(_, line)| line.contains("DJANGO_SETTINGS_MODULE") || line.contains("--settings"))
        .map(|(index, line)| Selection {
            file: relative.to_path_buf(),
            line: index + 1,
            text: crate::analysis::sites::clip(line.trim())
                .chars()
                .take(SELECTION_TEXT)
                .collect(),
        })
        .collect()
}

/// The selections that name the settings module at `relative` by its dotted
/// module path (`site.settings.production`, or from its last two parts).
pub fn selected_by<'s>(relative: &Path, selections: &'s [Selection]) -> Vec<&'s Selection> {
    let module = relative.with_extension("");
    let parts: Vec<&str> = module.iter().filter_map(|p| p.to_str()).collect();
    let shortest = parts.len().min(2);
    let names: Vec<String> = (0..=parts.len() - shortest)
        .map(|start| parts[start..].join("."))
        .collect();
    selections
        .iter()
        .filter(|s| names.iter().any(|name| names_module(&s.text, name)))
        .take(SELECTIONS)
        .collect()
}

/// `name` appears as a whole dotted module path, not as part of a longer one.
fn names_module(line: &str, name: &str) -> bool {
    let part = |c: char| c.is_alphanumeric() || c == '_' || c == '.';
    line.match_indices(name).any(|(start, _)| {
        line[..start].chars().next_back().is_none_or(|c| !part(c))
            && line[start + name.len()..]
                .chars()
                .next()
                .is_none_or(|c| !part(c))
    })
}
