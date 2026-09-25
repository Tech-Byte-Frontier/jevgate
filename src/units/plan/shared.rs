//! Facts that span the selected files: clone candidates, imports, callable
//! subjects and their sources, routes, helpers, enums, constants and hashes.
use super::{Scope, java};
use crate::{
    analysis::{
        clones::{self, SourceFile},
        imports::Imports,
        routes::Route,
        test_map::{self, TestCase},
    },
    catalog,
    options::CheckArgs,
    units::{spacetimedb, test_units},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

/// Facts that span files: clone groups, imports, callable subjects and hashes.
pub(super) struct Shared<'a> {
    pub(super) rules: &'a [String],
    pub(super) pairs: clones::Candidates,
    pub(super) imports: BTreeMap<usize, Imports>,
    /// Callable short names to their signatures, for test subjects.
    pub(super) subjects: BTreeMap<String, String>,
    /// Java method short names to the Java types that own a method of that name.
    pub(super) subject_owners: BTreeMap<String, BTreeSet<String>>,
    /// Callable short names to their file and source, for the test recheck.
    pub(super) subject_sources: BTreeMap<String, test_units::SubjectSource>,
    /// Ruby methods defined among tests (in a test class, an example group
    /// or a support file) by short name, as the helpers a test calls.
    pub(super) helpers: BTreeMap<String, Vec<test_units::SubjectSource>>,
    /// Web routes and the full name of the controller method that serves each.
    pub(super) routes: Vec<(Route, String)>,
    /// Each controller method's first route, as `GET /owners/{ownerId}`.
    pub(super) route_labels: BTreeMap<String, String>,
    /// With access control, every callable by short name, for SpacetimeDB helpers.
    pub(super) module_helpers: BTreeMap<String, Vec<spacetimedb::Helper>>,
    /// Test cases of each selected file with a test view, inside its test lines.
    pub(super) cases: BTreeMap<PathBuf, Vec<TestCase>>,
    /// Enum definitions by name, from selected files and context, for security traces.
    pub(super) enums: BTreeMap<String, String>,
    /// C# constants by field name, as `Class.Field = value`, for security traces.
    pub(super) constants: BTreeMap<String, Vec<String>>,
    pub(super) hashes: BTreeMap<PathBuf, String>,
}

impl<'a> Shared<'a> {
    /// A test that sends a request, such as MockMvc's `get("/owners/{id}")`,
    /// calls the controller method whose route serves it, by its full name:
    /// controllers share method names such as `initCreationForm`.
    pub(super) fn link_routes(&self, cases: &mut [TestCase]) {
        for case in cases {
            for request in &case.requests {
                let served: Vec<(usize, &String)> = self
                    .routes
                    .iter()
                    .filter_map(|(route, name)| {
                        route.serves(request).map(|literal| (literal, name))
                    })
                    .collect();
                let best = served.iter().map(|(literal, _)| *literal).max();
                for (literal, name) in &served {
                    if Some(*literal) == best {
                        case.calls.insert((*name).clone());
                    }
                }
            }
        }
    }

    /// Name each subject by the type that owns it, `StringUtil::isBlank`,
    /// when one Java type in scope has a method of that name: an outline of
    /// bare method names hid that a Java test file covers one class. Other
    /// languages keep bare names: a Go test's `resp.Body.Close()` was named
    /// after the one type of the package that had a `Close` method.
    pub(super) fn qualify_subjects(&self, cases: &mut [TestCase]) {
        for case in cases {
            for subject in &mut case.subjects {
                if let Some(owners) = self.subject_owners.get(subject.as_str())
                    && let [owner] = owners.iter().collect::<Vec<_>>()[..]
                    && !owner.is_empty()
                {
                    *subject = format!("{owner}::{subject}");
                }
            }
        }
    }

    pub(super) fn new(scope: &Scope<'_>, args: &'a CheckArgs) -> Self {
        let cases = test_cases(scope);
        let mut shared = Self {
            rules: &args.rules,
            pairs: clones::Candidates::default(),
            imports: imports(scope),
            subjects: BTreeMap::new(),
            subject_owners: BTreeMap::new(),
            subject_sources: BTreeMap::new(),
            helpers: test_helpers(scope, &cases),
            routes: Vec::new(),
            route_labels: BTreeMap::new(),
            module_helpers: BTreeMap::new(),
            cases,
            enums: BTreeMap::new(),
            constants: BTreeMap::new(),
            hashes: BTreeMap::new(),
        };
        if shared.enabled(catalog::SHARED_LOGIC) {
            shared.pairs = duplicate_candidates(scope);
        }
        for (path, source, unit) in scope.scope_units() {
            shared
                .subjects
                .entry(unit.short_name.clone())
                .or_insert_with(|| unit.signature.clone());
            if java(path) {
                shared
                    .subject_owners
                    .entry(unit.short_name.clone())
                    .or_default()
                    .insert(unit.owner.clone());
            }
            shared
                .subject_sources
                .entry(unit.short_name.clone())
                .or_insert_with(|| test_units::SubjectSource {
                    path: path.to_path_buf(),
                    source: unit.source(source).to_string(),
                    shared: true,
                });
            if !unit.routes.is_empty() {
                shared
                    .subjects
                    .insert(unit.name.clone(), unit.signature.clone());
                shared.subject_sources.insert(
                    unit.name.clone(),
                    test_units::SubjectSource {
                        path: path.to_path_buf(),
                        source: unit.source(source).to_string(),
                        shared: true,
                    },
                );
                for route in &unit.routes {
                    shared.routes.push((route.clone(), unit.name.clone()));
                }
                let first = &unit.routes[0];
                let method = if first.method.is_empty() {
                    "ANY".to_string()
                } else {
                    first.method.to_uppercase()
                };
                shared
                    .route_labels
                    .insert(unit.name.clone(), format!("{method} {}", first.path));
            }
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
        if shared.enabled(catalog::ACCESS_CONTROL) {
            shared.module_helpers = module_helpers(scope, &shared.hashes);
        }
        if shared.enabled(catalog::INJECTION) {
            shared.enums = enums(scope);
        }
        if shared.enabled(catalog::UNSAFE_SETTINGS) {
            shared.constants = csharp_constants(scope);
        }
        shared
    }

    pub(super) fn enabled(&self, key: &str) -> bool {
        self.rules.iter().any(|r| r == key || r == catalog::id(key))
    }
}

/// Ruby methods inside the test lines of selected files, by short name: the
/// helpers tests call, such as `mock_app` in a support file or a `def` in an
/// example group. A file with test cases keeps its helpers to itself, as an
/// RSpec group scopes its methods; a support file, with none, shares them.
/// Other languages' tests show their helpers in the file.
fn test_helpers(
    scope: &Scope<'_>,
    cases: &BTreeMap<PathBuf, Vec<TestCase>>,
) -> BTreeMap<String, Vec<test_units::SubjectSource>> {
    let mut helpers = BTreeMap::<String, Vec<test_units::SubjectSource>>::new();
    for &owner in &scope.owners {
        let input = &scope.inputs[owner];
        if input.result.path.extension().is_none_or(|e| e != "rb") {
            continue;
        }
        let shared = cases.get(&input.result.path).is_none_or(Vec::is_empty);
        let lines = scope.test_lines(owner);
        let source = input.source.as_deref().unwrap_or("");
        for unit in scope.units[&owner].units.iter().filter(|u| u.callable()) {
            if lines.iter().any(|l| unit.overlaps(l)) {
                helpers.entry(unit.short_name.clone()).or_default().push(
                    test_units::SubjectSource {
                        path: input.result.path.clone(),
                        source: unit.source(source).to_string(),
                        shared,
                    },
                );
            }
        }
    }
    helpers
}

/// Every callable of the scope by short name, with its file's hash, as the
/// helpers a SpacetimeDB definition may call.
fn module_helpers(
    scope: &Scope<'_>,
    hashes: &BTreeMap<PathBuf, String>,
) -> BTreeMap<String, Vec<spacetimedb::Helper>> {
    let mut helpers = BTreeMap::<String, Vec<spacetimedb::Helper>>::new();
    for (path, source, unit) in scope.scope_units() {
        let Some(source_hash) = hashes.get(path) else {
            continue;
        };
        helpers
            .entry(unit.short_name.clone())
            .or_default()
            .push(spacetimedb::Helper {
                name: unit.short_name.clone(),
                path: path.to_path_buf(),
                source_hash: source_hash.clone(),
                source: unit.source(source).to_string(),
            });
    }
    helpers
}

/// An enum shown with a security trace is at most this long.
const ENUM_BYTES: usize = 1500;

/// Enum definitions in selected files and context by name; a name defined
/// twice is left out, since the site could mean either.
fn enums(scope: &Scope<'_>) -> BTreeMap<String, String> {
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
fn csharp_constants(scope: &Scope<'_>) -> BTreeMap<String, Vec<String>> {
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
        package: scope.inputs[owner].package.as_ref(),
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
            package: None,
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
