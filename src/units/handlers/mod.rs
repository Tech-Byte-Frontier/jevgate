//! Web framework error handlers: found where the program registers them
//! (`.onError(…)`, `.setErrorHandler(…)`, Express error middleware, Flask and
//! FastAPI decorators, Django URLconf error views and Django REST framework's
//! `EXCEPTION_HANDLER`) or implements them (axum `IntoResponse` and actix-web
//! `ResponseError` for an error type, Rocket catchers, NestJS exception
//! filters, Django middleware `process_exception`), and asked once each
//! whether they send clients more than the
//! program's own messages and codes, with the program's error classes as
//! evidence.
mod classes;
mod implemented;
mod registered;

use super::{
    Detail, FileContext, FilePlan, Plan, Planned, Presence, Questions, UnitPlan, compact, identity,
    plan::Scope, questions,
};
use crate::{
    analysis::{imports::Links, units::Unit},
    catalog::SENSITIVE_DATA,
    options::CheckArgs,
    schema::Pass,
    token_budget::TokenBudget,
};
use classes::error_classes;
use implemented::implemented;
use registered::{decorated, django_views, registered};
use serde_json::json;
use std::{collections::BTreeMap, path::PathBuf};

/// What handler lookups need from the whole scope.
pub(super) struct Evidence<'a> {
    pub links: &'a Links,
    pub hashes: &'a BTreeMap<PathBuf, String>,
}

/// One unit per registered handler, on the file that defines it.
pub(super) fn plan(
    scope: &Scope<'_>,
    evidence: &Evidence<'_>,
    args: &CheckArgs,
    budget: &TokenBudget,
    result: &mut Plan,
) {
    let handlers = error_handlers(scope, evidence.links);
    let classes = error_classes(scope, evidence.hashes);
    for handler in &handlers {
        let input = &scope.inputs[handler.owner];
        let context = FileContext {
            owner: handler.owner,
            path: &input.result.path,
            language: crate::file_kind::language(&input.result.path),
            source: input.source.as_deref().unwrap_or(""),
            source_hash: &input.result.source_hash,
            model: args.model(),
            budget,
            framework: super::nextjs::describe(
                &input.result.path,
                input.source.as_deref().unwrap_or(""),
                input.package.as_ref(),
            ),
        };
        if let Some(file) = result.files.get_mut(&handler.owner) {
            let django = scope.units[&handler.owner].django;
            plan_handler(
                &context,
                handler,
                &classes,
                django,
                file,
                &mut result.requests,
            );
        }
    }
}

/// Error handlers registered in application code outside tests, once each.
fn error_handlers(scope: &Scope<'_>, links: &Links) -> Vec<Handler> {
    let mut found: Vec<Handler> = Vec::new();
    for &owner in &scope.owners {
        if !scope.views[&owner].application {
            continue;
        }
        let file = registered(scope, links, owner)
            .into_iter()
            .chain(decorated(scope, owner))
            .chain(django_views(scope, links, owner))
            .chain(implemented(scope, owner));
        for handler in file {
            if !found
                .iter()
                .any(|h| h.owner == handler.owner && h.lines == handler.lines)
            {
                let helpers = handler_helpers(scope, &handler);
                found.push(Handler { helpers, ..handler });
            }
        }
    }
    found
}

/// Helper functions shown with a handler: at most this many, each whole and
/// at most `HELPER_BYTES` long.
const HELPERS: usize = 4;
const HELPER_BYTES: usize = 3000;

/// Functions in the handler's file that it calls, and those they call: an
/// axum handler often delegates its body to `self.error_response()`, whose
/// messages decide what clients see.
fn handler_helpers(scope: &Scope<'_>, handler: &Handler) -> Vec<String> {
    let source = scope.inputs[handler.owner].source.as_deref().unwrap_or("");
    let units = &scope.units[&handler.owner].units;
    let own = units
        .iter()
        .find(|u| u.callable() && (u.line, u.end_line) == handler.lines);
    let mut calls: Vec<&String> = own.map(|u| u.calls.iter().collect()).unwrap_or_default();
    let mut found: Vec<&crate::analysis::units::Unit> = Vec::new();
    for _ in 0..2 {
        let mut next = Vec::new();
        for name in calls {
            let callee = units.iter().find(|u| {
                u.callable()
                    && &u.short_name == name
                    && (u.line, u.end_line) != handler.lines
                    && u.source(source).len() <= HELPER_BYTES
            });
            if let Some(callee) = callee
                && found.len() < HELPERS
                && !found.iter().any(|f| f.span == callee.span)
            {
                found.push(callee);
                next.extend(callee.calls.iter());
            }
        }
        calls = next;
    }
    found.iter().map(|u| u.source(source).to_string()).collect()
}

/// The application function a registration names: in the registering file,
/// else its only definition among the selected files, else the one in a
/// file the registering file imports. Imports often pass through a barrel
/// module, so a unique name is enough.
fn named_handler(
    scope: &Scope<'_>,
    links: &Links,
    owner: usize,
    name: &str,
) -> Option<(usize, String, String, (usize, usize))> {
    let definitions: Vec<(usize, &Unit)> = std::iter::once(owner)
        .chain(scope.owners.iter().copied().filter(|&o| o != owner))
        .filter(|o| scope.views[o].application)
        .flat_map(|o| {
            let lines = scope.test_lines(o);
            scope.units[&o]
                .units
                .iter()
                .filter(move |u| {
                    u.callable() && u.short_name == name && !lines.iter().any(|l| u.overlaps(l))
                })
                .map(move |u| (o, u))
        })
        .collect();
    let (found, unit) = definitions
        .iter()
        .find(|(o, _)| *o == owner)
        .or_else(|| (definitions.len() == 1).then(|| &definitions[0]))
        .or_else(|| definitions.iter().find(|(o, _)| links.reach(owner, *o)))?;
    let source = scope.inputs[*found].source.as_deref().unwrap_or("");
    Some((
        *found,
        unit.name.clone(),
        unit.source(source).to_string(),
        (unit.line, unit.end_line),
    ))
}

/// A function a web framework calls for every error a request handler
/// throws, found where the program registers it.
pub(super) struct Handler {
    pub owner: usize,
    pub name: String,
    pub source: String,
    pub lines: (usize, usize),
    /// The registration as written and where: `app.onError(errorHandler)` (src/app.ts:150).
    pub registered: String,
    /// Functions of its file that it calls, two deep, such as the method
    /// that builds the response body.
    pub helpers: Vec<String>,
}

/// The program's own error classes, as evidence for the handler question:
/// their text and the files it comes from.
pub(super) struct ErrorClasses {
    pub text: String,
    pub sources: Vec<(std::path::PathBuf, String)>,
}

/// One unit per registered error handler: whether it sends clients more
/// than the program's own messages and codes. The functions whose
/// responses it writes are judged by their own messages.
fn plan_handler(
    file: &FileContext<'_>,
    handler: &Handler,
    classes: &ErrorClasses,
    django: bool,
    out: &mut FilePlan,
    requests: &mut Vec<Planned>,
) {
    let id = format!("handler:{}", handler.name);
    let mut questions = Questions::default();
    questions.ask(
        "handler_leaks".into(),
        questions::security_handler_leaks(django),
        &id,
        SENSITIVE_DATA,
        "handler_leaks",
        Pass::First,
    );
    let state = json!({
        "file": file.file_state(),
        "error_handler": if handler.helpers.is_empty() {
            json!({"registered": handler.registered, "source": handler.source})
        } else {
            json!({"registered": handler.registered, "source": handler.source, "helpers": handler.helpers})
        },
        "error_classes": classes.text,
    });
    let mut sources = vec![(file.path, file.source_hash)];
    sources.extend(
        classes
            .sources
            .iter()
            .filter(|(path, _)| path != file.path)
            .map(|(path, hash)| (path.as_path(), hash.as_str())),
    );
    let questions = questions.reworded(file.language);
    let (request, asked) = super::request(file.model, "security", &sources, state, questions);
    let fits = file.budget.fits(&request);
    out.units.push(UnitPlan {
        rule: SENSITIVE_DATA,
        id,
        name: handler.name.clone(),
        presence: if fits {
            Presence::Judged
        } else {
            Presence::NeedsContext
        },
        locations: vec![file.location(handler.lines.0, handler.lines.1, Some(&handler.name))],
        quote: None,
        lines: handler.lines.1 + 1 - handler.lines.0,
        identity: identity(&["error handler", &handler.name, &compact(&handler.source)]),
        detail: Detail::Handler {
            registered: handler.registered.clone(),
        },
        recheck: None,
    });
    if fits {
        requests.push(Planned {
            owner: file.owner,
            request,
            asked,
        });
    }
}
