//! Settings modules: which modules are settings, which settings modules
//! extend others, and the settings their statements assign.
use super::{module_name, redacted, secret_literals, secret_name};
use crate::analysis::text;
use std::{ops::Range, path::Path};
use tree_sitter::Node;

/// Settings only a Django settings module assigns; one of them marks a
/// module as settings wherever it lives.
const PROJECT_SETTINGS: [&str; 4] = [
    "INSTALLED_APPS",
    "ROOT_URLCONF",
    "MIDDLEWARE",
    "MIDDLEWARE_CLASSES",
];

/// Settings that decide how the deployed site protects itself; in a module
/// under a `settings` directory or named `settings.py`, one of them marks it
/// as settings, such as a `dev.py` that only turns `DEBUG` on.
const SECURITY_SETTINGS: [&str; 12] = [
    "DEBUG",
    "SECRET_KEY",
    "ALLOWED_HOSTS",
    "PASSWORD_HASHERS",
    "SESSION_",
    "CSRF_",
    "SECURE_",
    "CORS_",
    "X_FRAME_OPTIONS",
    "REST_FRAMEWORK",
    "JWT_",
    "SIMPLE_JWT",
];

/// Whether a setting name decides security: the offered sites put them first.
pub fn security_setting(name: &str) -> bool {
    SECURITY_SETTINGS.iter().any(|s| {
        if s.ends_with('_') {
            name.starts_with(s)
        } else {
            name == *s
        }
    })
}

/// Whether a Python module is a Django settings module: it assigns a
/// project setting such as `INSTALLED_APPS`, or it is named `settings.py` or
/// sits in a `settings` directory and assigns a security setting or a
/// secret, such as `JWT_AUTH` or `GOOGLE_OAUTH2_CLIENT_SECRET`.
pub fn settings_module(path: &Path, root: Node<'_>, source: &str) -> bool {
    if path.extension().is_none_or(|e| e != "py") {
        return false;
    }
    let names = assigned_settings(root, source);
    if names.iter().any(|n| PROJECT_SETTINGS.contains(&n.as_str())) {
        return true;
    }
    let named = path.file_stem().is_some_and(|s| s == "settings")
        || path
            .parent()
            .is_some_and(|p| p.components().any(|c| c.as_os_str() == "settings"))
        || star_import(root, source);
    named && names.iter().any(|n| security_setting(n) || secret_name(n))
}

/// Whether the module imports every name of another (`from .base import *`),
/// as settings for one environment extend the shared ones.
fn star_import(root: Node<'_>, source: &str) -> bool {
    let mut cursor = root.walk();
    root.named_children(&mut cursor).any(|node| {
        node.kind() == "import_from_statement" && text(node, source).trim_end().ends_with('*')
    })
}

/// Whether a module's source imports every name of the module at `target`
/// (`from .base import *`, `from config.settings.cors import *`), named by
/// its last dotted part: settings for one environment extend shared ones.
pub fn extends(source: &str, target: &Path) -> bool {
    let name = module_name(target);
    !name.is_empty()
        && source.lines().any(|line| {
            let code = line.split('#').next().unwrap_or("").trim();
            let Some(module) = code
                .strip_prefix("from ")
                .and_then(|rest| rest.strip_suffix("*"))
                .and_then(|rest| rest.trim_end().strip_suffix("import"))
            else {
                return false;
            };
            module.trim().rsplit('.').next() == Some(name)
        })
}

/// Upper-case module constants of a Django file, such as
/// `FIXTURE_DIR = Path(settings.BASE_DIR) / "fixtures"`, each with its
/// assignment as shown (secret literals redacted): a function that builds a
/// path or query from one is shown what it holds.
pub fn module_constants(root: Node<'_>, source: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    let mut redactions = Vec::new();
    let mut cursor = root.walk();
    for node in root.named_children(&mut cursor) {
        if node.kind() == "expression_statement" {
            secret_literals(node, source, &mut redactions);
            collect_settings(node, source, &mut found);
        }
    }
    found
        .into_iter()
        .map(|(name, range)| {
            (
                name,
                crate::analysis::sites::clip(&redacted(source, range, &redactions)),
            )
        })
        .collect()
}

/// Upper-case names assigned at the top level, including inside `if` and
/// `try` blocks, as settings modules choose values per environment.
fn assigned_settings(root: Node<'_>, source: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut cursor = root.walk();
    for node in root.named_children(&mut cursor) {
        collect_settings(node, source, &mut found);
    }
    found.into_iter().map(|(name, _)| name).collect()
}

/// Settings a top-level statement assigns, directly or inside its blocks,
/// each with the byte range of its assignment statement.
pub fn collect_settings(node: Node<'_>, source: &str, found: &mut Vec<(String, Range<usize>)>) {
    match node.kind() {
        "expression_statement" => {
            if let Some(name) = setting_assigned(node, source) {
                found.push((name.to_string(), node.byte_range()));
            }
        }
        "if_statement" | "try_statement" | "block" | "else_clause" | "elif_clause"
        | "except_clause" | "finally_clause" | "with_statement" => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                collect_settings(child, source, found);
            }
        }
        _ => {}
    }
}

/// The setting an expression statement assigns, such as `DEBUG` in
/// `DEBUG = True` or `SECRET_KEY: str = …`.
pub fn setting_assigned<'s>(statement: Node<'_>, source: &'s str) -> Option<&'s str> {
    let assignment = statement.named_child(0)?;
    if !matches!(assignment.kind(), "assignment" | "augmented_assignment") {
        return None;
    }
    let left = assignment.child_by_field_name("left")?;
    let name = text(left, source);
    (left.kind() == "identifier" && setting_name(name)).then_some(name)
}

/// Whether an expression statement changes part of a setting, such as
/// `CACHES["default"]["OPTIONS"]["ssl_cert_reqs"] = None`.
pub fn setting_changed(statement: Node<'_>, source: &str) -> bool {
    let Some(assignment) = statement
        .named_child(0)
        .filter(|a| a.kind() == "assignment")
    else {
        return false;
    };
    let mut target = assignment.child_by_field_name("left");
    while let Some(node) = target {
        match node.kind() {
            "subscript" => target = node.child_by_field_name("value"),
            "attribute" => target = node.child_by_field_name("object"),
            "identifier" => {
                return node != assignment.child_by_field_name("left").unwrap()
                    && setting_name(text(node, source));
            }
            _ => return false,
        }
    }
    false
}

/// An upper-case name such as `SESSION_COOKIE_SECURE`.
pub fn setting_name(name: &str) -> bool {
    name.chars().any(|c| c.is_ascii_uppercase())
        && name
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}
