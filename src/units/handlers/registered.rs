//! Handlers found where the program registers them: a function passed to a
//! registration call (`.onError(…)`, `.setErrorHandler(…)`, Express error
//! middleware, `UseExceptionHandler`, PHP's `set_exception_handler`), a
//! Python function under an error-handler decorator, and the views a Django
//! URLconf or Django REST framework's `EXCEPTION_HANDLER` names.
use super::{Handler, Scope, named_handler};
use crate::analysis::imports::Links;

/// Calls that register a web framework's error handler, by the method that
/// takes it; the handler is the function named or written in the call.
/// PHP's `set_exception_handler` takes a closure or a function's name; it
/// is the only registration looked for in PHP files.
const HANDLER_REGISTRATIONS: [&str; 3] = [".onError(", ".setErrorHandler(", PHP_REGISTRATION];
const PHP_REGISTRATION: &str = "set_exception_handler(";
/// Express registers a function of four parameters (`err, req, res, next`)
/// passed to `.use(…)` as its error handler.
const MIDDLEWARE_REGISTRATION: &str = ".use(";
const MIDDLEWARE_PARAMETERS: usize = 4;
/// ASP.NET Core's exception handler middleware runs the lambda it is given
/// for every unhandled exception; given a path (`"/Error"`), it re-executes
/// a page instead, which is judged as the page's own code.
const EXCEPTION_HANDLER_REGISTRATION: &str = ".UseExceptionHandler(";
/// Python decorators that register the function below them as an error handler.
const HANDLER_DECORATORS: [&str; 2] = [".exception_handler(", ".errorhandler("];

/// Handlers one file passes to a registration call: a function named there,
/// or the function written inside the call.
pub(super) fn registered(scope: &Scope<'_>, links: &Links, owner: usize) -> Vec<Handler> {
    let input = &scope.inputs[owner];
    let source = input.source.as_deref().unwrap_or("");
    let lines = scope.test_lines(owner);
    let tree = crate::syntax::parse(&input.result.path, source)
        .ok()
        .flatten();
    let mut found = Vec::new();
    let php = crate::analysis::php::file(&input.result.path);
    for needle in HANDLER_REGISTRATIONS
        .into_iter()
        .chain([MIDDLEWARE_REGISTRATION, EXCEPTION_HANDLER_REGISTRATION])
        .filter(|needle| php == (*needle == PHP_REGISTRATION))
    {
        for (at, _) in source.match_indices(needle) {
            let line = crate::analysis::line_of(source, at);
            if tree.as_ref().is_some_and(|tree| in_text(tree, at))
                || lines.iter().any(|l| l.contains(&line))
            {
                continue;
            }
            found.extend(registration(scope, links, owner, needle, at));
        }
    }
    found
}

/// The handler that the registration call `needle` at byte `at` passes: a
/// function named there, or the function written inside the call. Middleware
/// counts only with the error-middleware parameter count.
fn registration(
    scope: &Scope<'_>,
    links: &Links,
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
    let quoted = argument.trim_matches(['\'', '"']);
    let named = quoted
        .chars()
        .all(|c| c.is_alphanumeric() || c == '_' || c == '$');
    if needle == EXCEPTION_HANDLER_REGISTRATION
        && !(named || argument.contains("=>") || argument.contains("delegate"))
    {
        return None;
    }
    let (owner, name, source, lines) = if named {
        named_handler(scope, links, owner, quoted)?
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
pub(super) fn decorated(scope: &Scope<'_>, owner: usize) -> Vec<Handler> {
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

/// Module-level names a Django URLconf assigns its error views to.
const DJANGO_ERROR_VIEWS: [&str; 4] = ["handler400", "handler403", "handler404", "handler500"];
/// The Django REST framework setting that names its exception handler.
const DRF_EXCEPTION_HANDLER: &str = "EXCEPTION_HANDLER";

/// Views a Django URLconf names for errors (`handler500 = views.server_error`
/// or a dotted path in a string), and the function Django REST framework's
/// `EXCEPTION_HANDLER` setting names, found by their last name segment.
pub(super) fn django_views(scope: &Scope<'_>, links: &Links, owner: usize) -> Vec<Handler> {
    let input = &scope.inputs[owner];
    if input.result.path.extension().is_none_or(|e| e != "py") {
        return Vec::new();
    }
    let source = input.source.as_deref().unwrap_or("");
    let tests = scope.test_lines(owner);
    let mut offset = 0;
    let mut found = Vec::new();
    for line in source.split_inclusive('\n') {
        let at = offset;
        offset += line.len();
        if line.trim_start().starts_with('#')
            || tests
                .iter()
                .any(|l| l.contains(&crate::analysis::line_of(source, at)))
        {
            continue;
        }
        let named = DJANGO_ERROR_VIEWS
            .iter()
            .find_map(|view| {
                let value = line.strip_prefix(view)?.trim_start().strip_prefix('=')?;
                Some(value.trim())
            })
            .or_else(|| {
                let key = line.find(DRF_EXCEPTION_HANDLER)?;
                let rest = line[key + DRF_EXCEPTION_HANDLER.len()..]
                    .trim_start_matches(['"', '\''])
                    .trim_start();
                Some(rest.strip_prefix(':')?.trim())
            });
        let Some(value) = named else {
            continue;
        };
        let name = value
            .trim_end_matches(',')
            .trim_matches(['"', '\''])
            .rsplit('.')
            .next()
            .unwrap_or_default();
        if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
            continue;
        }
        let Some((handler_owner, handler_name, handler_source, lines)) =
            named_handler(scope, links, owner, name)
        else {
            continue;
        };
        let line_number = crate::analysis::line_of(source, at);
        found.push(Handler {
            owner: handler_owner,
            name: handler_name,
            source: handler_source,
            lines,
            helpers: Vec::new(),
            registered: format!(
                "`{}` ({}:{line_number})",
                line.trim(),
                input.result.path.display()
            ),
        });
    }
    found
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
