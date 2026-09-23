//! Planning every rule's units and requests over the selected scope: parse the
//! files once, build the facts that span files, then plan each file.
/// A scope of files sharing clone, subject and caller evidence.
pub(super) struct Scope<'a> {
    owners: Vec<usize>,
    inputs: &'a [Input],
    views: &'a BTreeMap<usize, View>,
    units: BTreeMap<usize, FileUnits>,
    context: Vec<(PathBuf, &'a str, FileUnits)>,
    /// Agent instruction files, judged by the documentation rules only.
    documents: Vec<usize>,
}

use super::{
    FileContext, FilePlan, Plan, Planned, documents, drift, duplicates, functions, hardcoded,
    instructions, outline, security, test_units,
};
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
    let drift = drift::Shared::new(
        inputs,
        &scope.documents,
        args.enabled(catalog::DOC_STALENESS),
        args.enabled(catalog::DOC_DUPLICATION),
    );
    for &owner in &scope.documents {
        let file = plan_document(
            &inputs[owner],
            owner,
            args,
            budget,
            &drift,
            &mut result.requests,
        );
        result.files.insert(owner, file);
    }
    result
}

/// The documentation rules for one agent instruction file.
fn plan_document(
    input: &Input,
    owner: usize,
    args: &CheckArgs,
    budget: &TokenBudget,
    drift: &drift::Shared<'_>,
    requests: &mut Vec<Planned>,
) -> FilePlan {
    let mut file = FilePlan {
        path: input.result.path.clone(),
        ..Default::default()
    };
    let context = FileContext {
        owner,
        path: &input.result.path,
        language: "Markdown",
        source: input.source.as_deref().unwrap_or(""),
        source_hash: &input.result.source_hash,
        model: &args.model,
        budget,
    };
    if input.result.role == crate::inventory::DOCS {
        if args.enabled(catalog::LARGE_DOCS) {
            documents::plan(&context, &mut file, requests);
        }
    } else if let Some(repository) = &input.repository
        && args.enabled(catalog::AGENT_CONTEXT)
    {
        instructions::plan(&context, repository, &mut file, requests);
    }
    drift.plan(&context, &mut file, requests);
    file
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
        documents: Vec::new(),
    };
    for (&owner, view) in views {
        let input = &inputs[owner];
        if [crate::file_kind::INSTRUCTIONS, crate::file_kind::DOCS]
            .contains(&view.classification.kind.as_str())
        {
            scope.documents.push(owner);
            continue;
        }
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
        budget,
    };
    let mut file = FilePlan {
        path: input.result.path.clone(),
        ..Default::default()
    };
    let lines = scope.test_lines(owner);
    let cases = shared.cases.get(context.path).cloned().unwrap_or_default();
    if shared.enabled(catalog::FUNCTION_SIMPLIFICATION) {
        plan_functions(scope, &context, view, &lines, &cases, &mut file, requests);
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
            outline::plan(&context, parsed, &members, &callers, &mut file, requests);
        }
    }
    if shared.enabled(catalog::HARDCODED_VALUES) && view.application {
        file.rules.insert(catalog::HARDCODED_VALUES, 0);
        let parsed = &scope.units[&owner];
        let outside_tests = |line: usize| !lines.iter().any(|l| l.contains(&line));
        let units: Vec<&Unit> = parsed
            .units
            .iter()
            .filter(|u| u.callable() && outside_tests(u.line))
            .collect();
        let constants: Vec<_> = parsed
            .constants
            .iter()
            .filter(|c| outside_tests(c.line))
            .cloned()
            .collect();
        hardcoded::plan(&context, &units, &constants, &mut file, requests);
    }
    let rules: Vec<&'static str> = catalog::SECURITY
        .into_iter()
        .filter(|rule| shared.enabled(rule))
        .collect();
    if !rules.is_empty() && view.application {
        plan_security(scope, shared, &context, &lines, &rules, &mut file, requests);
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
            &mut file,
            requests,
        );
    }
    if view.tests && args.include_tests {
        plan_tests(shared, &context, cases, &mut file, requests);
    }
    file
}

/// Application functions outside tests and the file's setup statements; the
/// injection recheck shows up to three callers of each function.
fn plan_security(
    scope: &Scope<'_>,
    shared: &Shared<'_>,
    context: &FileContext<'_>,
    lines: &[Range<usize>],
    rules: &[&'static str],
    file: &mut FilePlan,
    requests: &mut Vec<Planned>,
) {
    for rule in rules {
        file.rules.insert(rule, 0);
    }
    let parsed = &scope.units[&context.owner];
    let outside_tests = |line: usize| !lines.iter().any(|l| l.contains(&line));
    let subjects: Vec<security::Subject<'_>> = parsed
        .units
        .iter()
        .filter(|u| u.callable() && outside_tests(u.line))
        .map(|unit| {
            let callers = if rules.contains(&catalog::INJECTION) {
                callers_of(scope, &shared.imports, context.owner, unit)
            } else {
                Vec::new()
            };
            security::function_subject(context, unit, callers)
        })
        .collect();
    let setup = security::setup_subject(context, &parsed.setup)
        .filter(|_| parsed.setup.statements.iter().all(|s| outside_tests(s.1)));
    security::plan(context, &subjects, setup, rules, file, requests);
}

/// Functions outside tests, in this file and in selected files that import
/// it, that call `unit`, as (name, source).
fn callers_of(
    scope: &Scope<'_>,
    imports: &BTreeMap<usize, Imports>,
    owner: usize,
    unit: &Unit,
) -> Vec<(String, String)> {
    let path = &scope.inputs[owner].result.path;
    let others = scope.owners.iter().filter(|&&o| o != owner);
    let mut found = Vec::new();
    for &other in std::iter::once(&owner).chain(others) {
        if other != owner && !imports[&other].reach(path) {
            continue;
        }
        let source = scope.inputs[other].source.as_deref().unwrap_or("");
        let lines = scope.test_lines(other);
        for caller in &scope.units[&other].units {
            if found.len() == security::CALLERS {
                return found;
            }
            if caller.callable()
                && caller.name != unit.name
                && caller.calls.contains(&unit.short_name)
                && !lines.iter().any(|l| caller.overlaps(l))
            {
                found.push((caller.name.clone(), caller.source(source).to_string()));
            }
        }
    }
    found
}

/// Callable units: application code with the application view, and test
/// support (not test cases) with the test view.
fn plan_functions(
    scope: &Scope<'_>,
    context: &FileContext<'_>,
    view: &View,
    lines: &[Range<usize>],
    cases: &[TestCase],
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
        functions::plan(context, &judged, scope, file, requests);
    }
}

/// Test value and redundancy, with each test linked to the functions it calls.
fn plan_tests(
    shared: &Shared<'_>,
    context: &FileContext<'_>,
    mut cases: Vec<TestCase>,
    file: &mut FilePlan,
    requests: &mut Vec<Planned>,
) {
    test_map::link(&mut cases, &shared.subjects.keys().cloned().collect());
    if shared.enabled(catalog::TEST_VALUE) {
        file.rules.insert(catalog::TEST_VALUE, 0);
        test_units::plan_values(context, &cases, &shared.subjects, file, requests);
    }
    if shared.enabled(catalog::TEST_REDUNDANCY) {
        file.rules.insert(catalog::TEST_REDUNDANCY, 0);
        test_units::plan_pairs(context, &cases, &shared.subjects, file, requests);
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
