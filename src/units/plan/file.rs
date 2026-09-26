//! Planning one selected code file: every rule's units and requests, with
//! the facts of the file and the scope each rule needs.
use super::{Scope, Shared, plan_security};
use crate::{
    analysis::{
        imports::Links,
        test_map::{self, TestCase},
        units::{FileUnits, Unit},
    },
    catalog,
    file_kind::View,
    inventory::Input,
    options::CheckArgs,
    token_budget::TokenBudget,
    units::{
        FileContext, FilePlan, Planned, comments, duplicates, functions, hardcoded, outline,
        spacetimedb, test_units,
    },
};
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range,
    path::{Path, PathBuf},
};

/// Every rule's units and requests for one selected file.
pub(super) fn plan_file(
    scope: &Scope<'_>,
    shared: &Shared<'_>,
    owner: usize,
    args: &CheckArgs,
    budget: &TokenBudget,
    requests: &mut Vec<Planned>,
) -> FilePlan {
    let input = &scope.inputs[owner];
    let view = &scope.views[&owner];
    let context = file_context(input, owner, args, budget);
    let mut file = FilePlan {
        path: input.result.path.clone(),
        ..Default::default()
    };
    let lines = scope.test_lines(owner);
    let cases = shared.cases.get(context.path).cloned().unwrap_or_default();
    if shared.enabled(catalog::FUNCTION_SIMPLIFICATION) {
        plan_functions(scope, &context, view, &lines, &cases, &mut file, requests);
    }
    if shared.enabled(catalog::FILE_ORGANIZATION) {
        plan_outline(scope, shared, &context, view, &lines, &mut file, requests);
    }
    // Example code spells its values out for the reader: sqlmodel's
    // `docs_src` tutorials each open a `database.db` with sample heroes.
    if shared.enabled(catalog::HARDCODED_VALUES)
        && view.application
        && !crate::analysis::clones::example_code(context.path)
    {
        plan_values(&scope.units[&owner], &context, &lines, &mut file, requests);
    }
    // Laravel's configuration files come from the framework and its
    // packages, with their documentation as comments: on two Laravel apps,
    // all six comment considers there were the publisher's text, and 14
    // files' comments stayed undecided.
    let published = shared.laravel && laravel_config(context.path);
    if shared.enabled(catalog::COMMENTS) && view.application && !published {
        let comments = (&lines[..], shared.teaching);
        plan_comments(
            &scope.units[&owner],
            &context,
            comments,
            &mut file,
            requests,
        );
    }
    let rules: Vec<&'static str> = catalog::SECURITY
        .into_iter()
        .filter(|rule| shared.enabled(rule))
        .collect();
    if !rules.is_empty() && view.application {
        plan_security(scope, shared, &context, &lines, &rules, &mut file, requests);
    }
    if shared.enabled(catalog::SHARED_LOGIC) && (view.application || view.tests) {
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
        let table = parameterizable(input);
        plan_tests(
            shared,
            &context,
            cases,
            (&lines, table),
            &mut file,
            requests,
        );
    }
    if shared.enabled(catalog::ACCESS_CONTROL)
        && view.application
        && let Some(framework) = &input.framework
    {
        plan_module(shared, &context, framework, &mut file, requests);
    }
    file
}

/// One selected code file's facts, with the role a web framework gives it.
fn file_context<'a>(
    input: &'a Input,
    owner: usize,
    args: &'a CheckArgs,
    budget: &'a TokenBudget,
) -> FileContext<'a> {
    FileContext {
        owner,
        path: &input.result.path,
        language: crate::file_kind::language(&input.result.path),
        source: input.source.as_deref().unwrap_or(""),
        source_hash: &input.result.source_hash,
        model: args.model(),
        budget,
        framework: crate::components::server_template(&input.result.path)
            .then(|| crate::components::TEMPLATE_SCRIPT.to_string())
            .or_else(|| {
                crate::units::nextjs::describe(
                    &input.result.path,
                    input.source.as_deref().unwrap_or(""),
                    input.package.as_ref(),
                )
            })
            .or_else(|| {
                crate::units::sveltekit::describe(&input.result.path, input.package.as_ref())
            })
            .or_else(|| {
                crate::units::graphql::describe(
                    &input.result.path,
                    input.source.as_deref().unwrap_or(""),
                )
                .or_else(|| crate::units::client_app::describe(input.package.as_ref()))
                .map(str::to_string)
            }),
    }
}

/// An application file's members outside tests, or a test file's cases.
fn plan_outline(
    scope: &Scope<'_>,
    shared: &Shared<'_>,
    context: &FileContext<'_>,
    view: &View,
    lines: &[Range<usize>],
    file: &mut FilePlan,
    requests: &mut Vec<Planned>,
) {
    let owner = context.owner;
    if view.application {
        let units = &scope.units[&owner].units;
        let members: Vec<usize> = (0..units.len())
            .filter(|&i| !lines.iter().any(|l| units[i].overlaps(l)))
            .collect();
        file.rules.insert(catalog::FILE_ORGANIZATION, 0);
        if members.len() >= 2 {
            let callers = callers(scope, &shared.links, owner);
            let parsed = &scope.units[&owner];
            outline::plan(context, parsed, &members, &callers, file, requests);
        }
    } else if view.classification.kind == crate::file_kind::TESTS {
        file.rules.insert(catalog::FILE_ORGANIZATION, 0);
        let mut cases = test_map::cases(context.path, context.source).unwrap_or_default();
        shared.link_routes(&mut cases);
        test_map::link(&mut cases, &shared.subjects.keys().cloned().collect());
        if java(context.path) {
            shared.qualify_subjects(&mut cases);
        }
        outline::plan_tests(context, &scope.units[&owner], &cases, file, requests);
    }
}

/// Callables and module constants outside tests.
fn plan_values(
    parsed: &FileUnits,
    context: &FileContext<'_>,
    lines: &[Range<usize>],
    file: &mut FilePlan,
    requests: &mut Vec<Planned>,
) {
    file.rules.insert(catalog::HARDCODED_VALUES, 0);
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
    hardcoded::plan(context, &units, &constants, file, requests);
}

/// Comments outside tests.
/// `teaching` when the project writes its comments for learners.
fn plan_comments(
    parsed: &FileUnits,
    context: &FileContext<'_>,
    (lines, teaching): (&[Range<usize>], bool),
    file: &mut FilePlan,
    requests: &mut Vec<Planned>,
) {
    file.rules.insert(catalog::COMMENTS, 0);
    let found = crate::analysis::comments::comments(context.path, context.source, &parsed.units)
        .unwrap_or_default();
    let found: Vec<_> = found
        .into_iter()
        .filter(|c| !lines.iter().any(|l| l.contains(&c.line)))
        .collect();
    comments::plan(context, (&parsed.units, teaching), &found, file, requests);
}

/// A PHP file directly in the project's `config` directory.
fn laravel_config(path: &std::path::Path) -> bool {
    let mut parts = path.iter();
    parts.next().is_some_and(|dir| dir == "config")
        && parts
            .next()
            .is_some_and(|file| file.to_string_lossy().ends_with(".php"))
        && parts.next().is_none()
}

/// A SpacetimeDB module file, whose definitions are sent with the helpers
/// they call: of each name, the one in the module's package nearest the file.
fn plan_module(
    shared: &Shared<'_>,
    context: &FileContext<'_>,
    framework: &crate::inventory::Framework,
    file: &mut FilePlan,
    requests: &mut Vec<Planned>,
) {
    let lookup = |name: &str| {
        shared.module_helpers.get(name).and_then(|candidates| {
            candidates
                .iter()
                .filter(|h| h.path.starts_with(&framework.root))
                .max_by_key(|h| {
                    let shared_parts = h
                        .path
                        .iter()
                        .zip(context.path.iter())
                        .take_while(|(a, b)| a == b)
                        .count();
                    (shared_parts, std::cmp::Reverse(h.path.clone()))
                })
        })
    };
    spacetimedb::plan(context, &framework.version, lookup, file, requests);
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

pub(super) fn java(path: &Path) -> bool {
    path.extension().is_some_and(|e| e == "java")
}

/// Whether a file's tests can hold their cases in one parameterized test.
/// Rust has no such test built in: without a crate such as rstest or
/// test-case, suggesting one asks the project for a dependency, and just's
/// one-case integration tests read as 160 considers to merge.
fn parameterizable(input: &Input) -> bool {
    const CRATES: [&str; 4] = ["rstest", "test-case", "test_case", "yare"];
    input.result.path.extension().is_none_or(|e| e != "rs")
        || input
            .package
            .as_ref()
            .is_some_and(|p| CRATES.iter().any(|c| p.dependencies.contains(*c)))
}

/// Test value and redundancy, with each test linked to the functions it
/// calls; `table` when its tests can be parameterized.
fn plan_tests(
    shared: &Shared<'_>,
    context: &FileContext<'_>,
    mut cases: Vec<TestCase>,
    (test_lines, table): (&[Range<usize>], bool),
    file: &mut FilePlan,
    requests: &mut Vec<Planned>,
) {
    shared.link_routes(&mut cases);
    test_map::link(&mut cases, &shared.subjects.keys().cloned().collect());
    let subjects = test_units::Subjects {
        signatures: &shared.subjects,
        sources: &shared.subject_sources,
        helpers: &shared.helpers,
        hashes: &shared.hashes,
        routes: &shared.route_labels,
    };
    if shared.enabled(catalog::TEST_VALUE) {
        file.rules.insert(catalog::TEST_VALUE, 0);
        test_units::plan_values(context, &cases, &subjects, test_lines, file, requests);
    }
    if shared.enabled(catalog::TEST_REDUNDANCY) {
        file.rules.insert(catalog::TEST_REDUNDANCY, 0);
        test_units::plan_pairs(context, (&cases, table), &subjects, file, requests);
    }
}

/// Short callee name to the other selected files that import `target` and call it.
fn callers(scope: &Scope<'_>, links: &Links, target: usize) -> BTreeMap<String, BTreeSet<PathBuf>> {
    let mut callers = BTreeMap::<String, BTreeSet<PathBuf>>::new();
    for &owner in links.importers(target).iter() {
        // Tests exercise a group; only application code makes it a dependency.
        let tests = scope.test_lines(owner);
        let units = scope.units[&owner].units.iter();
        for unit in units.filter(|u| !tests.iter().any(|l| u.overlaps(l))) {
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
