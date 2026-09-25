//! The names each language's imports bring into a file.
use crate::analysis::text;
use std::collections::BTreeSet;
use tree_sitter::Node;

/// Go imports: each package's name, its alias or the last path segment.
pub(super) fn go_imports(node: Node<'_>, source: &str, names: &mut BTreeSet<String>) {
    if node.kind() == "import_spec" {
        let name = node
            .child_by_field_name("name")
            .map(|n| text(n, source).to_string())
            .or_else(|| {
                let path = text(node.child_by_field_name("path")?, source);
                Some(
                    path.trim_matches(['"', '`'])
                        .rsplit('/')
                        .next()?
                        .to_string(),
                )
            });
        names.extend(name.filter(|n| !matches!(n.as_str(), "_" | ".")));
        return;
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        go_imports(child, source, names);
    }
}

/// A C# `using` directive: the alias it declares, or the last segment of
/// the namespace or type it imports.
pub(super) fn csharp_import(node: Node<'_>, source: &str, names: &mut BTreeSet<String>) {
    let name = node
        .child_by_field_name("name")
        .or_else(|| {
            let mut cursor = node.walk();
            node.named_children(&mut cursor)
                .find(|c| matches!(c.kind(), "qualified_name" | "identifier"))
        })
        .map(|n| {
            if n.kind() == "qualified_name" {
                n.child_by_field_name("name").unwrap_or(n)
            } else {
                n
            }
        })
        .map(|n| text(n, source).to_string());
    names.extend(name.filter(|n| !n.is_empty()));
}

/// A Java import's last name: the class, or the member of a static import.
/// A wildcard import names no one class.
pub(super) fn java_import(node: Node<'_>, source: &str, names: &mut BTreeSet<String>) {
    let mut cursor = node.walk();
    let parts: Vec<Node<'_>> = node.named_children(&mut cursor).collect();
    if parts.iter().any(|p| p.kind() == "asterisk") {
        return;
    }
    let name = parts.iter().find_map(|p| match p.kind() {
        "scoped_identifier" => p.child_by_field_name("name"),
        "identifier" => Some(*p),
        _ => None,
    });
    names.extend(name.map(|n| text(n, source).to_string()));
}

pub(super) fn imports(node: Node<'_>, source: &str, names: &mut BTreeSet<String>) {
    if matches!(node.kind(), "identifier" | "type_identifier") {
        let name = text(node, source);
        if !matches!(name, "self" | "super" | "crate") {
            names.insert(name.to_string());
        }
        return;
    }
    if node.kind() == "scoped_identifier" {
        if let Some(name) = node.child_by_field_name("name") {
            imports(name, source, names);
        }
        return;
    }
    if node.kind() == "string" {
        return;
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        imports(child, source, names);
    }
}
