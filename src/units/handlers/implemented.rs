//! Handlers a web framework calls by what the program implements: a method
//! of an error type, a catcher, an exception filter or middleware class.
use super::{Handler, Scope};
use crate::analysis::units::Unit;
use std::path::Path;

/// ASP.NET Core interfaces and base classes whose method handles every
/// exception a controller or request throws, by the method they define.
const CSHARP_HANDLER_TYPES: [(&str, &str); 5] = [
    ("IExceptionFilter", "OnException"),
    ("IAsyncExceptionFilter", "OnExceptionAsync"),
    ("ExceptionFilterAttribute", "OnException"),
    ("ExceptionFilterAttribute", "OnExceptionAsync"),
    ("IExceptionHandler", "TryHandleAsync"),
];

/// Methods and functions a web framework calls to turn any error a request
/// handler returns into a response, each with how the framework reaches it.
pub(super) fn implemented(scope: &Scope<'_>, owner: usize) -> Vec<Handler> {
    let input = &scope.inputs[owner];
    let source = input.source.as_deref().unwrap_or("");
    let lines = scope.test_lines(owner);
    let file = File {
        path: &input.result.path,
        source,
        filters: exception_filters(source),
    };
    scope.units[&owner]
        .units
        .iter()
        .filter(|u| u.callable() && !lines.iter().any(|l| l.contains(&u.line)))
        .filter_map(|unit| {
            let registration = framework_call(&file, unit)?;
            Some(Handler {
                owner,
                name: unit.name.clone(),
                source: unit.source(source).to_string(),
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

/// The file a handler is looked for in, with its NestJS exception filters.
struct File<'a> {
    path: &'a Path,
    source: &'a str,
    filters: Vec<String>,
}

/// How the framework calls `unit` for every error, when it does: axum's
/// `into_response` on an error type, actix-web's `error_response`, Rocket's
/// `#[catch(…)]` functions, the `catch` method of a NestJS `@Catch(…)`
/// class, the `respond`, `render` or `register` method of a PHP class
/// extending an `…ErrorHandler` or `…ExceptionHandler` (Slim, Laravel), an
/// ASP.NET Core handler method, or a Django middleware's `process_exception`.
fn framework_call(file: &File<'_>, unit: &Unit) -> Option<String> {
    let (path, source) = (file.path, file.source);
    let csharp = path.extension().is_some_and(|e| e == "cs");
    let python = path.extension().is_some_and(|e| e == "py");
    let text = unit.source(source);
    let implements = |name: &str| {
        let registration = format!("impl {name} for {}", unit.owner);
        source
            .contains(&format!("{name} for {} ", unit.owner))
            .then_some(registration)
    };
    Some(match unit.short_name.as_str() {
        "into_response" if error_type(&unit.owner) => implements("IntoResponse")?,
        "error_response" if !unit.owner.is_empty() => implements("ResponseError")?,
        "catch" if file.filters.contains(&unit.owner) => {
            format!("@Catch(…) class {}", unit.owner)
        }
        "respond" | "render" | "register"
            if !unit.owner.is_empty() && crate::analysis::php::file(path) =>
        {
            php_error_handler(source, &unit.owner)?
        }
        name if csharp && !unit.owner.is_empty() => {
            csharp_handler(source, &unit.owner, name, text)?
        }
        // Django calls a middleware's `process_exception` for every
        // exception a view raises; a response it returns is sent.
        "process_exception" if python && !unit.owner.is_empty() => {
            format!("Django middleware {}.process_exception", unit.owner)
        }
        _ => text
            .lines()
            .map(str::trim)
            .find(|l| l.starts_with("#[catch(") || l.starts_with("#[rocket::catch("))?
            .to_string(),
    })
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

/// `class Owner extends Base` when `Base` is a PHP framework's error
/// handler: Slim's `ErrorHandler` (whose `respond` writes the response) or
/// Laravel's `ExceptionHandler` (`render`, and `register` for renderables).
fn php_error_handler(source: &str, owner: &str) -> Option<String> {
    let declaration = format!("class {owner} extends ");
    let at = source.find(&declaration)? + declaration.len();
    let base: String = source[at..]
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '\\')
        .collect();
    let name = base.rsplit('\\').next().unwrap_or("");
    (name.ends_with("ErrorHandler") || name.ends_with("ExceptionHandler"))
        .then(|| format!("{declaration}{base}"))
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
