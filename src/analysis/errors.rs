//! Errors a function creates and their message arguments: evidence of what
//! error messages say. Jev judges where their text comes from.
use super::{is_comment, sites::clip, text};
use tree_sitter::Node;

/// An error a function creates: its class and its message argument.
#[derive(Clone, Debug, PartialEq)]
pub struct CreatedError {
    pub error: String,
    pub message: String,
}

/// At most this many created errors are listed per function.
pub const MAX_ERRORS: usize = 12;

/// Errors a body creates, in source order: JavaScript and TypeScript
/// `new …Error(…)` or `new …Exception(…)` and any call or `new` a `throw`
/// statement makes, the call a Python `raise` makes, Go's `errors.New`
/// and `fmt.Errorf`, C# and PHP `new …Exception(…)` or any object a `throw`
/// creates, a Ruby `raise`, and Java's `new …Exception(…)` or `new …Error(…)`
/// and any `new` or call a `throw` makes. The message is the
/// first argument, or a Python `detail`, `message` or `msg` keyword. They
/// are evidence of what the messages say; Jev judges where their text comes from.
pub fn created_errors(body: Node<'_>, source: &str) -> Vec<CreatedError> {
    let mut found = Vec::new();
    errors_in(body, source, &mut found);
    found.truncate(MAX_ERRORS);
    found
}

fn errors_in(node: Node<'_>, source: &str, found: &mut Vec<CreatedError>) {
    if is_comment(node) {
        return;
    }
    let created = match node.kind() {
        "new_expression" => node.child_by_field_name("constructor").filter(|c| {
            let name = text(*c, source).rsplit('.').next().unwrap_or("");
            name.ends_with("Error")
                || name.ends_with("Exception")
                || node.parent().is_some_and(|p| p.kind() == "throw_statement")
        }),
        "call_expression" if node.parent().is_some_and(|p| p.kind() == "throw_statement") => {
            node.child_by_field_name("function")
        }
        "call" if node.parent().is_some_and(|p| p.kind() == "raise_statement") => {
            node.child_by_field_name("function")
        }
        // C# and Java: `new OrderNotFoundException(…)`; PHP: `new \App\NotFound(…)`;
        // or any object a `throw` creates.
        "object_creation_expression" => node
            .child_by_field_name("type")
            .or_else(|| super::php::callee(node))
            .filter(|t| {
                let name = super::callee_name(*t, source).unwrap_or_else(|| {
                    text(*t, source)
                        .rsplit('\\')
                        .next()
                        .unwrap_or("")
                        .to_string()
                });
                name.ends_with("Exception")
                    || name.ends_with("Error")
                    || node
                        .parent()
                        .is_some_and(|p| matches!(p.kind(), "throw_statement" | "throw_expression"))
            }),
        // Java: `throw fail("…")`.
        "method_invocation" if node.parent().is_some_and(|p| p.kind() == "throw_statement") => {
            node.child_by_field_name("name")
        }
        // Go: `errors.New("…")` and `fmt.Errorf("…", err)`.
        "call_expression" => node
            .child_by_field_name("function")
            .filter(|f| matches!(text(*f, source), "errors.New" | "fmt.Errorf")),
        // Ruby: `raise NotFound, "…"`, `raise NotFound.new("…")` or `raise "…"`.
        "call"
            if matches!(super::ruby::method(node, source), "raise" | "fail")
                && node.child_by_field_name("receiver").is_none() =>
        {
            found.extend(
                node.child_by_field_name("arguments")
                    .and_then(|arguments| ruby_raise(arguments, source)),
            );
            None
        }
        _ => None,
    };
    if let Some(callee) = created {
        let message = super::php::arguments(node)
            .and_then(|arguments| message_argument(arguments, source))
            .unwrap_or_default();
        found.push(CreatedError {
            error: clip(text(callee, source)),
            message,
        });
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        errors_in(child, source, found);
    }
}

/// A Ruby `raise`: the error class and message it passes, or the message
/// alone for `raise "…"` (a `RuntimeError`).
fn ruby_raise(arguments: Node<'_>, source: &str) -> Option<CreatedError> {
    let mut cursor = arguments.walk();
    let all: Vec<Node<'_>> = arguments
        .named_children(&mut cursor)
        .filter(|a| !is_comment(*a))
        .collect();
    let (error, rest) = all.split_first()?;
    if rest.is_empty() && error.kind() == "string" {
        return Some(CreatedError {
            error: "RuntimeError".into(),
            message: clip(text(*error, source)),
        });
    }
    let built = (error.kind() == "call" && super::ruby::method(*error, source) == "new")
        .then(|| error.child_by_field_name("receiver"))
        .flatten();
    let message = match (built, rest.first()) {
        (_, Some(message)) => clip(text(*message, source)),
        (Some(_), None) => error
            .child_by_field_name("arguments")
            .and_then(|a| message_argument(a, source))
            .unwrap_or_default(),
        (None, None) => String::new(),
    };
    Some(CreatedError {
        error: clip(text(built.unwrap_or(*error), source)),
        message,
    })
}

/// The first argument, or a `detail`, `message` or `msg` keyword argument.
fn message_argument(arguments: Node<'_>, source: &str) -> Option<String> {
    let mut cursor = arguments.walk();
    let all: Vec<Node<'_>> = arguments
        .named_children(&mut cursor)
        .filter(|a| !is_comment(*a))
        .collect();
    let keyword = all.iter().find(|a| {
        a.kind() == "keyword_argument"
            && a.child_by_field_name("name")
                .is_some_and(|n| matches!(text(n, source), "detail" | "message" | "msg"))
    });
    keyword
        .or_else(|| all.iter().find(|a| a.kind() != "keyword_argument"))
        .map(|a| clip(text(*a, source)))
}

#[cfg(test)]
mod tests {
    use crate::analysis::units::parse;
    use std::path::Path;

    #[test]
    fn created_errors_list_each_message_argument() {
        let errors = |path: &str, source: &str| {
            parse(Path::new(path), source).unwrap().units[0]
                .errors
                .iter()
                .map(|e| (e.error.clone(), e.message.clone()))
                .collect::<Vec<_>>()
        };
        let typescript = errors(
            "a.ts",
            "async function load(id: string) {\n  const { data, error } = await db.from('t').select().eq('id', id)\n  if (error) throw new InternalError(`Query failed: ${error.message}`, error)\n  if (!data) return next(new NotFoundError('Missing'))\n  if (id === '') throw httpError(400)\n  return new Map()\n}\n",
        );
        assert_eq!(
            typescript,
            [
                (
                    "InternalError".into(),
                    "`Query failed: ${error.message}`".into()
                ),
                ("NotFoundError".into(), "'Missing'".into()),
                ("httpError".into(), "400".into()),
            ]
        );
        let python = errors(
            "a.py",
            "def load(id):\n    if not id:\n        raise HTTPException(status_code=400, detail=f'bad {id}')\n    raise ValueError('missing')\n",
        );
        assert_eq!(
            python,
            [
                ("HTTPException".into(), "detail=f'bad {id}'".into()),
                ("ValueError".into(), "'missing'".into()),
            ]
        );
    }
}
