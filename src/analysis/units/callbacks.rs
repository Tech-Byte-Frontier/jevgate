//! Functions registered through a call instead of declared by name: route
//! handlers and middleware passed as arguments in JavaScript, TypeScript and
//! C#, and the function a declaration binds (`const view = memo(() => …)`).
use crate::analysis::{callee_name, text};
use tree_sitter::Node;

/// Calls that declare tests or module mocks rather than register
/// application callbacks, by callee or method name.
const TEST_CALLS: [&str; 16] = [
    "describe",
    "it",
    "test",
    "suite",
    "bench",
    "context",
    "specify",
    "beforeEach",
    "afterEach",
    "beforeAll",
    "afterAll",
    "fixture",
    "vi",
    "jest",
    "mock",
    "doMock",
];

/// Functions a module-level statement registers through a call, such as the
/// route handler in `app.post('/pages', validator(…), async (c) => …)`, with a
/// name for each from its registration: `app.post('/pages')`. Each call of a
/// chain (`router.get(…).post(…)`) registers its last function argument.
/// Test declarations (`describe`, `it`, `test`) are left to the test rules.
pub(super) fn registered_callbacks<'t>(
    statement: Node<'t>,
    source: &str,
) -> Vec<(String, Node<'t>)> {
    let is_function = |n: &Node<'_>| {
        matches!(
            n.kind(),
            "arrow_function" | "function_expression" | "function"
        )
    };
    let mut found = Vec::new();
    let mut call = statement
        .named_child(0)
        .filter(|e| e.kind() == "call_expression");
    while let Some(current) = call {
        let function = current.child_by_field_name("function");
        let root = function.map(|f| chain_root(f, source)).unwrap_or_default();
        let method = function
            .and_then(|f| f.child_by_field_name("property"))
            .map(|p| text(p, source));
        let arguments: Vec<Node<'t>> = current
            .child_by_field_name("arguments")
            .map(|a| a.named_children(&mut a.walk()).collect())
            .unwrap_or_default();
        let test =
            TEST_CALLS.contains(&root.as_str()) || method.is_some_and(|m| TEST_CALLS.contains(&m));
        if let Some(handler) = arguments.iter().rev().find(|n| is_function(n))
            && !test
        {
            let path = arguments
                .first()
                .filter(|a| matches!(a.kind(), "string" | "template_string"))
                .map(|a| text(*a, source))
                .unwrap_or("…");
            let name = match method {
                Some(method) => format!("{root}.{method}({path})"),
                None => format!("{root}({path})"),
            };
            found.push((name, *handler));
        }
        call = function
            .filter(|f| f.kind() == "member_expression")
            .and_then(|f| f.child_by_field_name("object"))
            .filter(|o| o.kind() == "call_expression");
    }
    found.reverse();
    found
}

/// Methods that register an ASP.NET Core request handler or middleware
/// written inline: minimal API routes (`app.MapGet("/orders", …)`) and
/// `app.Use(…)` or `app.Run(…)`. Other top-level calls that take a lambda,
/// such as `builder.Services.AddCors(o => …)`, configure the program and
/// stay in its setup.
fn csharp_registration(method: &str) -> bool {
    matches!(method, "Use" | "Run")
        || method
            .strip_prefix("Map")
            .is_some_and(|rest| rest.is_empty() || rest.starts_with(char::is_uppercase))
}

/// Lambdas a C# top-level statement registers as request handlers, named by
/// their registration: `app.MapPost("/orders")`. Each call of a chain
/// (`app.MapGet(…).RequireAuthorization()`) is read.
pub(super) fn csharp_callbacks<'t>(statement: Node<'t>, source: &str) -> Vec<(String, Node<'t>)> {
    let mut found = Vec::new();
    let mut call = statement
        .named_child(0)
        .map(|e| {
            if e.kind() == "await_expression" {
                e.named_child(0).unwrap_or(e)
            } else {
                e
            }
        })
        .filter(|e| e.kind() == "invocation_expression");
    while let Some(current) = call {
        let function = current
            .child_by_field_name("function")
            .filter(|f| f.kind() == "member_access_expression");
        let method = function
            .and_then(|f| f.child_by_field_name("name"))
            .and_then(|n| callee_name(n, source))
            .unwrap_or_default();
        let arguments: Vec<Node<'t>> = current
            .child_by_field_name("arguments")
            .map(|a| {
                a.named_children(&mut a.walk())
                    .filter_map(|argument| {
                        argument.named_child(argument.named_child_count().saturating_sub(1) as u32)
                    })
                    .collect()
            })
            .unwrap_or_default();
        let handler = arguments.iter().rev().find(|n| {
            matches!(
                n.kind(),
                "lambda_expression" | "anonymous_method_expression"
            )
        });
        if let Some(handler) = handler
            && csharp_registration(&method)
        {
            let root = function
                .map(|f| csharp_chain_root(f, source))
                .unwrap_or_default();
            let path = arguments
                .first()
                .filter(|a| a.kind().contains("string"))
                .map(|a| text(*a, source))
                .unwrap_or("…");
            found.push((format!("{root}.{method}({path})"), *handler));
        }
        call = function
            .and_then(|f| f.child_by_field_name("expression"))
            .filter(|o| o.kind() == "invocation_expression");
    }
    found.reverse();
    found
}

/// The leftmost name of a C# callee such as `app.MapGet` or `app.MapGroup("/x").MapGet`.
fn csharp_chain_root(callee: Node<'_>, source: &str) -> String {
    let mut node = callee;
    loop {
        let next = match node.kind() {
            "member_access_expression" => node.child_by_field_name("expression"),
            "invocation_expression" => node.child_by_field_name("function"),
            _ => None,
        };
        match next {
            Some(inner) => node = inner,
            None => return text(node, source).to_string(),
        }
    }
}

/// The leftmost name of a callee such as `app.get` or `router.route('/x').get`.
fn chain_root(callee: Node<'_>, source: &str) -> String {
    let mut node = callee;
    loop {
        let next = match node.kind() {
            "member_expression" => node.child_by_field_name("object"),
            "call_expression" => node.child_by_field_name("function"),
            _ => None,
        };
        match next {
            Some(inner) => node = inner,
            None => return text(node, source).to_string(),
        }
    }
}

/// The function a declaration defines: the value itself, or the last function
/// argument of a call (as in `useCallback(fn, deps)`), looking through up to
/// `depth` nested calls, parentheses and type assertions such as
/// `(async () => …) satisfies GetStaticPaths`.
pub(super) fn callback(value: Node<'_>, depth: usize) -> Option<Node<'_>> {
    match value.kind() {
        "arrow_function" | "function_expression" | "function" => Some(value),
        "parenthesized_expression"
        | "satisfies_expression"
        | "as_expression"
        | "non_null_expression" => callback(value.named_child(0)?, depth),
        "call_expression" if depth > 0 => {
            let arguments = value.child_by_field_name("arguments")?;
            let mut cursor = arguments.walk();
            let arguments: Vec<Node<'_>> = arguments.named_children(&mut cursor).collect();
            let function = |n: &&Node<'_>| {
                matches!(
                    n.kind(),
                    "arrow_function" | "function_expression" | "function"
                )
            };
            match arguments.iter().rev().find(function) {
                Some(found) => Some(*found),
                None => arguments
                    .iter()
                    .rev()
                    .filter(|n| n.kind() == "call_expression")
                    .find_map(|n| callback(*n, depth - 1)),
            }
        }
        _ => None,
    }
}
