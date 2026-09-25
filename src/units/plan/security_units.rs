//! Security units of one file: its application functions and setup
//! statements, with the evidence each check needs, such as the callers of a
//! function or, in Django code, the routes, templates and settings modules
//! around it (`settings_modules`). The enums and C# constants of the scope
//! that traces show are in `trace_evidence`.
use super::{
    Scope, Shared,
    settings_modules::{selections, settings_extended_by},
};
use crate::{
    analysis::{imports::Imports, units::Unit},
    catalog,
    units::{FileContext, FilePlan, Planned, security},
};
use std::{collections::BTreeMap, ops::Range};

/// Application functions outside tests and the file's setup statements; the
/// injection recheck shows up to three callers of each function.
pub(super) fn plan_security(
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
            let mut subject = security::function_subject(
                context,
                unit,
                callers,
                &shared.enums,
                &shared.constants,
            );
            subject.django = parsed.django;
            if !parsed.django {
                return subject;
            }
            if let Some(command) = crate::analysis::django::management_command(context.path) {
                subject.evidence.insert(
                    "django_management_command".into(),
                    serde_json::json!(format!(
                        "A person runs it by hand with `manage.py {command}`; its options come from that person's command line."
                    )),
                );
            }
            let constants = constants_used(&parsed.module_constants, &subject.source);
            if !constants.is_empty() {
                subject.evidence.insert(
                    "module_constants_it_uses".into(),
                    serde_json::json!(constants),
                );
            }
            let routes = routes_to(scope, context, unit);
            if !routes.is_empty() {
                subject.evidence.insert(
                    "url_routes_that_send_requests_to_it".into(),
                    serde_json::json!(routes),
                );
            }
            let templates: Vec<serde_json::Value> = crate::analysis::django::rendered(
                subject.source.as_str(),
                &scope.inputs[context.owner].templates,
            )
            .into_iter()
            .take(crate::analysis::django::TEMPLATES)
            .map(|t| {
                serde_json::json!({
                    "template": t.path.display().to_string(),
                    "unescaped_output": t.unescaped,
                })
            })
            .collect();
            if rules.contains(&catalog::INJECTION) && !templates.is_empty() {
                subject.evidence.insert(
                    "templates_it_renders_that_write_values_without_escaping".into(),
                    serde_json::json!(templates),
                );
            }
            subject
        })
        .collect();
    let mut setup = security::setup_subject(context, &parsed.setup, &shared.constants)
        .filter(|_| parsed.setup.statements.iter().all(|s| outside_tests(s.1)));
    // Only a settings module's statements are asked the Django settings
    // checks; other top-level setup, such as `wsgi.py` choosing a default
    // settings module, keeps the common questions.
    if let Some(setup) = setup.as_mut() {
        setup.django = parsed.setup.settings;
    }
    if let Some(setup) = setup.as_mut().filter(|_| parsed.setup.settings) {
        let selected = selections(&scope.inputs[context.owner].settings_selected_by);
        if !selected.is_empty() {
            setup.evidence.insert(
                "selected_as_the_settings_to_run_with_by".into(),
                serde_json::json!(selected),
            );
        }
        let extending = settings_extended_by(scope, context.owner);
        if !extending.is_empty() {
            setup.evidence.insert(
                "settings_modules_that_import_it".into(),
                serde_json::json!(extending),
            );
        }
    }
    security::plan(
        context,
        &subjects,
        setup,
        rules,
        parsed.django,
        file,
        requests,
    );
}

/// Module constants shown with one function, at most.
const CONSTANTS: usize = 4;

/// The assignments of the module constants a function's source names as a
/// whole word, such as the base directory it joins file names to.
fn constants_used(constants: &[(String, String)], source: &str) -> Vec<String> {
    let word = |c: char| c.is_alphanumeric() || c == '_';
    constants
        .iter()
        .filter(|(name, _)| {
            source.match_indices(name.as_str()).any(|(at, _)| {
                source[..at].chars().next_back().is_none_or(|c| !word(c))
                    && source[at + name.len()..]
                        .chars()
                        .next()
                        .is_none_or(|c| !word(c))
            })
        })
        .map(|(_, shown)| shown.clone())
        .take(CONSTANTS)
        .collect()
}

/// URL routes shown with one view, at most.
const ROUTES: usize = 3;

/// The Django URL routes, in selected files, that send requests to a
/// function or to the class of a request method, as `path:line: text`: they
/// show that it answers outside requests and what its URL parameters hold,
/// such as digits only for `(?P<task_id>\d+)`.
fn routes_to(
    scope: &Scope<'_>,
    context: &FileContext<'_>,
    unit: &crate::analysis::units::Unit,
) -> Vec<String> {
    scope
        .owners
        .iter()
        .flat_map(|&owner| {
            let path = &scope.inputs[owner].result.path;
            scope.units[&owner]
                .routes
                .iter()
                .filter(|route| {
                    crate::analysis::django::routes_to(
                        route,
                        context.path,
                        &unit.short_name,
                        &unit.owner,
                    )
                })
                .map(move |route| format!("{}:{}: {}", path.display(), route.line, route.text))
        })
        .take(ROUTES)
        .collect()
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
