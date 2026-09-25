//! Facts that span the selected files: clone candidates, imports, callable
//! subjects and their sources, routes, helpers, enums, constants and hashes.
use super::{
    Scope, java,
    trace_evidence::{csharp_constants, enums},
};
use crate::{
    analysis::{
        clones::{self, SourceFile},
        imports::Imports,
        routes::Route,
        test_map::{self, TestCase},
        units::Unit,
    },
    catalog,
    options::CheckArgs,
    units::{spacetimedb, test_units},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
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
            hashes: source_hashes(scope),
        };
        if shared.enabled(catalog::SHARED_LOGIC) {
            shared.pairs = duplicate_candidates(scope);
        }
        for (path, source, unit) in scope.scope_units() {
            shared.add_subject(path, source, unit);
            if !unit.routes.is_empty() {
                shared.add_routes(path, source, unit);
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

    /// A callable as a test subject by its short name, the first of that
    /// name winning; a Java method also names the type that owns it.
    fn add_subject(&mut self, path: &Path, source: &str, unit: &Unit) {
        self.subjects
            .entry(unit.short_name.clone())
            .or_insert_with(|| unit.signature.clone());
        if java(path) {
            self.subject_owners
                .entry(unit.short_name.clone())
                .or_default()
                .insert(unit.owner.clone());
        }
        self.subject_sources
            .entry(unit.short_name.clone())
            .or_insert_with(|| test_units::SubjectSource {
                path: path.to_path_buf(),
                source: unit.source(source).to_string(),
                shared: true,
            });
    }

    /// A controller method as a subject by its full name, since controllers
    /// share method names, with the routes that reach it and its first route
    /// as a label.
    fn add_routes(&mut self, path: &Path, source: &str, unit: &Unit) {
        self.subjects
            .insert(unit.name.clone(), unit.signature.clone());
        self.subject_sources.insert(
            unit.name.clone(),
            test_units::SubjectSource {
                path: path.to_path_buf(),
                source: unit.source(source).to_string(),
                shared: true,
            },
        );
        for route in &unit.routes {
            self.routes.push((route.clone(), unit.name.clone()));
        }
        let first = &unit.routes[0];
        let method = if first.method.is_empty() {
            "ANY".to_string()
        } else {
            first.method.to_uppercase()
        };
        self.route_labels
            .insert(unit.name.clone(), format!("{method} {}", first.path));
    }

    pub(super) fn enabled(&self, key: &str) -> bool {
        self.rules.iter().any(|r| r == key || r == catalog::id(key))
    }
}

/// The source hash of every selected file and explicit context file.
fn source_hashes(scope: &Scope<'_>) -> BTreeMap<PathBuf, String> {
    let mut hashes = BTreeMap::new();
    for &owner in &scope.owners {
        let result = &scope.inputs[owner].result;
        hashes.insert(result.path.clone(), result.source_hash.clone());
    }
    if let Some(first) = scope.owners.first() {
        for context in &scope.inputs[*first].context {
            hashes.insert(context.file.path.clone(), context.file.source_hash.clone());
        }
    }
    hashes
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
