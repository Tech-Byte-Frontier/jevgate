//! Planning one selected code file: every rule's units and requests, with
//! the facts of the file and the scope each rule needs.
use super::{Scope, Shared, left_out, plan_security};
use crate::{
    analysis::{
        imports::Links,
        test_map::{self, TestCase},
        units::{FileUnits, Role, Unit},
    },
    catalog,
    file_kind::View,
    inventory::Input,
    options::CheckArgs,
    token_budget::Limits,
    units::{
        FileContext, FilePlan, Plan, Planned, comments, custom, duplicates, functions, guards,
        hardcoded, laws, outline, packs, spacetimedb, test_units,
    },
};
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range,
    path::{Path, PathBuf},
};

/// Every rule's units and requests for one selected file, then its custom
/// questions', which may ride in those requests.
pub(super) fn plan_file(
    scope: &Scope<'_>,
    shared: &Shared<'_>,
    owner: usize,
    args: &CheckArgs,
    budget: Limits<'_>,
    (requests, custom): (&mut Vec<Planned>, &mut custom::Planner),
) -> FilePlan {
    let since = requests.len();
    let input = &scope.inputs[owner];
    let view = &scope.views[&owner];
    let context = file_context(input, owner, args, budget);
    let mut file = FilePlan {
        path: input.result.path.clone(),
        left_out: left_out::entries(&scope.units[&owner], context.source, context.language),
        ..Default::default()
    };
    let lines = scope.test_lines(owner);
    let cases = shared.cases.get(context.path).cloned().unwrap_or_default();
    // What function simplification, hardcoded values and security ask about
    // each function, sent together once every rule has planned.
    let mut asks = Vec::new();
    // A file of the generic tier gets the rules its units serve: function
    // simplification, file organization, shared logic and comments. Values
    // and security need per-language knowledge its tag query does not give,
    // and its test files are not judged (`file_kind::generic_prepared`).
    let generic = scope.units[&owner].generic;
    if shared.enabled(catalog::FUNCTION_SIMPLIFICATION) {
        asks = plan_functions(scope, &context, view, &lines, &cases, &mut file);
    }
    if shared.enabled(catalog::FILE_ORGANIZATION) {
        plan_outline(scope, shared, &context, view, &lines, &mut file, requests);
    }
    // Example code spells its values out for the reader: sqlmodel's
    // `docs_src` tutorials each open a `database.db` with sample heroes. A
    // Bend 2 benchmark's values are its workload, the sizes, seeds and
    // ranges its C or TypeScript twin shares and its expected output pins:
    // 70 of 82 hardcoded-value findings in Bend 2 benchmarks were wrong.
    let benchmark = crate::analysis::bend::file(context.path)
        && crate::analysis::clones::benchmark_code(context.path);
    if shared.enabled(catalog::HARDCODED_VALUES)
        && !generic
        && view.application
        && !crate::analysis::clones::example_code(context.path)
        && !benchmark
    {
        let predicates = &shared.law_predicates;
        asks.extend(plan_values(
            &scope.units[&owner],
            &context,
            (&lines, predicates),
            &mut file,
            requests,
        ));
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
    if !rules.is_empty() && !generic && view.application {
        asks.extend(plan_security(
            scope, shared, &context, &lines, &rules, &mut file, requests,
        ));
    }
    packs::send(&context, asks, &mut file, requests);
    if shared.enabled(catalog::SHARED_LOGIC) && (view.application || view.tests) {
        file.rules.insert(catalog::SHARED_LOGIC, 0);
        duplicates::plan(&context, &shared.pairs, &shared.hashes, &mut file, requests);
    }
    if shared.enabled(catalog::LAWS)
        && view.application
        && crate::analysis::bend::file(context.path)
    {
        plan_laws(scope, shared, &context, &lines, &mut file, requests);
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
    let judged_tests = view.tests && args.include_tests;
    let code = custom::Code {
        parsed: &scope.units[&owner],
        test_lines: &lines,
        application: view.application,
        tests: judged_tests.then(|| {
            shared
                .cases
                .get(context.path)
                .map_or(&[][..], Vec::as_slice)
        }),
    };
    custom.code(&context, code, &mut file, (requests, since));
    file
}

/// Ask about the text addressed to a reviewer in one selected code file,
/// once every rule has planned its requests and `--base` has kept those
/// about what the change touched: only text a request sends can move an
/// answer.
pub(super) fn plan_steering(
    scope: &Scope<'_>,
    owner: usize,
    args: &CheckArgs,
    budget: Limits<'_>,
    plan: &mut Plan,
) {
    let input = &scope.inputs[owner];
    let source = input.source.as_deref().unwrap_or("");
    let tests = scope.test_lines(owner);
    // A string in test code is the test's data: JevGate's own tests of this
    // check hold steering examples, all read as steering.
    let texts: Vec<_> = crate::analysis::steering::texts(&input.result.path, source)
        .unwrap_or_default()
        .into_iter()
        .filter(|text| !(text.string && tests.iter().any(|lines| lines.contains(&text.line))))
        .collect();
    ask_steering(scope, owner, texts, (args, budget), plan);
}

/// Ask about each of `texts` of `owner` that one of its requests sends.
fn ask_steering(
    scope: &Scope<'_>,
    owner: usize,
    texts: Vec<crate::analysis::steering::Addressed>,
    (args, budget): (&CheckArgs, Limits<'_>),
    plan: &mut Plan,
) {
    if let (false, Some(file)) = (texts.is_empty(), plan.files.get_mut(&owner)) {
        let context = file_context(&scope.inputs[owner], owner, args, budget);
        guards::plan_steering(&context, texts, file, &mut plan.requests);
    }
}

/// Ask about each paragraph of a document, such as an instruction file,
/// that addresses a reviewer and that a request of the document sends: a
/// section asked beside it cannot clear on its answers, as a function
/// asked beside a steering comment cannot.
pub(super) fn plan_document_steering(
    scope: &Scope<'_>,
    owner: usize,
    args: &CheckArgs,
    budget: Limits<'_>,
    plan: &mut Plan,
) {
    let source = scope.inputs[owner].source.as_deref().unwrap_or("");
    let texts = crate::analysis::steering::paragraphs(
        source,
        crate::analysis::steering::Audience::Reviewers,
    );
    ask_steering(scope, owner, texts, (args, budget), plan);
}

/// Ask about each paragraph of a text file only custom questions read (a
/// source file without a parser, Terraform, a shell script) that addresses
/// a reviewer or a model and that a request of the file sends: a `file` or
/// `hunk` unit asked beside it cannot clear on its answers.
pub(super) fn plan_text_steering(
    scope: &Scope<'_>,
    owner: usize,
    args: &CheckArgs,
    budget: Limits<'_>,
    plan: &mut Plan,
) {
    let source = scope.inputs[owner].source.as_deref().unwrap_or("");
    let texts =
        crate::analysis::steering::paragraphs(source, crate::analysis::steering::Audience::Anyone);
    ask_steering(scope, owner, texts, (args, budget), plan);
}

/// One selected code file's facts, with the role a web framework gives it.
fn file_context<'a>(
    input: &'a Input,
    owner: usize,
    args: &'a CheckArgs,
    budget: Limits<'a>,
) -> FileContext<'a> {
    FileContext {
        owner,
        path: &input.result.path,
        language: crate::file_kind::read_language(
            &input.result.path,
            input.source.as_deref().unwrap_or(""),
        ),
        source: input.source.as_deref().unwrap_or(""),
        source_hash: &input.result.source_hash,
        model: args.model(),
        budget,
        project: args.project.as_deref(),
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
        changed: input.changed.as_ref(),
    }
}

/// An application file's members outside tests, or a test file's cases;
/// none when syntax errors leave too little of the file to describe it.
/// With a change judged, the outline is asked only when the change adds a
/// member: editing a body leaves the file's layout as it was.
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
    if crate::analysis::bend::law_file(context.path) {
        return;
    }
    let parsed = &scope.units[&owner];
    let units = &parsed.units;
    if view.application {
        let members: Vec<usize> = (0..units.len())
            .filter(|&i| !lines.iter().any(|l| units[i].overlaps(l)))
            .collect();
        file.rules.insert(catalog::FILE_ORGANIZATION, 0);
        let listed = members
            .iter()
            .map(|&i| (units[i].name.as_str(), units[i].line));
        if members.len() >= 2
            && context.adds(listed, unit_names)
            && left_out::outline_covered(parsed, context.source, file)
        {
            let callers = callers(scope, &shared.links, owner);
            outline::plan(context, parsed, &members, &callers, file, requests);
        }
    } else if view.classification.kind == crate::file_kind::TESTS {
        file.rules.insert(catalog::FILE_ORGANIZATION, 0);
        let mut cases = test_map::cases(context.path, context.source).unwrap_or_default();
        let listed = cases
            .iter()
            .map(|c| (c.name.as_str(), c.line))
            .chain(units.iter().map(|u| (u.name.as_str(), u.line)));
        if !context.adds(listed, test_names)
            || !left_out::outline_covered(parsed, context.source, file)
        {
            return;
        }
        shared.link_routes(&mut cases);
        test_map::link(&mut cases, &shared.subjects.keys().cloned().collect());
        if java(context.path) {
            shared.qualify_subjects(&mut cases);
        }
        outline::plan_tests(context, parsed, &cases, file, requests);
    }
}

/// The names of the units of a file's base version.
fn unit_names(path: &Path, source: &str) -> BTreeSet<String> {
    crate::analysis::units::parse(path, source)
        .map(|parsed| parsed.units.into_iter().map(|u| u.name).collect())
        .unwrap_or_default()
}

/// The names of the units and test cases of a test file's base version.
fn test_names(path: &Path, source: &str) -> BTreeSet<String> {
    let mut names = unit_names(path, source);
    let cases = test_map::cases(path, source).unwrap_or_default();
    names.extend(cases.into_iter().map(|case| case.name));
    names
}

/// Callables and module constants outside tests; returns what the first
/// pass asks about the callables.
/// A Bend 2 law's predicates and the defs only they call hold its samples,
/// so their literals are not asked about.
fn plan_values(
    parsed: &FileUnits,
    context: &FileContext<'_>,
    (lines, predicates): (&[Range<usize>], &BTreeSet<String>),
    file: &mut FilePlan,
    requests: &mut Vec<Planned>,
) -> Vec<packs::FunctionAsk> {
    file.rules.insert(catalog::HARDCODED_VALUES, 0);
    let outside_tests = |line: usize| !lines.iter().any(|l| l.contains(&line));
    // A Bend 2 proof's literals state its property (`1n+p`, `{Nat.add(x,
    // 0n) == x : Nat}`), and a type-level def's are part of a type.
    let units: Vec<(usize, &Unit)> = parsed
        .units
        .iter()
        .enumerate()
        .filter(|(_, u)| u.callable() && u.role == Role::Code && outside_tests(u.line))
        .filter(|(_, u)| !predicates.contains(&u.name))
        .collect();
    // With a change judged, only the constants on its lines are asked.
    let constants: Vec<_> = parsed
        .constants
        .iter()
        .filter(|c| outside_tests(c.line) && context.judges(c.line, c.end_line))
        .cloned()
        .collect();
    hardcoded::plan(context, &units, &constants, file, requests)
}

/// The claims of a Bend 2 file outside tests, with the defs they name: the
/// file's own and those of the files it imports.
fn plan_laws(
    scope: &Scope<'_>,
    shared: &Shared<'_>,
    context: &FileContext<'_>,
    lines: &[Range<usize>],
    file: &mut FilePlan,
    requests: &mut Vec<Planned>,
) {
    let outside_tests = |line: usize| !lines.iter().any(|l| l.contains(&line));
    let units: Vec<Unit> = scope.units[&context.owner]
        .units
        .iter()
        .filter(|u| outside_tests(u.line))
        .cloned()
        .collect();
    let defs: Vec<laws::Named<'_>> = shared
        .links
        .reachable_from(context.owner)
        .iter()
        .filter_map(|owner| {
            let source = scope.inputs[*owner].source.as_deref()?;
            Some(
                scope
                    .units
                    .get(owner)?
                    .units
                    .iter()
                    .map(move |unit| laws::Named { unit, source }),
            )
        })
        .flatten()
        .filter(|named| named.unit.callable())
        .collect();
    laws::plan(context, &units, &shared.propositions, &defs, file, requests);
}

/// Comments outside tests and outside what syntax errors left out: a
/// comment in a left-out function would read as top-level code's.
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
        .filter(|c| !lines.iter().any(|l| l.contains(&c.line)) && parsed.intact(&c.span))
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
/// support (not test cases) with the test view. A Bend 2 proof is left out:
/// its steps follow the cases of what it proves, not jobs a reader could
/// pull apart, and the 16 function-simplification findings on proofs across
/// 41 Bend 2 projects were all wrong (2 more debatable). Returns what the
/// first pass asks about them.
fn plan_functions(
    scope: &Scope<'_>,
    context: &FileContext<'_>,
    view: &View,
    lines: &[Range<usize>],
    cases: &[TestCase],
    file: &mut FilePlan,
) -> Vec<packs::FunctionAsk> {
    let judged: Vec<(usize, &Unit)> = scope.units[&context.owner]
        .units
        .iter()
        .enumerate()
        .filter(|(_, u)| u.callable() && u.role != Role::Proof)
        .filter(|(_, u)| {
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
    if !view.application && judged.is_empty() {
        return Vec::new();
    }
    file.rules.insert(catalog::FUNCTION_SIMPLIFICATION, 0);
    functions::plan(context, &judged, scope, file)
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
            // A function passed by path is used like a call.
            for call in unit.calls.iter().chain(&unit.passed) {
                callers
                    .entry(call.clone())
                    .or_default()
                    .insert(scope.inputs[owner].result.path.clone());
            }
        }
    }
    callers
}
