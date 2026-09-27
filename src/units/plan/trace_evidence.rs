//! Definitions across the scope that a security trace shows beside a site:
//! enums the site names and C# constants, often declared in another file.
use super::Scope;
use crate::analysis::units::Unit;
use std::collections::BTreeMap;

/// An enum shown with a security trace is at most this long.
const ENUM_BYTES: usize = 1500;

/// Enum definitions in selected files and context by name; a name defined
/// twice is left out, since the site could mean either.
pub(super) fn enums(scope: &Scope<'_>) -> BTreeMap<String, String> {
    definitions(scope, |unit, source| {
        let text = unit.source(source);
        (declaration(unit, text).contains("enum ") && text.len() <= ENUM_BYTES)
            .then(|| text.to_string())
    })
}

/// A type shown with a path confirm is at most this long.
const TYPE_BYTES: usize = 800;

/// Definitions of the types in selected files and context (structs,
/// classes, records; not enums) by name, with the documentation and
/// attributes above them, such as a Rust derive list that says how a route
/// parameter of that type is parsed; a name defined twice is left out.
pub(super) fn types(scope: &Scope<'_>) -> BTreeMap<String, String> {
    definitions(scope, |unit, source| {
        let text = source.get(unit.span.clone())?;
        (!declaration(unit, text).contains("enum ") && text.len() <= TYPE_BYTES)
            .then(|| text.to_string())
    })
}

/// The line of a definition that names it.
fn declaration<'t>(unit: &Unit, text: &'t str) -> &'t str {
    text.lines()
        .find(|l| l.contains(&unit.short_name))
        .unwrap_or("")
}

/// The definitions `shown` keeps among the types, enums and other
/// non-callable units of selected files and context, by short name; a name
/// defined twice is left out.
fn definitions(
    scope: &Scope<'_>,
    shown: impl Fn(&Unit, &str) -> Option<String>,
) -> BTreeMap<String, String> {
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
            if let Some(text) = shown(unit, source) {
                found
                    .entry(unit.short_name.clone())
                    .and_modify(|d| *d = None)
                    .or_insert(Some(text));
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
