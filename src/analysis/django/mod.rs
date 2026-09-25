//! Django conventions read from syntax: which Python files are Django code,
//! the URL routes that reach its views, settings modules (whose top-level
//! assignments configure the deployed site) and the secrets they hold, which
//! are shown redacted, the lines elsewhere that select a settings module, and
//! the templates views render.
mod routes;
mod secrets;
mod selection;
mod settings;
mod templates;

pub use routes::*;
pub use secrets::*;
pub use selection::*;
pub use settings::*;
pub use templates::*;

use super::text;
use std::path::Path;
use tree_sitter::Node;

/// Packages whose import marks a Python file as Django code.
const DJANGO_PACKAGES: [&str; 2] = ["django", "rest_framework"];

/// Whether a Python file imports Django or Django REST framework, at the top
/// level or inside a top-level `try` or `if` block.
pub fn imports_django(path: &Path, root: Node<'_>, source: &str) -> bool {
    if path.extension().is_none_or(|e| e != "py") {
        return false;
    }
    let mut cursor = root.walk();
    root.named_children(&mut cursor)
        .any(|node| imports_package(node, source))
}

fn imports_package(node: Node<'_>, source: &str) -> bool {
    match node.kind() {
        "import_statement" | "import_from_statement" => {
            let module = if node.kind() == "import_from_statement" {
                node.child_by_field_name("module_name")
            } else {
                node.named_child(0)
            };
            module.is_some_and(|m| {
                let name = text(m, source);
                let package = name.split(['.', ' ']).next().unwrap_or(name);
                DJANGO_PACKAGES.contains(&package)
            })
        }
        "if_statement" | "try_statement" | "block" | "else_clause" | "elif_clause"
        | "except_clause" | "finally_clause" => {
            let mut cursor = node.walk();
            node.named_children(&mut cursor)
                .any(|c| imports_package(c, source))
        }
        _ => false,
    }
}

/// The name a module is imported by: its file stem, or its package's name
/// for an `__init__.py`.
fn module_name(path: &Path) -> &str {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    if stem == "__init__" {
        path.parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("")
    } else {
        stem
    }
}

/// The command name of a Django management command module
/// (`app/management/commands/<name>.py`), which a person runs by hand with
/// `manage.py <name>`.
pub fn management_command(path: &Path) -> Option<&str> {
    let parts: Vec<&str> = path.iter().filter_map(|p| p.to_str()).collect();
    let [.., management, commands, _] = parts.as_slice() else {
        return None;
    };
    let name = path.file_stem()?.to_str()?;
    (*management == "management" && *commands == "commands" && !name.starts_with('_'))
        .then_some(name)
}

#[cfg(test)]
mod tests;
