//! URLconf entries and the views they route requests to.
use super::{module_name, secrets::literal_content};
use crate::analysis::text;
use std::path::Path;
use tree_sitter::Node;

/// A URLconf entry that routes requests to a view, such as
/// `path('tasks/<int:task_id>/', views.task_edit)`: it shows that the view
/// receives outside requests and what its URL parameters may hold.
#[derive(Clone, Debug, PartialEq)]
pub struct Route {
    /// The view's name: a function, or the class of a class-based view or
    /// Django REST framework viewset.
    pub view: String,
    /// The module part before the name, such as `views` in `views.index`.
    pub module: Option<String>,
    pub line: usize,
    /// The whole entry on one line.
    pub text: String,
}

/// Calls that route URLs to views: `path`, `re_path`, `url`, and a router's
/// `register`.
const ROUTE_CALLS: [&str; 4] = ["path", "re_path", "url", "register"];

/// The URL routes a Django file declares, wherever the calls are.
pub fn routes(root: Node<'_>, source: &str) -> Vec<Route> {
    let mut found = Vec::new();
    collect_routes(root, source, &mut found);
    found
}

fn collect_routes(node: Node<'_>, source: &str, found: &mut Vec<Route>) {
    if node.kind() == "call"
        && let Some(route) = route(node, source)
    {
        found.push(route);
        return;
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_routes(child, source, found);
    }
}

/// A call to a route function whose first argument is a string pattern and
/// whose next one names a view.
fn route(call: Node<'_>, source: &str) -> Option<Route> {
    let function = crate::analysis::callee_name(call.child_by_field_name("function")?, source)?;
    if !ROUTE_CALLS.contains(&function.as_str()) {
        return None;
    }
    let arguments = call.child_by_field_name("arguments")?;
    let mut cursor = arguments.walk();
    let positional: Vec<Node<'_>> = arguments
        .named_children(&mut cursor)
        .filter(|a| a.kind() != "keyword_argument" && a.kind() != "comment")
        .collect();
    if positional.first()?.kind() != "string" {
        return None;
    }
    let (module, view) = view_name(*positional.get(1)?, source)?;
    Some(Route {
        view,
        module,
        line: crate::analysis::line_of(source, call.start_byte()),
        text: crate::analysis::sites::clip(text(call, source)),
    })
}

/// The view an argument names: `views.index`, `index`,
/// `views.TaskView.as_view()`, or the dotted path `'app.views.index'`.
fn view_name(argument: Node<'_>, source: &str) -> Option<(Option<String>, String)> {
    let dotted = match argument.kind() {
        "identifier" | "attribute" => text(argument, source).to_string(),
        "call" => {
            let function = argument.child_by_field_name("function")?;
            let object = function.child_by_field_name("object")?;
            if function.kind() != "attribute"
                || text(function.child_by_field_name("attribute")?, source) != "as_view"
            {
                return None;
            }
            text(object, source).to_string()
        }
        "string" => literal_content(argument).map(|r| source[r].to_string())?,
        _ => return None,
    };
    let mut parts = dotted.rsplit('.');
    let view = parts.next()?.trim().to_string();
    let module = parts.next().map(|m| m.trim().to_string());
    (!view.is_empty() && view.chars().all(|c| c.is_alphanumeric() || c == '_'))
        .then_some((module, view))
}

/// Methods Django and Django REST framework call on a class-based view or
/// viewset to answer a request.
const VIEW_METHODS: [&str; 16] = [
    "get",
    "post",
    "put",
    "patch",
    "delete",
    "dispatch",
    "list",
    "create",
    "retrieve",
    "update",
    "partial_update",
    "destroy",
    "form_valid",
    "get_queryset",
    "get_object",
    "get_context_data",
];

/// Whether a route reaches a unit of the module at `path`: a function of
/// that name, or a request method of a class of that name. A route that
/// names a module (`views.index`) must name this one.
pub fn routes_to(route: &Route, path: &Path, short_name: &str, owner: &str) -> bool {
    let name = if owner.is_empty() {
        short_name
    } else if VIEW_METHODS.contains(&short_name) {
        owner
    } else {
        return false;
    };
    route.view == name
        && route
            .module
            .as_deref()
            .is_none_or(|m| m == module_name(path))
}
