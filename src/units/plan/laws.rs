//! Facts about the laws of a scope's Bend 2 files: the defs that compute a
//! type, which make a law a claim, and the predicates laws check, whose
//! literals are the laws' samples.
use super::Scope;
use std::collections::{BTreeMap, BTreeSet};

/// The Bend 2 defs of the scope that compute a type, by name.
pub(super) fn propositions(scope: &Scope<'_>) -> BTreeSet<String> {
    let bend = scope
        .owners
        .iter()
        .filter(|o| crate::analysis::bend::file(&scope.inputs[**o].result.path))
        .map(|o| &scope.units[o])
        .chain(
            scope
                .context
                .iter()
                .filter(|(path, ..)| crate::analysis::bend::file(path))
                .map(|(_, _, units)| units),
        );
    bend.flat_map(|file| &file.units)
        .filter(|u| u.role == crate::analysis::units::Role::TypeLevel)
        .map(|u| u.name.clone())
        .collect()
}

/// The Bend 2 predicates that laws check, `law flood: {flood_capped(24n) ==
/// True{} : Bool}` with a def returning `Bool` that no other code calls, and
/// the defs only such predicates call: they build the law's samples, and
/// their literals are its inputs. On a Bend 2 IRC client, 9 of 22 wrong
/// hardcoded-value considers were such samples. A def a law calls that
/// returns data, such as the `get` of a JSON library, is the law's subject.
pub(super) fn law_predicates(scope: &Scope<'_>) -> BTreeSet<String> {
    let calls = LawCalls::of(scope);
    let predicates = calls
        .checked
        .iter()
        .copied()
        .filter(|name| calls.returns_bool.contains(name))
        .collect();
    let predicates = called_only_among(predicates, &calls.callers);
    with_defs_only_they_call(predicates, &calls.callers)
        .into_iter()
        .map(str::to_string)
        .collect()
}

/// What the laws and tests of a scope's Bend 2 files call, its defs that
/// return `Bool`, and who calls each def outside laws and tests.
struct LawCalls<'a> {
    checked: BTreeSet<&'a str>,
    returns_bool: BTreeSet<&'a str>,
    callers: BTreeMap<&'a str, BTreeSet<&'a str>>,
}

impl<'a> LawCalls<'a> {
    fn of(scope: &'a Scope<'_>) -> Self {
        use crate::analysis::units::Kind;
        let bend = |owner: &&usize| crate::analysis::bend::file(&scope.inputs[**owner].result.path);
        let mut calls = Self {
            checked: BTreeSet::new(),
            returns_bool: BTreeSet::new(),
            callers: BTreeMap::new(),
        };
        for owner in scope.owners.iter().filter(bend) {
            let lines = scope.test_lines(*owner);
            for unit in &scope.units[owner].units {
                let test = lines.iter().any(|l| unit.overlaps(l));
                if unit.kind == Kind::Law || test {
                    calls.checked.extend(unit.calls.iter().map(String::as_str));
                    continue;
                }
                if !unit.callable() {
                    continue;
                }
                if unit.signature.ends_with("-> Bool") {
                    calls.returns_bool.insert(unit.name.as_str());
                }
                for call in unit.calls.iter().filter(|c| **c != unit.short_name) {
                    calls
                        .callers
                        .entry(call)
                        .or_default()
                        .insert(unit.name.as_str());
                }
            }
        }
        calls
    }
}

/// The predicates that no def outside them calls, dropping one another
/// until none is left to drop.
fn called_only_among<'a>(
    mut predicates: BTreeSet<&'a str>,
    callers: &BTreeMap<&'a str, BTreeSet<&'a str>>,
) -> BTreeSet<&'a str> {
    loop {
        let kept: BTreeSet<&str> = predicates
            .iter()
            .copied()
            .filter(|name| {
                callers
                    .get(name)
                    .is_none_or(|by| by.iter().all(|c| predicates.contains(c)))
            })
            .collect();
        if kept == predicates {
            return predicates;
        }
        predicates = kept;
    }
}

/// The predicates and the defs only they call, added until none is left.
fn with_defs_only_they_call<'a>(
    mut predicates: BTreeSet<&'a str>,
    callers: &BTreeMap<&'a str, BTreeSet<&'a str>>,
) -> BTreeSet<&'a str> {
    loop {
        let only_theirs: Vec<&str> = callers
            .iter()
            .filter(|(name, by)| {
                !predicates.contains(**name) && by.iter().all(|c| predicates.contains(c))
            })
            .map(|(name, _)| *name)
            .collect();
        if only_theirs.is_empty() {
            return predicates;
        }
        predicates.extend(only_theirs);
    }
}
