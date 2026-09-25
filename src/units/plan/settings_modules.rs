//! The Django settings modules around one settings module: those that
//! extend it with the settings they assign again, and the lines that select
//! a module to run with.
use super::Scope;

/// Settings modules shown extending one settings module, at most.
const EXTENDING_MODULES: usize = 6;

/// The settings modules that extend a settings module, directly or through
/// others (`production.py` doing `from .base import *`, where `base.py` does
/// `from config.settings.cors import *`), each with the security settings of
/// this module it assigns again and the lines that select it to run with: a
/// `DEBUG = True` in base settings is no concern when every deployed module
/// turns it off, and a module no deployed one extends is not deployed.
pub(super) fn settings_extended_by(scope: &Scope<'_>, owner: usize) -> Vec<serde_json::Value> {
    let setup = &scope.units[&owner].setup;
    if !setup.settings {
        return Vec::new();
    }
    // The security settings this module sets, which an extending module may
    // assign again.
    let names: Vec<&str> = setup
        .assigned
        .iter()
        .map(|(name, _)| name.as_str())
        .filter(|name| {
            crate::analysis::django::security_setting(name)
                || crate::analysis::django::secret_name(name)
        })
        .collect();
    extending_modules(scope, owner)
        .into_iter()
        .take(EXTENDING_MODULES)
        .map(|other| extending_entry(scope, other, &names))
        .collect()
}

/// The other settings modules that import `owner`, directly or through
/// others, in path order.
fn extending_modules(scope: &Scope<'_>, owner: usize) -> Vec<usize> {
    let candidates: Vec<usize> = scope
        .owners
        .iter()
        .copied()
        .filter(|&other| other != owner && scope.units[&other].setup.settings)
        .collect();
    let mut reached = vec![owner];
    let mut extending = Vec::new();
    loop {
        let found: Vec<usize> = candidates
            .iter()
            .copied()
            .filter(|other| !extending.contains(other))
            .filter(|&other| {
                let source = scope.inputs[other].source.as_deref().unwrap_or("");
                reached.iter().any(|&module| {
                    crate::analysis::django::extends(source, &scope.inputs[module].result.path)
                })
            })
            .collect();
        if found.is_empty() {
            break;
        }
        reached.extend(&found);
        extending.extend(found);
    }
    extending.sort_by_key(|&other| &scope.inputs[other].result.path);
    extending
}

/// One extending module, its own assignments of `names` and the lines that
/// select it to run with.
fn extending_entry(scope: &Scope<'_>, other: usize, names: &[&str]) -> serde_json::Value {
    let mut again: Vec<&str> = scope.units[&other]
        .setup
        .assigned
        .iter()
        .filter(|(name, _)| names.contains(&name.as_str()))
        .map(|(_, shown)| shown.as_str())
        .collect();
    again.dedup();
    let mut entry = serde_json::json!({
        "module": scope.inputs[other].result.path.display().to_string(),
        "sets_these_settings_again": again,
    });
    let selected = selections(&scope.inputs[other].settings_selected_by);
    if !selected.is_empty() {
        entry["selected_as_the_settings_to_run_with_by"] = serde_json::json!(selected);
    }
    entry
}

/// Lines that select a settings module to run with, as `path:line: text`.
/// A `setdefault` in `manage.py` or `wsgi.py` names only the module used
/// when the environment names none, which deployments set: shown bare, it
/// read as the deployed choice, and CORS settings that a production module
/// sets again were a review.
pub(super) fn selections(selected: &[crate::analysis::django::Selection]) -> Vec<String> {
    selected
        .iter()
        .map(|s| {
            let default = if s.text.contains("setdefault(") {
                " (only a default: a DJANGO_SETTINGS_MODULE set in the environment replaces it)"
            } else {
                ""
            };
            format!("{}:{}: {}{default}", s.file.display(), s.line, s.text)
        })
        .collect()
}
