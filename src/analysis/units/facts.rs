//! The names a unit calls, references and mentions, read from its body.
use crate::analysis::{call_name, callee_name, is_comment, macro_calls, text};
use std::collections::BTreeSet;
use tree_sitter::Node;

#[derive(Default)]
pub(super) struct Facts {
    pub(super) calls: BTreeSet<String>,
    /// Functions named by a path where it is not called (`parse`).
    pub(super) paths: BTreeSet<String>,
    pub(super) refs: BTreeSet<String>,
    pub(super) idents: BTreeSet<String>,
}

impl Facts {
    pub(super) fn visit(&mut self, node: Node<'_>, source: &str) {
        if is_comment(node) {
            return;
        }
        match node.kind() {
            "call_expression" | "call" | "invocation_expression" => {
                if let Some(name) = call_name(node, source) {
                    self.calls.insert(name);
                }
            }
            "scoped_identifier" => {
                self.paths.extend(passed_path(node, source));
            }
            // Java: `repository.findById(id)`.
            "method_invocation" => {
                if let Some(name) = node.child_by_field_name("name") {
                    self.calls.insert(text(name, source).to_string());
                }
            }
            "new_expression" | "object_creation_expression" => {
                // PHP names the class without a field: `new \App\Cursor($db)`.
                if let Some(name) = node
                    .child_by_field_name("constructor")
                    .or_else(|| node.child_by_field_name("type"))
                    .and_then(|c| callee_name(c, source))
                    .or_else(|| crate::analysis::php::callee_name(node, source))
                {
                    self.refs.insert(name.clone());
                    self.calls.insert(name);
                }
            }
            // C# names types and members with plain identifiers.
            "identifier" if csharp_reference(node) => {
                let name = text(node, source).to_string();
                self.refs.insert(name.clone());
                self.idents.insert(name);
            }
            kind if crate::analysis::php::CALLS.contains(&kind) => {
                self.calls
                    .extend(crate::analysis::php::callee_name(node, source));
            }
            // PHP: a declared type such as `Request $request` or `: ?User`.
            "named_type" => {
                let name = text(node, source).rsplit('\\').next().unwrap_or("");
                self.refs.insert(name.trim_start_matches('?').to_string());
            }
            "jsx_opening_element" | "jsx_self_closing_element" => {
                if let Some(name) = node.child_by_field_name("name") {
                    self.calls.insert(text(name, source).to_string());
                }
            }
            "token_tree"
                if node
                    .parent()
                    .is_some_and(|p| p.kind() == "macro_invocation") =>
            {
                macro_calls(node, source, &mut self.calls);
            }
            // Ruby constants name classes and modules; instance variables are fields.
            "type_identifier"
            | "field_identifier"
            | "property_identifier"
            | "constant"
            | "instance_variable" => {
                self.refs.insert(text(node, source).to_string());
            }
            "identifier" => {
                self.idents.insert(text(node, source).to_string());
            }
            _ => {}
        }
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            self.visit(child, source);
        }
    }
}

/// The function a Rust path names where the path is a value, not the callee
/// of a call: `unconfirmed_units` in
/// `follow_ups(plan, files, compose::unconfirmed_units)` or `helper` in
/// `.map(Self::helper)`. The name starts in lower case, since Rust names
/// types, variants and constants in upper case (`Outcome::Clear`); a path
/// inside a longer path or a `use` is not one.
fn passed_path(node: Node<'_>, source: &str) -> Option<String> {
    let parent = node.parent()?;
    let callee = |n: Node<'_>| {
        n.parent().is_some_and(|p| {
            p.kind() == "call_expression" && p.child_by_field_name("function") == Some(n)
        })
    };
    if callee(node)
        || parent.kind() == "generic_function" && callee(parent)
        || matches!(
            parent.kind(),
            "scoped_identifier"
                | "use_declaration"
                | "use_list"
                | "scoped_use_list"
                | "use_as_clause"
        )
    {
        return None;
    }
    let name = text(node.child_by_field_name("name")?, source);
    name.starts_with(|c: char| c.is_ascii_lowercase() || c == '_')
        .then(|| name.to_string())
}

/// A C# identifier that names a type or a member rather than a local: a
/// member access (`_repository.ListAsync`), a type argument, base type,
/// declared type or pattern type.
fn csharp_reference(node: Node<'_>) -> bool {
    let Some(parent) = node.parent() else {
        return false;
    };
    match parent.kind() {
        "member_access_expression" | "member_binding_expression" => {
            parent.child_by_field_name("name") == Some(node)
        }
        "generic_name"
        | "type_argument_list"
        | "base_list"
        | "nullable_type"
        | "typeof_expression"
        | "declaration_pattern"
        | "catch_declaration"
        | "qualified_name" => true,
        // Rust, Go and TypeScript array types name their element `element`
        // or nothing; a length there is a value, not a type.
        "array_type"
        | "variable_declaration"
        | "parameter"
        | "property_declaration"
        | "cast_expression" => parent.child_by_field_name("type") == Some(node),
        "method_declaration" | "local_function_statement" => {
            parent.child_by_field_name("returns") == Some(node)
                || parent.child_by_field_name("type") == Some(node)
        }
        _ => false,
    }
}
