//! The walk of the languages with analyzers of their own: each grammar's
//! definitions, as its node kinds name them, placed as units with the facts
//! every rule reads. The generic tier reads its languages through their tag
//! queries instead (`generic`).
use super::{
    BendNames, Definition, FileUnits, Kind, Role, Unit,
    callbacks::{callback, csharp_callbacks, registered_callbacks},
    facts::Facts,
    import_names::{csharp_import, go_imports, imports, java_import},
    name_of,
    ruby_definitions::{ruby_assignment, ruby_call},
};
use crate::analysis::{bend, text};
use std::collections::BTreeSet;
use tree_sitter::Node;

pub(super) fn walk(node: Node<'_>, source: &str, owner: &str, file: &mut FileUnits) {
    // What a parser could not read holds no definitions to judge.
    if node.is_error() {
        return;
    }
    match node.kind() {
        // Bend 2: `import ./main.bend as Sort` names its module `Sort`.
        "import_declaration" if file.bend.is_some() => {
            if let Some(alias) = node.child_by_field_name("alias") {
                file.imports.insert(text(alias, source).to_string());
            }
        }
        "type_declaration" if file.bend.is_some() => {
            let name = name_of(node, source);
            push(Definition::whole(node), &name, "", Kind::Type, source, file);
        }
        "law_declaration" => {
            let name = name_of(node, source);
            push(Definition::whole(node), &name, "", Kind::Law, source, file);
        }
        "use_declaration" | "import_statement" | "import_from_statement" => {
            imports(node, source, &mut file.imports);
        }
        // Go's import block, or one Java `import a.b.Name;`.
        "import_declaration" => {
            go_imports(node, source, &mut file.imports);
            java_import(node, source, &mut file.imports);
        }
        "using_directive" => csharp_import(node, source, &mut file.imports),
        "namespace_use_declaration" => {
            crate::analysis::php::imports(node, source, &mut file.imports)
        }
        // PHP: `namespace App { … }` holds its declarations in a block.
        "namespace_definition" => {
            if let Some(body) = node.child_by_field_name("body") {
                children(body, source, owner, file);
            }
        }
        // PHP: `return function (App $app) { … };` configures its includer.
        "return_statement" if crate::analysis::php::returned_closure(node).is_some() => {
            if let Some(closure) = crate::analysis::php::returned_closure(node) {
                let definition = Definition {
                    outer: node,
                    node: closure,
                    body: closure.child_by_field_name("body"),
                };
                let name = crate::analysis::php::RETURNED_CLOSURE;
                push(definition, name, owner, Kind::Function, source, file);
            }
        }
        // Go: `func (s *Store) Find(…)` is a method of `Store`; a C# or Java
        // method belongs to the class, struct, record, interface or enum
        // around it.
        "method_declaration" => {
            let receiver = node
                .child_by_field_name("receiver")
                .and_then(|r| r.named_child(0))
                .and_then(|p| p.child_by_field_name("type"))
                .map(|t| base_type(text(t, source).trim_start_matches('*')));
            function(
                node,
                node,
                source,
                receiver.as_deref().unwrap_or(owner),
                file,
            );
        }
        "constructor_declaration"
        | "destructor_declaration"
        | "compact_constructor_declaration" => {
            function(node, node, source, owner, file);
        }
        // A C# property or indexer whose accessors have statement bodies.
        "property_declaration" | "indexer_declaration" => {
            let accessors = node.child_by_field_name("accessors").filter(|list| {
                let mut cursor = list.walk();
                list.named_children(&mut cursor).any(|accessor| {
                    accessor
                        .child_by_field_name("body")
                        .is_some_and(|b| b.kind() == "block")
                })
            });
            if let Some(accessors) = accessors {
                let name = match node.kind() {
                    "indexer_declaration" => "this".to_string(),
                    _ => name_of(node, source),
                };
                let definition = Definition {
                    outer: node,
                    node,
                    body: Some(accessors),
                };
                push(definition, &name, owner, Kind::Method, source, file);
            }
        }
        "namespace_declaration" => {
            if let Some(body) = node.child_by_field_name("body") {
                children(body, source, owner, file);
            }
        }
        // C# top-level statements: minimal API route handlers and middleware
        // written inline, and local functions.
        "global_statement" => {
            let Some(statement) = node.named_child(0) else {
                return;
            };
            if statement.kind() == "local_function_statement" {
                function(node, statement, source, owner, file);
                return;
            }
            let callbacks = csharp_callbacks(statement, source);
            let single = callbacks.len() == 1;
            for (name, lambda) in callbacks {
                let definition = Definition {
                    outer: if single { node } else { lambda },
                    node: lambda,
                    body: lambda.child_by_field_name("body"),
                };
                push(definition, &name, owner, Kind::Function, source, file);
            }
        }
        "type_declaration" => {
            let mut cursor = node.walk();
            let specs: Vec<Node<'_>> = node
                .named_children(&mut cursor)
                .filter(|c| matches!(c.kind(), "type_spec" | "type_alias"))
                .collect();
            let single = specs.len() == 1;
            for spec in specs {
                let definition = Definition {
                    outer: if single { node } else { spec },
                    node: spec,
                    body: None,
                };
                push(
                    definition,
                    &name_of(spec, source),
                    "",
                    Kind::Type,
                    source,
                    file,
                );
            }
        }
        // Ruby: `module Billing` and `class Invoice < Base` own their methods.
        "module" | "class"
            if node
                .child_by_field_name("name")
                .is_some_and(|n| matches!(n.kind(), "constant" | "scope_resolution")) =>
        {
            owning_type(node, &base_type(&name_of(node, source)), source, file);
        }
        // `class << self` holds its owner's singleton methods.
        "singleton_class" => {
            if let Some(body) = node.child_by_field_name("body") {
                children(body, source, owner, file);
            }
        }
        "method" | "singleton_method" => function(node, node, source, owner, file),
        "call" => ruby_call(node, source, owner, file),
        "assignment" => ruby_assignment(node, source, owner, file),
        // Definitions made under a condition, such as `unless method_defined?(:x)`.
        "if" | "unless" | "then" | "else" | "begin" => children(node, source, owner, file),
        "source_file"
        | "program"
        | "module"
        | "declaration_list"
        | "class_body"
        | "export_statement"
        | "statement_block"
        | "compilation_unit"
        | "interface_body"
        | "enum_body"
        | "enum_body_declarations" => children(node, source, owner, file),
        // A Java enum constant with its own body, such as a state machine's
        // `Data { void read(…) { … } }`: its methods belong to the constant.
        "enum_constant" => {
            if let Some(body) = node.child_by_field_name("body") {
                children(body, source, &name_of(node, source), file);
            }
        }
        // `export default { async fetch(request, env) { … } }`, as Cloudflare Workers write it.
        "object"
            if node
                .parent()
                .is_some_and(|p| p.kind() == "export_statement") =>
        {
            children(node, source, owner, file)
        }
        "expression_statement" => {
            if let Some((object, name, function)) = assigned_function(node, source) {
                let definition = Definition {
                    outer: node,
                    node: function,
                    body: function.child_by_field_name("body"),
                };
                push(definition, name, object, Kind::Method, source, file);
                return;
            }
            let callbacks = if node
                .named_child(0)
                .is_some_and(crate::analysis::php::registers)
            {
                crate::analysis::php::registered_callbacks(node, source)
            } else {
                registered_callbacks(node, source)
            };
            let single = callbacks.len() == 1;
            for (name, function) in callbacks {
                let definition = Definition {
                    outer: if single { node } else { function },
                    node: function,
                    body: function.child_by_field_name("body"),
                };
                push(definition, &name, owner, Kind::Function, source, file);
            }
        }
        "block"
            if node
                .parent()
                .is_some_and(|p| p.kind() == "class_definition") =>
        {
            children(node, source, owner, file)
        }
        "impl_item" => {
            let name = node
                .child_by_field_name("type")
                .map(|n| base_type(text(n, source)))
                .unwrap_or_default();
            if let Some(body) = node.child_by_field_name("body") {
                children(body, source, &name, file);
            }
        }
        "mod_item" => {
            if let Some(body) = node.child_by_field_name("body") {
                children(body, source, owner, file);
            }
        }
        // A C# or PHP interface or enum is one type: its members have no
        // bodies to judge. A Java interface's default methods and a Java
        // enum's methods have them, so those are read like classes below.
        "interface_declaration" | "enum_declaration"
            if node.child_by_field_name("body").is_some_and(|b| {
                matches!(
                    b.kind(),
                    "declaration_list" | "enum_member_declaration_list" | "enum_declaration_list"
                )
            }) =>
        {
            let name = name_of(node, source);
            if !name.is_empty() {
                push(Definition::whole(node), &name, "", Kind::Type, source, file);
            }
        }
        "class_declaration"
        | "class_definition"
        | "class"
        | "abstract_class_declaration"
        | "struct_declaration"
        | "record_declaration"
        | "trait_declaration"
        | "interface_declaration"
        | "enum_declaration" => {
            owning_type(node, &name_of(node, source), source, file);
        }
        "decorated_definition" => {
            if let Some(definition) = node.child_by_field_name("definition") {
                if definition.kind() == "class_definition" {
                    walk(definition, source, owner, file);
                } else if definition.kind() == "function_definition" {
                    function(node, definition, source, owner, file);
                }
            }
        }
        "function_item"
        | "function_definition"
        | "function_declaration"
        | "generator_function_declaration"
        | "method_definition" => {
            function(node, node, source, owner, file);
        }
        "lexical_declaration" | "variable_declaration" => {
            let mut cursor = node.walk();
            for declarator in node.named_children(&mut cursor) {
                if declarator.kind() != "variable_declarator" {
                    continue;
                }
                let Some(value) = declarator.child_by_field_name("value") else {
                    continue;
                };
                // `const f = () => …`, or a callback registered through a call such as
                // `const view = database.view(options, (ctx) => …)` or `memo(forwardRef(…))`.
                let name = declarator
                    .child_by_field_name("name")
                    .map(|n| text(n, source).to_string())
                    .unwrap_or_default();
                let Some(function) = callback(value, 2) else {
                    // `export const actions = { default: async (event) => … }`, as
                    // SvelteKit form actions and handler maps write it.
                    if let Some(object) = object_literal(value) {
                        object_functions(object, source, &name, file);
                    }
                    continue;
                };
                let definition = Definition {
                    outer: node,
                    node: value,
                    body: function.child_by_field_name("body"),
                };
                push(definition, &name, owner, Kind::Function, source, file);
            }
        }
        "struct_item"
        | "enum_item"
        | "trait_item"
        | "type_item"
        | "union_item"
        | "type_alias_declaration"
        | "delegate_declaration"
        | "annotation_type_declaration" => {
            let name = name_of(node, source);
            if !name.is_empty() {
                push(Definition::whole(node), &name, "", Kind::Type, source, file);
            }
        }
        _ => {}
    }
}

/// A function a module assigns to an object's property, as CommonJS modules
/// define their API: `res.status = function status(code) { … }` is `status`
/// of `res`. The object is named by its last part (`app.response` is
/// `response`); `module.exports = function …` is named by the function.
fn assigned_function<'t>(
    statement: Node<'t>,
    source: &'t str,
) -> Option<(&'t str, &'t str, Node<'t>)> {
    let assignment = statement
        .named_child(0)
        .filter(|n| n.kind() == "assignment_expression")?;
    let (left, right) = (
        assignment.child_by_field_name("left")?,
        assignment.child_by_field_name("right")?,
    );
    if !matches!(
        right.kind(),
        "function_expression" | "function" | "arrow_function"
    ) || left.kind() != "member_expression"
    {
        return None;
    }
    let object = left.child_by_field_name("object")?;
    let property = text(left.child_by_field_name("property")?, source);
    // `Router.prototype.handle` is `handle` of `Router`.
    let owner = match object.kind() {
        "member_expression" => {
            let last = text(object.child_by_field_name("property")?, source);
            match object.child_by_field_name("object") {
                Some(inner) if last == "prototype" => text(inner, source),
                _ => last,
            }
        }
        _ => text(object, source),
    };
    if owner == "module" && property == "exports" || owner == "exports" && property == "default" {
        let name = right.child_by_field_name("name").map(|n| text(n, source))?;
        return Some(("", name, right));
    }
    Some((owner, property, right))
}

/// The object literal a declaration's value is, through TypeScript's
/// `satisfies` and `as` and parentheses.
fn object_literal(value: Node<'_>) -> Option<Node<'_>> {
    match value.kind() {
        "object" => Some(value),
        "satisfies_expression" | "as_expression" | "parenthesized_expression" => {
            object_literal(value.named_child(0)?)
        }
        _ => None,
    }
}

/// The functions an object literal named `owner` holds as properties or
/// methods, each a method of `owner`.
fn object_functions(object: Node<'_>, source: &str, owner: &str, file: &mut FileUnits) {
    let mut cursor = object.walk();
    for property in object.named_children(&mut cursor) {
        match property.kind() {
            "method_definition" => function(property, property, source, owner, file),
            "pair" => {
                let (Some(key), Some(value)) = (
                    property.child_by_field_name("key"),
                    property.child_by_field_name("value"),
                ) else {
                    continue;
                };
                if !matches!(
                    value.kind(),
                    "arrow_function" | "function_expression" | "function"
                ) {
                    continue;
                }
                let definition = Definition {
                    outer: property,
                    node: value,
                    body: value.child_by_field_name("body"),
                };
                let key = text(key, source).trim_matches(['"', '\'', '`']);
                push(definition, key, owner, Kind::Method, source, file);
            }
            _ => {}
        }
    }
}

/// A type named `name` whose body holds its members: each member is a unit
/// the type owns, and a type without any is one unit itself. Members left
/// out over syntax errors are still its members.
fn owning_type(node: Node<'_>, name: &str, source: &str, file: &mut FileUnits) {
    let before = (file.units.len(), file.left_out.len());
    if let Some(body) = node.child_by_field_name("body") {
        children(body, source, name, file);
    }
    if (file.units.len(), file.left_out.len()) == before && !name.is_empty() {
        push(Definition::whole(node), name, "", Kind::Type, source, file);
    }
}

pub(super) fn children(node: Node<'_>, source: &str, owner: &str, file: &mut FileUnits) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        walk(child, source, owner, file);
    }
}

fn function(outer: Node<'_>, node: Node<'_>, source: &str, owner: &str, file: &mut FileUnits) {
    let name = name_of(node, source);
    let kind = if owner.is_empty() {
        Kind::Function
    } else {
        Kind::Method
    };
    let definition = Definition {
        outer,
        node,
        body: node.child_by_field_name("body"),
    };
    push(definition, &name, owner, kind, source, file);
}

/// `Store<T>` and `&mut Store` both name `Store`.
fn base_type(text: &str) -> String {
    text.trim_start_matches(['&', ' '])
        .trim_start_matches("mut ")
        .split(['<', ' '])
        .next()
        .unwrap_or("")
        .rsplit("::")
        .next()
        .unwrap_or("")
        .to_string()
}

pub(super) fn push(
    definition: Definition<'_>,
    short_name: &str,
    owner: &str,
    kind: Kind,
    source: &str,
    file: &mut FileUnits,
) {
    if short_name.is_empty() {
        return;
    }
    let Some(placed) = file.place(definition, (short_name, owner), kind, source) else {
        return;
    };
    let Definition { node, body, .. } = definition;
    let (facts, refs) = references(node, (short_name, owner), kind, &file.imports, source);
    let equality = equality_override(node, short_name, source);
    let literals = body
        .filter(|_| !equality && !crate::analysis::literals::returns_constant(node))
        .map_or_else(Vec::new, |b| crate::analysis::literals::in_node(b, source));
    let (role, effects, joins_text) = bend_facts(node, short_name, file, source);
    let unit = Unit {
        nesting: body.map_or(0, |b| crate::analysis::nesting::control(b).0),
        branch_chain: body.map_or(0, |b| crate::analysis::nesting::control(b).1),
        blocks: body.map_or_else(Vec::new, |b| crate::analysis::blocks::blocks(b, source)),
        literals,
        sites: body.map_or_else(Vec::new, |b| {
            crate::analysis::sites::in_node(b, source, file.django)
        }),
        errors: body.map_or_else(Vec::new, |b| {
            crate::analysis::errors::created_errors(b, source)
        }),
        calls: facts.calls,
        passed: facts.paths,
        equality,
        routes: crate::analysis::routes::spring(node, source),
        refs,
        role,
        effects,
        joins_text,
        statement: bend::statement(node, source),
        mentions: facts.idents,
        ..placed
    };
    file.units.push(unit);
}

/// What a definition calls and mentions, and the names it references: its
/// parameters and return type count, not only its body, as do the file's
/// imports its code names and its own and its owner's names. A type's calls
/// and mentions are left out.
fn references(
    node: Node<'_>,
    (short_name, owner): (&str, &str),
    kind: Kind,
    imports: &BTreeSet<String>,
    source: &str,
) -> (Facts, BTreeSet<String>) {
    let mut facts = Facts::default();
    facts.visit(node, source);
    if kind == Kind::Type {
        facts.calls.clear();
    }
    let mut refs = std::mem::take(&mut facts.refs);
    refs.extend(facts.idents.intersection(imports).cloned());
    if kind == Kind::Type {
        facts.idents.clear();
    }
    refs.insert(short_name.to_string());
    if !owner.is_empty() {
        refs.insert(owner.to_string());
    }
    (facts, refs)
}

/// A Bend 2 def's role, whether it performs effects and whether it joins
/// text; code that does neither outside Bend 2.
fn bend_facts(
    node: Node<'_>,
    short_name: &str,
    file: &FileUnits,
    source: &str,
) -> (Role, bool, bool) {
    match &file.bend {
        Some(names) if node.kind() == "function_definition" => (
            bend_role(node, names, source),
            bend::effectful(node, source) || names.effects.iter().any(|n| n == short_name),
            bend::joins_text(node, source),
        ),
        _ => (Role::Code, false, false),
    }
}

fn bend_role(definition: Node<'_>, names: &BendNames, source: &str) -> Role {
    if names.proofs || bend::proof(definition, &names.claims, &names.aliases, source) {
        Role::Proof
    } else if bend::type_level(definition) {
        Role::TypeLevel
    } else {
        Role::Code
    }
}

/// A Java method that overrides `Object.equals` or `Object.hashCode`.
fn equality_override(node: Node<'_>, name: &str, source: &str) -> bool {
    node.kind() == "method_declaration"
        && node.child_by_field_name("parameters").is_some_and(|p| {
            let parameters = text(p, source);
            match name {
                "equals" => p.named_child_count() == 1 && parameters.contains("Object"),
                "hashCode" => p.named_child_count() == 0,
                _ => false,
            }
        })
}
