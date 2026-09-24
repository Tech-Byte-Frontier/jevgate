//! Web framework error handlers: found where the program registers them
//! (`.onError(…)`, `.setErrorHandler(…)`, Express error middleware, Flask and
//! FastAPI decorators) or implements them (axum `IntoResponse` and actix-web
//! `ResponseError` for an error type, Rocket catchers, NestJS exception
//! filters), and asked once each whether they send clients more than the
//! program's own messages and codes, with the program's error classes as
//! evidence.
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
/// Express registers a function of four parameters (`err, req, res, next`)
/// passed to `.use(…)` as its error handler.
const MIDDLEWARE_REGISTRATION: &str = ".use(";
const MIDDLEWARE_PARAMETERS: usize = 4;
/// ASP.NET Core's exception handler middleware runs the lambda it is given
/// for every unhandled exception; given a path (`"/Error"`), it re-executes
/// a page instead, which is judged as the page's own code.
const EXCEPTION_HANDLER_REGISTRATION: &str = ".UseExceptionHandler(";
/// ASP.NET Core interfaces and base classes whose method handles every
/// exception a controller or request throws, by the method they define.
const CSHARP_HANDLER_TYPES: [(&str, &str); 5] = [
    ("IExceptionFilter", "OnException"),
    ("IAsyncExceptionFilter", "OnExceptionAsync"),
    ("ExceptionFilterAttribute", "OnException"),
    ("ExceptionFilterAttribute", "OnExceptionAsync"),
    ("IExceptionHandler", "TryHandleAsync"),
];
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
            framework: super::nextjs::describe(
                &input.result.path,
                input.source.as_deref().unwrap_or(""),
                input.package.as_ref(),
            ),
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
            .chain(decorated(scope, owner))
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

/// Handlers one file passes to a registration call: a function named there,
/// or the function written inside the call.
fn registered(scope: &Scope<'_>, imports: &BTreeMap<usize, Imports>, owner: usize) -> Vec<Handler> {
    let input = &scope.inputs[owner];
    let source = input.source.as_deref().unwrap_or("");
    let lines = scope.test_lines(owner);
    let tree = crate::syntax::parse(&input.result.path, source)
        .ok()
        .flatten();
    let mut found = Vec::new();
    for needle in HANDLER_REGISTRATIONS
        .into_iter()
        .chain([MIDDLEWARE_REGISTRATION, EXCEPTION_HANDLER_REGISTRATION])
    {
        for (at, _) in source.match_indices(needle) {
            let line = crate::analysis::line_of(source, at);
            if tree.as_ref().is_some_and(|tree| in_text(tree, at))
                || lines.iter().any(|l| l.contains(&line))
            {
                continue;
            }
            found.extend(registration(scope, imports, owner, needle, at));
        }
    }
    found
}

/// The handler that the registration call `needle` at byte `at` passes: a
/// function named there, or the function written inside the call. Middleware
/// counts only with the error-middleware parameter count.
fn registration(
    scope: &Scope<'_>,
    imports: &BTreeMap<usize, Imports>,
    owner: usize,
    needle: &str,
    at: usize,
) -> Option<Handler> {
    let input = &scope.inputs[owner];
    let source = input.source.as_deref().unwrap_or("");
    let line = crate::analysis::line_of(source, at);
    let open = at + needle.len();
    let argument = call_argument(&source[open..])?;
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
    if needle == EXCEPTION_HANDLER_REGISTRATION
        && !(named || argument.contains("=>") || argument.contains("delegate"))
    {
        return None;
    }
    let (owner, name, source, lines) = if named {
        named_handler(scope, imports, owner, argument)?
    } else {
        let first = crate::analysis::line_of(source, open);
        let last = crate::analysis::line_of(source, open + argument.len());
        (
            owner,
            "error handler".into(),
            argument.into(),
            (first, last),
        )
    };
    let middleware = needle == MIDDLEWARE_REGISTRATION;
    (!middleware || parameter_count(&source) == Some(MIDDLEWARE_PARAMETERS)).then(|| Handler {
        owner,
        name,
        source,
        lines,
        helpers: Vec::new(),
        registered,
    })
}

/// Whether byte `at` lies in a comment or a string literal: a registration
/// named in documentation or in a list of patterns registers nothing.
fn in_text(tree: &tree_sitter::Tree, at: usize) -> bool {
    let mut node = tree.root_node().descendant_for_byte_range(at, at + 1);
    while let Some(current) = node {
        let kind = current.kind();
        if crate::analysis::is_comment(current) || kind.contains("string") {
            return true;
        }
        node = current.parent();
    }
    false
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
                helpers: Vec::new(),
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

/// Methods and functions a web framework calls to turn any error a request
/// handler returns into a response: axum's `into_response` on an error type,
/// actix-web's `error_response`, Rocket's `#[catch(…)]` functions and the
/// `catch` method of a NestJS `@Catch(…)` class.
fn implemented(scope: &Scope<'_>, owner: usize) -> Vec<Handler> {
    let input = &scope.inputs[owner];
    let source = input.source.as_deref().unwrap_or("");
    let lines = scope.test_lines(owner);
    let filters = exception_filters(source);
    let csharp = input.result.path.extension().is_some_and(|e| e == "cs");
    scope.units[&owner]
        .units
        .iter()
        .filter(|u| u.callable() && !lines.iter().any(|l| l.contains(&u.line)))
        .filter_map(|unit| {
            let text = unit.source(source);
            let implements = |name: &str| {
                let registration = format!("impl {name} for {}", unit.owner);
                source
                    .contains(&format!("{name} for {} ", unit.owner))
                    .then_some(registration)
            };
            let registration = match unit.short_name.as_str() {
                "into_response" if error_type(&unit.owner) => implements("IntoResponse")?,
                "error_response" if !unit.owner.is_empty() => implements("ResponseError")?,
                "catch" if filters.contains(&unit.owner) => {
                    format!("@Catch(…) class {}", unit.owner)
                }
                name if csharp && !unit.owner.is_empty() => {
                    csharp_handler(source, &unit.owner, name, text)?
                }
                _ => text
                    .lines()
                    .map(str::trim)
                    .find(|l| l.starts_with("#[catch(") || l.starts_with("#[rocket::catch("))?
                    .to_string(),
            };
            Some(Handler {
                owner,
                name: unit.name.clone(),
                source: text.to_string(),
                lines: (unit.line, unit.end_line),
                helpers: Vec::new(),
                registered: format!(
                    "`{registration}` ({}:{})",
                    input.result.path.display(),
                    unit.line
                ),
            })
        })
        .collect()
}

/// How ASP.NET Core calls a C# method for every exception a request throws:
/// the `OnException` of an exception filter, the `TryHandleAsync` of an
/// `IExceptionHandler`, or the `Invoke`/`InvokeAsync` of a middleware class
/// that catches what the rest of the pipeline throws.
fn csharp_handler(source: &str, owner: &str, method: &str, text: &str) -> Option<String> {
    let bases = csharp_bases(source, owner);
    let implements = |base: &str| {
        bases
            .split(|c: char| !(c.is_alphanumeric() || c == '_'))
            .any(|b| b == base)
    };
    if let Some((base, _)) = CSHARP_HANDLER_TYPES
        .iter()
        .find(|(base, handles)| *handles == method && implements(base))
    {
        return Some(format!("class {owner} : {base}"));
    }
    let middleware = matches!(method, "Invoke" | "InvokeAsync")
        && text.contains("HttpContext")
        && text.contains("catch");
    middleware.then(|| format!("middleware class {owner}"))
}

/// The base class and interfaces a C# class declares, as written after `:`.
fn csharp_bases<'a>(source: &'a str, owner: &str) -> &'a str {
    let declaration = format!("class {owner}");
    source
        .match_indices(&declaration)
        .find_map(|(at, _)| {
            let rest = &source[at + declaration.len()..];
            if rest.starts_with(|c: char| c.is_alphanumeric() || c == '_') {
                return None;
            }
            let head = &rest[..rest.find('{').unwrap_or(rest.len())];
            let head = head.split(" where ").next().unwrap_or(head);
            head.find(':').map(|colon| head[colon + 1..].trim())
        })
        .unwrap_or("")
}

/// A type whose name marks it as an error, such as `Error`, `ApiError` or `AuthRejection`.
fn error_type(name: &str) -> bool {
    ["Error", "Exception", "Rejection"]
        .iter()
        .any(|suffix| name.ends_with(suffix))
}

/// Classes declared after a NestJS `@Catch(…)` decorator.
fn exception_filters(source: &str) -> Vec<String> {
    source
        .match_indices("@Catch(")
        .filter_map(|(at, _)| {
            let rest = &source[at..];
            let class = rest.find("class ")? + "class ".len();
            Some(
                rest[class..]
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '$')
                    .collect(),
            )
        })
        .collect()
}

/// How many parameters the first parameter list in `text` declares.
fn parameter_count(text: &str) -> Option<usize> {
    let open = text.find('(')?;
    let mut depth = 0usize;
    let mut count = 0usize;
    let mut empty = true;
    for c in text[open + 1..].chars() {
        match c {
            '(' | '[' | '{' | '<' => depth += 1,
            ')' if depth == 0 => return Some(if empty { 0 } else { count + 1 }),
            ')' | ']' | '}' | '>' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => count += 1,
            c if !c.is_whitespace() => empty = false,
            _ => {}
        }
    }
    None
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
/// through its closing brace, or its indented body in Python. In Rust, each
/// such enum or struct with the attributes above it, which hold the
/// messages `thiserror` writes.
fn error_class_texts<'a>(path: &Path, source: &'a str) -> Vec<&'a str> {
    let python = path.extension().is_some_and(|e| e == "py");
    let rust = path.extension().is_some_and(|e| e == "rs");
    let csharp = path.extension().is_some_and(|e| e == "cs");
    let mut found = Vec::new();
    let mut offset = 0;
    let mut attributes: Option<usize> = None;
    for line in source.split_inclusive('\n') {
        let line_start = offset;
        offset += line.len();
        if rust && line.trim_start().starts_with("#[") {
            attributes.get_or_insert(line_start);
            continue;
        }
        let start = if rust {
            attributes.take().unwrap_or(line_start)
        } else {
            line_start
        };
        let error = declared_class(line, rust, csharp)
            .is_some_and(|name| name.ends_with("Error") || name.ends_with("Exception"));
        if !error {
            continue;
        }
        let end = if python {
            Some(indented_end(&source[start..], line.len()))
        } else if (rust || csharp)
            && let Some(end) = declaration_end(&source[line_start..])
        {
            Some(line_start - start + end)
        } else {
            braced_end(&source[start..])
        };
        if let Some(end) = end {
            found.push(source[start..start + end].trim_end());
        }
    }
    found
}

/// The class a line declares: a JavaScript, TypeScript, Python or C# class,
/// or a Rust enum or struct.
fn declared_class(line: &str, rust: bool, csharp: bool) -> Option<String> {
    let rest = if csharp {
        let mut rest = line.trim_start();
        while let Some(after) = CSHARP_MODIFIERS
            .iter()
            .find_map(|modifier| rest.strip_prefix(modifier))
        {
            rest = after;
        }
        rest.strip_prefix("class ")
            .or_else(|| rest.strip_prefix("record "))?
    } else if rust {
        let visible = line.trim_start().trim_start_matches("pub(crate) ");
        let visible = visible.trim_start_matches("pub ");
        visible
            .strip_prefix("enum ")
            .or_else(|| visible.strip_prefix("struct "))?
    } else {
        line.trim_start_matches("export ")
            .trim_start_matches("default ")
            .trim_start_matches("abstract ")
            .strip_prefix("class ")?
    };
    Some(
        rest.chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '$')
            .collect(),
    )
}

/// Modifiers a C# class declaration can start with.
const CSHARP_MODIFIERS: [&str; 7] = [
    "public ",
    "internal ",
    "private ",
    "protected ",
    "sealed ",
    "abstract ",
    "partial ",
];

/// The end of a Python class: its first line and the indented lines after it.
fn indented_end(text: &str, first: usize) -> usize {
    let mut end = first;
    for next in text[first..].split_inclusive('\n') {
        if !next.trim().is_empty() && !next.starts_with([' ', '\t']) {
            break;
        }
        end += next.len();
    }
    end
}

/// The end of a declaration through the brace that closes its first `{`.
fn braced_end(text: &str) -> Option<usize> {
    let open = text.find('{')?;
    let mut depth = 0usize;
    text[open..].char_indices().find_map(|(i, c)| {
        match c {
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            _ => {}
        }
        (depth == 0).then_some(open + i + 1)
    })
}

/// Where a Rust unit or tuple struct declaration, or a C# class with a
/// primary constructor and no body, ends: at its `;`, when that comes before
/// any `{` outside parentheses. A C# base call can interpolate
/// (`: Exception($"{name} was not found");`).
fn declaration_end(declaration: &str) -> Option<usize> {
    let mut depth = 0usize;
    for (i, c) in declaration.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ';' if depth == 0 => return Some(i + 1),
            '{' if depth == 0 => return None,
            _ => {}
        }
    }
    None
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
