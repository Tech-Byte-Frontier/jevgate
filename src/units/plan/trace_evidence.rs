//! Definitions across the scope that a security trace shows beside a site:
//! enums the site names and C# constants, often declared in another file.
use super::Scope;
use std::collections::BTreeMap;

/// An enum shown with a security trace is at most this long.
const ENUM_BYTES: usize = 1500;

/// Enum definitions in selected files and context by name; a name defined
/// twice is left out, since the site could mean either.
pub(super) fn enums(scope: &Scope<'_>) -> BTreeMap<String, String> {
    let selected = scope.owners.iter().map(|&owner| {
        (
            scope.inputs[owner].source.as_deref().unwrap_or(""),
            &scope.units[&owner].units,
        )
    });
    let context = scope
        .context
        .iter()
        .map(|(_, source, units)| (*source, &units.units));
    let mut found: BTreeMap<String, Option<String>> = BTreeMap::new();
    for (source, units) in selected.chain(context) {
        for unit in units.iter().filter(|u| !u.callable()) {
            let text = unit.source(source);
            let declaration = text
                .lines()
                .find(|l| l.contains(&unit.short_name))
                .unwrap_or("");
            if declaration.contains("enum ") && text.len() <= ENUM_BYTES {
                found
                    .entry(unit.short_name.clone())
                    .and_modify(|d| *d = None)
                    .or_insert_with(|| Some(text.to_string()));
            }
        }
    }
    found
        .into_iter()
        .filter_map(|(name, text)| Some((name, text?)))
        .collect()
}

/// A constant shown with a security trace is at most this long.
const CONSTANT_BYTES: usize = 200;

/// The `const` and `static readonly` fields of C# files among the selected
/// files and context, by field name. C# declares them in a class, often in
/// another file than the code that names them (`AuthorizationConstants`), so
/// a trace could not tell a key written in the code from one read from
/// configuration.
pub(super) fn csharp_constants(scope: &Scope<'_>) -> BTreeMap<String, Vec<String>> {
    let selected = scope.owners.iter().map(|&owner| {
        (
            scope.inputs[owner].result.path.as_path(),
            &scope.units[&owner].constants,
        )
    });
    let context = scope
        .context
        .iter()
        .map(|(path, _, units)| (path.as_path(), &units.constants));
    let mut found: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (path, constants) in selected.chain(context) {
        if path.extension().is_none_or(|e| e != "cs") {
            continue;
        }
        for constant in constants {
            let Some(value) = &constant.value else {
                continue;
            };
            let field = constant.name.rsplit('.').next().unwrap_or(&constant.name);
            let declaration = format!("{} = {value}", constant.name);
            if declaration.len() <= CONSTANT_BYTES {
                found
                    .entry(field.to_string())
                    .or_default()
                    .push(declaration);
            }
        }
    }
    found
}
