//! Planning every rule's units and requests over the selected scope: parse the
//! files once, build the facts that span files, then plan each file.
/// A scope of files sharing clone, subject and caller evidence.
pub(super) struct Scope<'a> {
    owners: Vec<usize>,
    inputs: &'a [Input],
    views: &'a BTreeMap<usize, View>,
    units: BTreeMap<usize, FileUnits>,
    context: Vec<(PathBuf, &'a str, FileUnits)>,
}

use super::{FileContext, FilePlan, Plan, Planned, duplicates, functions, outline, test_units};
use crate::{
    analysis::{
        clones::{self, SourceFile},
        imports::Imports,
        test_map::{self, TestCase},
        units::{self as parsed, FileUnits, Unit},
    },
    catalog,
    file_kind::View,
    inventory::Input,
    options::CheckArgs,
    token_budget::TokenBudget,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range,
    path::{Path, PathBuf},
};

impl Scope<'_> {
    pub(super) fn test_lines(&self, owner: usize) -> Vec<Range<usize>> {
        self.views[&owner]
            .test_lines
            .iter()
            .map(|r| r.start_line..r.end_line + 1)
            .collect()
    }

    /// Callable units outside tests, in selected files and explicit context.
    pub(super) fn scope_units(&self) -> impl Iterator<Item = (&Path, &Unit)> {
        let selected = self.owners.iter().flat_map(move |owner| {
            let lines = self.test_lines(*owner);
            let path = self.inputs[*owner].result.path.as_path();
            self.units[owner]
                .units
                .iter()
                .filter(move |u| u.callable() && !lines.iter().any(|l| u.overlaps(l)))
                .map(move |u| (path, u))
        });
        let context = self.context.iter().flat_map(|(path, _, units)| {
            units
                .units
                .iter()
                .filter(|u| u.callable())
                .map(move |u| (path.as_path(), u))
        });
        selected.chain(context)
    }
}

pub fn plan(
    inputs: &[Input],
    views: &BTreeMap<usize, View>,
    args: &CheckArgs,
    budget: &TokenBudget,
) -> Plan {
    let mut result = Plan::default();
    let scope = parsed_scope(inputs, views, &mut result.skipped);
    let shared = Shared::new(&scope, args);
    for &owner in &scope.owners {
        let file = plan_file(&scope, &shared, owner, args, budget, &mut result.requests);
        result.files.insert(owner, file);
    }
    result
}

/// Parse every selected file and the explicit context. Files without a parser
/// or with syntax errors are skipped with a reason.
fn parsed_scope<'a>(
    inputs: &'a [Input],
    views: &'a BTreeMap<usize, View>,
    skipped: &mut BTreeMap<usize, String>,
) -> Scope<'a> {
    let mut scope = Scope {
        owners: Vec::new(),
        inputs,
        views,
        units: BTreeMap::new(),
        context: Vec::new(),
    };
    for (&owner, view) in views {
        let input = &inputs[owner];
        let source = input.source.as_deref().unwrap_or("");
        match parsed::parse(&input.result.path, source) {
            Ok(units) if units.parsed => {
                scope.units.insert(owner, units);
                scope.owners.push(owner);
            }
            Ok(_) => {
                let reason = format!(
                    "No {} parser; units cannot be located, so this file was not judged.",
                    view.classification.language
                );
                skipped.insert(owner, reason);
            }
            Err(_) => {
                skipped.insert(owner, "Syntax errors; this file was not judged.".into());
            }
        }
    }
    if let Some(first) = scope.owners.first() {
        for context in &inputs[*first].context {
            let units = parsed::parse(&context.file.path, &context.source).unwrap_or_default();
            scope
                .context
                .push((context.file.path.clone(), context.source.as_str(), units));
        }
    }
    scope
}

/// Facts that span files: clone groups, imports, callable subjects and hashes.
struct Shared<'a> {
    rules: &'a [String],
    pairs: clones::Candidates,
    imports: BTreeMap<usize, Imports>,
    /// Callable short names to their signatures, for test subjects.
    subjects: BTreeMap<String, String>,
    /// Test cases of each selected file with a test view, inside its test lines.
    cases: BTreeMap<PathBuf, Vec<TestCase>>,
    hashes: BTreeMap<PathBuf, String>,
}

impl<'a> Shared<'a> {
    fn new(scope: &Scope<'_>, args: &'a CheckArgs) -> Self {
        let mut shared = Self {
            rules: &args.rules,
            pairs: clones::Candidates::default(),
            imports: imports(scope),
            subjects: BTreeMap::new(),
            cases: test_cases(scope),
            hashes: BTreeMap::new(),
        };
        if shared.enabled(catalog::SHARED_LOGIC) {
            shared.pairs = duplicate_candidates(scope);
        }
        for (_, unit) in scope.scope_units() {
            shared
                .subjects
                .entry(unit.short_name.clone())
                .or_insert_with(|| unit.signature.clone());
        }
        for &owner in &scope.owners {
            let result = &scope.inputs[owner].result;
            shared
                .hashes
                .insert(result.path.clone(), result.source_hash.clone());
        }
        if let Some(first) = scope.owners.first() {
            for context in &scope.inputs[*first].context {
                shared
                    .hashes
                    .insert(context.file.path.clone(), context.file.source_hash.clone());
            }
        }
        shared
    }

    fn enabled(&self, key: &str) -> bool {
        self.rules.iter().any(|r| r == key || r == catalog::id(key))
    }
}

/// Every rule's units and requests for one selected file.
fn plan_file(
    scope: &Scope<'_>,
    shared: &Shared<'_>,
    owner: usize,
    args: &CheckArgs,
    budget: &TokenBudget,
    requests: &mut Vec<Planned>,
) -> FilePlan {
    let input = &scope.inputs[owner];
    let view = &scope.views[&owner];
    let context = FileContext {
        owner,
        path: &input.result.path,
        language: crate::file_kind::language(&input.result.path),
        source: input.source.as_deref().unwrap_or(""),
        source_hash: &input.result.source_hash,
        model: &args.model,
    };
    let mut file = FilePlan {
        path: input.result.path.clone(),
        ..Default::default()
    };
    let lines = scope.test_lines(owner);
    let cases = shared.cases.get(context.path).cloned().unwrap_or_default();
    if shared.enabled(catalog::FUNCTION_SIMPLIFICATION) {
        plan_functions(
            scope, &context, view, &lines, &cases, budget, &mut file, requests,
        );
    }
    if shared.enabled(catalog::FILE_ORGANIZATION) && view.application {
        let units = &scope.units[&owner].units;
        let members: Vec<usize> = (0..units.len())
            .filter(|&i| !lines.iter().any(|l| units[i].overlaps(l)))
            .collect();
        file.rules.insert(catalog::FILE_ORGANIZATION, 0);
        if members.len() >= 2 {
            let callers = callers(scope, &shared.imports, owner);
            let parsed = &scope.units[&owner];
            outline::plan(
                &context, parsed, &members, &callers, budget, &mut file, requests,
            );
        }
    }
    if shared.enabled(catalog::SHARED_LOGIC) {
        file.rules.insert(catalog::SHARED_LOGIC, 0);
        let pairs = &shared.pairs;
        duplicates::plan(
            &context,
            pairs,
            &shared.cases,
            &lines,
            &shared.hashes,
            budget,
            &mut file,
            requests,
        );
    }
    if view.tests && args.include_tests {
        plan_tests(shared, &context, cases, budget, &mut file, requests);
    }
    file
}

/// Callable units: application code with the application view, and test
/// support (not test cases) with the test view.
#[allow(clippy::too_many_arguments)]
fn plan_functions(
    scope: &Scope<'_>,
    context: &FileContext<'_>,
    view: &View,
    lines: &[Range<usize>],
    cases: &[TestCase],
    budget: &TokenBudget,
    file: &mut FilePlan,
    requests: &mut Vec<Planned>,
) {
    let judged: Vec<&Unit> = scope.units[&context.owner]
        .units
        .iter()
        .filter(|u| u.callable())
        .filter(|u| {
            if lines.iter().any(|l| u.overlaps(l)) {
                view.tests
                    && !cases
                        .iter()
                        .any(|c| c.line <= u.line && u.end_line <= c.end_line)
            } else {
                view.application
            }
        })
        .collect();
    if view.application || !judged.is_empty() {
        file.rules.insert(catalog::FUNCTION_SIMPLIFICATION, 0);
        functions::plan(context, &judged, scope, budget, file, requests);
    }
}

/// Test value and redundancy, with each test linked to the functions it calls.
fn plan_tests(
    shared: &Shared<'_>,
    context: &FileContext<'_>,
    mut cases: Vec<TestCase>,
    budget: &TokenBudget,
    file: &mut FilePlan,
    requests: &mut Vec<Planned>,
) {
    test_map::link(&mut cases, &shared.subjects.keys().cloned().collect());
    if shared.enabled(catalog::TEST_VALUE) {
        file.rules.insert(catalog::TEST_VALUE, 0);
        test_units::plan_values(context, &cases, &shared.subjects, budget, file, requests);
    }
    if shared.enabled(catalog::TEST_REDUNDANCY) {
        file.rules.insert(catalog::TEST_REDUNDANCY, 0);
        test_units::plan_pairs(context, &cases, &shared.subjects, budget, file, requests);
    }
}

/// Selected files, with test lines excluded unless tests are judged, then the
/// explicit context.
fn duplicate_candidates(scope: &Scope<'_>) -> clones::Candidates {
    let selected = scope.owners.iter().map(|&owner| SourceFile {
        path: &scope.inputs[owner].result.path,
        source: scope.inputs[owner].source.as_deref().unwrap_or(""),
        selected: true,
        units: &scope.units[&owner].units,
        excluded: if scope.views[&owner].tests {
            Vec::new()
        } else {
            scope.test_lines(owner)
        },
    });
    let context = scope
        .context
        .iter()
        .map(|(path, source, units)| SourceFile {
            path,
            source,
            selected: false,
            units: &units.units,
            excluded: Vec::new(),
        });
    clones::find(&selected.chain(context).collect::<Vec<_>>())
}

/// Test cases of every selected file judged with its test view.
fn test_cases(scope: &Scope<'_>) -> BTreeMap<PathBuf, Vec<TestCase>> {
    scope
        .owners
        .iter()
        .filter(|owner| scope.views[owner].tests)
        .map(|&owner| {
            let input = &scope.inputs[owner];
            let lines = scope.test_lines(owner);
            let cases = test_map::cases(&input.result.path, input.source.as_deref().unwrap_or(""))
                .unwrap_or_default()
                .into_iter()
                .filter(|c| lines.iter().any(|l| l.contains(&c.line)))
                .collect();
            (input.result.path.clone(), cases)
        })
        .collect()
}

/// Import lines of every selected file, for caller lookups.
fn imports(scope: &Scope<'_>) -> BTreeMap<usize, Imports> {
    scope
        .owners
        .iter()
        .map(|&owner| {
            let input = &scope.inputs[owner];
            let source = input.source.as_deref().unwrap_or("");
            (owner, Imports::new(&input.result.path, source))
        })
        .collect()
}

/// Short callee name to the other selected files that import `target` and call it.
fn callers(
    scope: &Scope<'_>,
    imports: &BTreeMap<usize, Imports>,
    target: usize,
) -> BTreeMap<String, BTreeSet<PathBuf>> {
    let path = &scope.inputs[target].result.path;
    let mut callers = BTreeMap::<String, BTreeSet<PathBuf>>::new();
    for &owner in &scope.owners {
        if owner == target || !imports[&owner].reach(path) {
            continue;
        }
        for unit in &scope.units[&owner].units {
            for call in &unit.calls {
                callers
                    .entry(call.clone())
                    .or_default()
                    .insert(scope.inputs[owner].result.path.clone());
            }
        }
    }
    callers
}
