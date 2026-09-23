//! Web framework error handlers: found where the program registers them
//! (`.onError(…)`, `.setErrorHandler(…)`, Flask and FastAPI decorators) and
//! asked once each whether they send clients more than the program's own
//! messages and codes, with the program's error classes as evidence.
use super::{
    Detail, FileContext, FilePlan, Plan, Planned, Presence, Questions, UnitPlan, compact, identity,
    plan::Scope, questions,
};
use crate::{
    analysis::{imports::Imports, units::Unit},
    catalog::SENSITIVE_DATA,
    options::CheckArgs,
    schema::Pass,
    token_budget::TokenBudget,
};
use serde_json::json;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

/// Calls that register a web framework's error handler, by the method that
/// takes it; the handler is the function named or written in the call.
const HANDLER_REGISTRATIONS: [&str; 2] = [".onError(", ".setErrorHandler("];
/// Python decorators that register the function below them as an error handler.
const HANDLER_DECORATORS: [&str; 2] = [".exception_handler(", ".errorhandler("];
/// Error classes shown with a handler question, whole, up to this many bytes.
const ERROR_CLASS_BYTES: usize = 3000;

/// What handler lookups need from the whole scope.
pub(super) struct Evidence<'a> {
    pub imports: &'a BTreeMap<usize, Imports>,
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
    let handlers = error_handlers(scope, evidence.imports);
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
        };
        if let Some(file) = result.files.get_mut(&handler.owner) {
            plan_handler(&context, handler, &classes, file, &mut result.requests);
        }
    }
}

/// Error handlers registered in application code outside tests, once each.
fn error_handlers(scope: &Scope<'_>, imports: &BTreeMap<usize, Imports>) -> Vec<Handler> {
    let mut found: Vec<Handler> = Vec::new();
    for &owner in &scope.owners {
        if !scope.views[&owner].application {
            continue;
        }
        let file = registered(scope, imports, owner)
            .into_iter()
            .chain(decorated(scope, owner));
        for handler in file {
            if !found
                .iter()
                .any(|h| h.owner == handler.owner && h.lines == handler.lines)
            {
                found.push(handler);
            }
        }
    }
    found
}

/// Handlers one file passes to a registration call: a function named there,
/// or the function written inside the call.
fn registered(scope: &Scope<'_>, imports: &BTreeMap<usize, Imports>, owner: usize) -> Vec<Handler> {
    let input = &scope.inputs[owner];
    let source = input.source.as_deref().unwrap_or("");
    let lines = scope.test_lines(owner);
    let mut found = Vec::new();
    for needle in HANDLER_REGISTRATIONS {
        for (at, _) in source.match_indices(needle) {
            let line = crate::analysis::line_of(source, at);
            let open = at + needle.len();
            let Some(argument) = call_argument(&source[open..]) else {
                continue;
            };
            if lines.iter().any(|l| l.contains(&line)) {
                continue;
            }
            let start = source[..at]
                .rfind(['\n', ' ', '\t', '('])
                .map_or(0, |i| i + 1);
            let registered = format!(
                "`{}` ({}:{line})",
                &source[start..open + argument.len() + 1],
                input.result.path.display()
            );
            let named = argument
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == '$');
            let handler = if named {
                named_handler(scope, imports, owner, argument)
            } else {
                let first = crate::analysis::line_of(source, open);
                let last = crate::analysis::line_of(source, open + argument.len());
                Some((
                    owner,
                    "error handler".into(),
                    argument.into(),
                    (first, last),
                ))
            };
            if let Some((owner, name, source, lines)) = handler {
                found.push(Handler {
                    owner,
                    name,
                    source,
                    lines,
                    registered,
                });
            }
        }
    }
    found
}

/// Python functions a decorator registers as error handlers.
fn decorated(scope: &Scope<'_>, owner: usize) -> Vec<Handler> {
    let input = &scope.inputs[owner];
    let source = input.source.as_deref().unwrap_or("");
    let lines = scope.test_lines(owner);
    scope.units[&owner]
        .units
        .iter()
        .filter(|u| u.callable() && !lines.iter().any(|l| l.contains(&u.line)))
        .filter_map(|unit| {
            let text = unit.source(source);
            let decorator = text.lines().take_while(|l| !l.contains("def ")).find(|l| {
                l.trim_start().starts_with('@') && HANDLER_DECORATORS.iter().any(|d| l.contains(d))
            })?;
            Some(Handler {
                owner,
                name: unit.name.clone(),
                source: text.to_string(),
                lines: (unit.line, unit.end_line),
                registered: format!(
                    "`{}` ({}:{})",
                    decorator.trim(),
                    input.result.path.display(),
                    unit.line
                ),
            })
        })
        .collect()
}

/// The text inside a call's parentheses, from just after `(` to its match.
fn call_argument(text: &str) -> Option<&str> {
    let mut depth = 1usize;
    for (i, c) in text.char_indices() {
        match c {
            '(' | '{' | '[' => depth += 1,
            ')' | '}' | ']' => {
                depth -= 1;
                if depth == 0 {
                    let argument = text[..i].trim();
                    return (!argument.is_empty()).then_some(argument);
                }
            }
            _ => {}
        }
    }
    None
}

/// The application function a registration names: in the registering file,
/// else its only definition among the selected files, else the one in a
/// file the registering file imports. Imports often pass through a barrel
/// module, so a unique name is enough.
fn named_handler(
    scope: &Scope<'_>,
    imports: &BTreeMap<usize, Imports>,
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
        .or_else(|| {
            definitions
                .iter()
                .find(|(o, _)| imports[&owner].reach(&scope.inputs[*o].result.path))
        })?;
    let source = scope.inputs[*found].source.as_deref().unwrap_or("");
    Some((
        *found,
        unit.name.clone(),
        unit.source(source).to_string(),
        (unit.line, unit.end_line),
    ))
}

/// Classes named `…Error` or `…Exception` in selected application files and
/// context, whole, in path order, while they fit.
fn error_classes(scope: &Scope<'_>, hashes: &BTreeMap<PathBuf, String>) -> ErrorClasses {
    let mut classes = ErrorClasses {
        text: String::new(),
        sources: Vec::new(),
    };
    let selected = scope
        .owners
        .iter()
        .filter(|o| scope.views[o].application)
        .map(|&o| {
            let input = &scope.inputs[o];
            (
                input.result.path.as_path(),
                input.source.as_deref().unwrap_or(""),
            )
        });
    let context = scope
        .context
        .iter()
        .map(|(path, source, _)| (path.as_path(), *source));
    let mut all: Vec<_> = selected.chain(context).collect();
    all.sort_by_key(|(path, _)| *path);
    for (path, source) in all {
        for text in error_class_texts(path, source) {
            if classes.text.len() + text.len() > ERROR_CLASS_BYTES {
                continue;
            }
            if !classes.text.is_empty() {
                classes.text.push_str("\n\n");
            }
            classes.text.push_str(text);
            if let Some(hash) = hashes.get(path)
                && !classes.sources.iter().any(|(p, _)| p == path)
            {
                classes.sources.push((path.to_path_buf(), hash.clone()));
            }
        }
    }
    classes
}

/// The whole text of each class whose name ends in `Error` or `Exception`:
/// through its closing brace, or its indented body in Python.
fn error_class_texts<'a>(path: &Path, source: &'a str) -> Vec<&'a str> {
    let python = path.extension().is_some_and(|e| e == "py");
    let mut found = Vec::new();
    let mut offset = 0;
    for line in source.split_inclusive('\n') {
        let start = offset;
        offset += line.len();
        let declaration = line
            .trim_start_matches("export ")
            .trim_start_matches("default ")
            .trim_start_matches("abstract ");
        let Some(rest) = declaration.strip_prefix("class ") else {
            continue;
        };
        let name: String = rest
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '$')
            .collect();
        if !(name.ends_with("Error") || name.ends_with("Exception")) {
            continue;
        }
        let text = &source[start..];
        let end = if python {
            let mut end = line.len();
            for next in text[line.len()..].split_inclusive('\n') {
                if !next.trim().is_empty() && !next.starts_with([' ', '\t']) {
                    break;
                }
                end += next.len();
            }
            Some(end)
        } else {
            text.find('{').and_then(|open| {
                let mut depth = 0usize;
                text[open..].char_indices().find_map(|(i, c)| {
                    match c {
                        '{' => depth += 1,
                        '}' => depth -= 1,
                        _ => {}
                    }
                    (depth == 0).then_some(open + i + 1)
                })
            })
        };
        if let Some(end) = end {
            found.push(text[..end].trim_end());
        }
    }
    found
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
    out: &mut FilePlan,
    requests: &mut Vec<Planned>,
) {
    let id = format!("handler:{}", handler.name);
    let mut questions = Questions::default();
    questions.ask(
        "handler_leaks".into(),
        questions::security_handler_leaks(),
        &id,
        SENSITIVE_DATA,
        "handler_leaks",
        Pass::First,
    );
    let state = json!({
        "file": file.file_state(),
        "error_handler": {"registered": handler.registered, "source": handler.source},
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
