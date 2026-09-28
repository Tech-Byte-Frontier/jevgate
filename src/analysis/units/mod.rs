//! Functions, methods and types of one file, with the local facts used for
//! grouping, callee and subject lookup, and marking bodies too small to judge.
mod callbacks;
mod facts;
mod generic;
mod import_names;
mod left_out;
mod ruby_definitions;
mod unit;

use super::{bend, is_comment, line_of, summary, text};
use anyhow::Result;
use callbacks::{callback, csharp_callbacks, registered_callbacks};
use facts::Facts;
use import_names::{csharp_import, go_imports, imports, java_import};
pub use left_out::LeftOut;
use ruby_definitions::{ruby_assignment, ruby_call};
use std::{collections::BTreeSet, ops::Range, path::Path};
use tree_sitter::Node;
pub use unit::{Kind, MIN_BODY_LINES, Role, Unit};

#[derive(Clone, Debug, Default)]
pub struct FileUnits {
    pub units: Vec<Unit>,
    pub imports: BTreeSet<String>,
    /// Top-level constants and bindings whose values hold literals.
    pub constants: Vec<super::literals::Constant>,
    /// Top-level statements that run when the module loads.
    pub setup: super::sites::Setup,
    /// False when no parser supports this language.
    pub parsed: bool,
    /// Read by the generic tier (`analysis::generic`): only function
    /// simplification, file organization, shared logic and comments judge it.
    pub generic: bool,
    /// Whether it is Django code: Python that imports Django or Django REST
    /// framework, or a Django settings module.
    pub django: bool,
    /// In Django code, the URL routes it declares.
    pub routes: Vec<super::django::Route>,
    /// In Django code, its upper-case module constants with their
    /// assignments as shown.
    pub module_constants: Vec<(String, String)>,
    /// In a server template, the code it runs while rendering that reads
    /// client data (`template_code`).
    pub template_code: super::sites::Setup,
    /// In Bend 2 code, the names that tell its proofs apart.
    bend: Option<BendNames>,
    /// Units whose syntax holds an error, left out of every rule, in source
    /// order (`left_out`).
    pub left_out: Vec<LeftOut>,
    /// Syntax errors outside every unit left out, as `syntax::error_regions`.
    errors: Vec<Range<usize>>,
}

/// A Bend 2 file's claims (laws a proof must hold), the laws that give a
/// def an `IO` type (`law main: IO(Unit)` above `def main():`) and import
/// aliases, and whether it is a file of proofs (`bend::proof_file`), to
/// tell its proofs from its code and its effects from pure code.
#[derive(Clone, Debug, Default)]
struct BendNames {
    claims: Vec<String>,
    effects: Vec<String>,
    aliases: Vec<String>,
    proofs: bool,
}

/// Units of a supported language. Unsupported languages return an unparsed,
/// empty result. A unit holding a syntax error is left out and recorded
/// (`left_out`); a file the parser could not read is an error, never an
/// empty clear file.
pub fn parse(path: &Path, source: &str) -> Result<FileUnits> {
    let Some(tree) = crate::syntax::parse(path, source)? else {
        return Ok(FileUnits::default());
    };
    let root = tree.root_node();
    let mut file = match super::generic::read(path, source) {
        Some(language) => tagged(language, root, source),
        None => walked(path, root, source),
    };
    calls_by_name(&mut file.units);
    if path.extension().is_none_or(|e| e != "rs") {
        for unit in &mut file.units {
            unit.passed.clear();
        }
    }
    Ok(file)
}

/// A file of the generic tier (`analysis::generic`): the definitions its
/// tag query captures, and the syntax errors outside them.
fn tagged(language: &super::generic::Language, root: Node<'_>, source: &str) -> FileUnits {
    let mut file = FileUnits {
        parsed: true,
        generic: true,
        ..Default::default()
    };
    generic::walk(language, root, source, &mut file);
    file.record_errors(root);
    file
}

/// A file of a language with an analyzer of its own: the units its walk
/// finds, then what the file holds outside them (`module_code`).
fn walked(path: &Path, root: Node<'_>, source: &str) -> FileUnits {
    let settings = super::django::settings_module(path, root, source);
    let mut file = FileUnits {
        parsed: true,
        django: settings || super::django::imports_django(path, root, source),
        bend: bend::file(path).then(|| bend_names(path, root, source)),
        ..Default::default()
    };
    walk(root, source, "", &mut file);
    file.record_errors(root);
    if let Some(names) = &file.bend {
        unaliased_calls(&mut file.units, &names.aliases);
    }
    module_code(path, (root, source), settings, &mut file);
    file
}

/// What a file holds outside its units: a Django module's routes and
/// constants, its module constants but those on lines syntax errors left
/// out, the statements that run outside every unit (a Django settings
/// module's with `settings`), and a server template's code.
fn module_code(
    path: &Path,
    (root, source): (Node<'_>, &str),
    settings: bool,
    file: &mut FileUnits,
) {
    if file.django {
        file.routes = super::django::routes(root, source);
        if !settings {
            file.module_constants = super::django::module_constants(root, source);
        }
    }
    file.constants = super::literals::constants(root, source);
    file.leave_out_constants(source);
    let spans: Vec<Range<usize>> = file.units.iter().map(|u| u.span.clone()).collect();
    file.setup = setup_of(path, root, source, &spans, settings);
    if crate::components::server_template(path) {
        file.template_code = super::template_code::template_code(path, source);
    }
}

/// The statements that run outside every unit: a framework configuration's
/// objects, a PHP page script, a server template's inline scripts, or a
/// module's setup (a Django settings module's with `settings`).
fn setup_of(
    path: &Path,
    root: Node<'_>,
    source: &str,
    spans: &[Range<usize>],
    settings: bool,
) -> super::sites::Setup {
    if framework_config(path) {
        super::sites::config_setup(root, source)
    } else if super::php::file(path) {
        super::sites::script(root, source, spans)
    } else if crate::components::server_template(path) {
        super::sites::inline_script(root, source, spans)
    } else {
        super::sites::setup(root, source, spans, settings)
    }
}

/// A Bend 2 file's claims and aliases. A law is a claim here when its
/// statement holds an equality or asks for a witness, or applies a def of
/// this file that computes a type, such as `Sorted(sort(xs))`.
fn bend_names(path: &Path, root: Node<'_>, source: &str) -> BendNames {
    let mut cursor = root.walk();
    let top: Vec<Node<'_>> = root.named_children(&mut cursor).collect();
    let propositions: BTreeSet<String> = top
        .iter()
        .filter(|n| n.kind() == "function_definition" && bend::type_level(**n))
        .map(|n| name_of(*n, source))
        .collect();
    let claims = top
        .iter()
        .filter(|n| bend::statement(**n, source).is_some_and(|s| s.claim(&propositions)))
        .map(|n| name_of(*n, source))
        .collect();
    let effects = top
        .iter()
        .filter(|n| bend::statement(**n, source).is_some_and(|s| s.head.as_deref() == Some("IO")))
        .map(|n| name_of(*n, source))
        .collect();
    BendNames {
        claims,
        effects,
        aliases: bend::aliases(root, source),
        proofs: bend::proof_file(path),
    }
}

/// A Bend 2 call through an import alias, `Sort.sort(xs)`, also calls the
/// def `sort` that the aliased file declares.
fn unaliased_calls(units: &mut [Unit], aliases: &[String]) {
    for unit in units {
        let unaliased: Vec<String> = unit
            .calls
            .iter()
            .filter_map(|call| {
                let (alias, name) = call.split_once('.')?;
                aliases.iter().any(|a| a == alias).then(|| name.to_string())
            })
            .collect();
        unit.calls.extend(unaliased);
    }
}

/// A function passed by name, such as `map(parse)`, is used like a call.
fn calls_by_name(units: &mut [Unit]) {
    let names: BTreeSet<String> = units.iter().map(|u| u.short_name.clone()).collect();
    for unit in units {
        let used: Vec<String> = unit
            .mentions
            .intersection(&names)
            .filter(|name| **name != unit.short_name)
            .cloned()
            .collect();
        unit.calls.extend(used);
        unit.mentions.clear();
    }
}

/// A framework configuration file whose settings are plain objects, such
/// as `next.config.mjs`.
fn framework_config(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|name| name.starts_with("next.config."))
}

fn walk(node: Node<'_>, source: &str, owner: &str, file: &mut FileUnits) {
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
        "namespace_use_declaration" => super::php::imports(node, source, &mut file.imports),
        // PHP: `namespace App { … }` holds its declarations in a block.
        "namespace_definition" => {
            if let Some(body) = node.child_by_field_name("body") {
                children(body, source, owner, file);
            }
        }
        // PHP: `return function (App $app) { … };` configures its includer.
        "return_statement" if super::php::returned_closure(node).is_some() => {
            if let Some(closure) = super::php::returned_closure(node) {
                let definition = Definition {
                    outer: node,
                    node: closure,
                    body: closure.child_by_field_name("body"),
                };
                let name = super::php::RETURNED_CLOSURE;
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
            let callbacks = if node.named_child(0).is_some_and(super::php::registers) {
                super::php::registered_callbacks(node, source)
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

fn children(node: Node<'_>, source: &str, owner: &str, file: &mut FileUnits) {
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

fn name_of(node: Node<'_>, source: &str) -> String {
    node.child_by_field_name("name")
        .map(|n| text(n, source).to_string())
        .unwrap_or_default()
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

/// The syntax of one definition: the outer node that carries its documentation
/// (a declaration or decorator), the defining node, and its body if any.
#[derive(Clone, Copy)]
struct Definition<'t> {
    outer: Node<'t>,
    node: Node<'t>,
    body: Option<Node<'t>>,
}

impl<'t> Definition<'t> {
    /// A definition without a separate body, such as a type.
    fn whole(node: Node<'t>) -> Self {
        Self {
            outer: node,
            node,
            body: None,
        }
    }
}

fn push(
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
        .filter(|_| !equality && !super::literals::returns_constant(node))
        .map_or_else(Vec::new, |b| super::literals::in_node(b, source));
    let (role, effects, joins_text) = bend_facts(node, short_name, file, source);
    let unit = Unit {
        nesting: body.map_or(0, |b| super::nesting::control(b).0),
        branch_chain: body.map_or(0, |b| super::nesting::control(b).1),
        blocks: body.map_or_else(Vec::new, |b| super::blocks::blocks(b, source)),
        literals,
        sites: body.map_or_else(Vec::new, |b| super::sites::in_node(b, source, file.django)),
        errors: body.map_or_else(Vec::new, |b| super::errors::created_errors(b, source)),
        calls: facts.calls,
        passed: facts.paths,
        equality,
        routes: super::routes::spring(node, source),
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

impl Unit {
    /// A unit placed in its file, before any fact about its code: its names
    /// and kind, its span with the documentation, comments and attributes
    /// above it, its lines, body, header and documentation line, and the
    /// size of its body.
    fn placed(
        definition: Definition<'_>,
        (short_name, owner): (&str, &str),
        kind: Kind,
        source: &str,
    ) -> Self {
        let Definition { outer, body, .. } = definition;
        let start = leading_start(outer);
        Self {
            name: if owner.is_empty() {
                short_name.to_string()
            } else {
                format!("{owner}::{short_name}")
            },
            short_name: short_name.to_string(),
            owner: owner.to_string(),
            kind,
            span: start..outer.end_byte(),
            line: line_of(source, outer.start_byte()),
            end_line: line_of(
                source,
                outer.end_byte().saturating_sub(1).max(outer.start_byte()),
            ),
            body: body.map(|b| b.byte_range()),
            signature: summary::signature(outer, body, source),
            doc: summary::doc_line(&source[start..outer.start_byte()], outer, source),
            body_lines: body.map_or(0, |b| body_lines(text(b, source))),
            nesting: 0,
            branch_chain: 0,
            blocks: Vec::new(),
            literals: Vec::new(),
            sites: Vec::new(),
            errors: Vec::new(),
            calls: BTreeSet::new(),
            passed: BTreeSet::new(),
            equality: false,
            routes: Vec::new(),
            refs: BTreeSet::new(),
            role: Role::Code,
            effects: false,
            joins_text: false,
            statement: None,
            mentions: BTreeSet::new(),
        }
    }
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

/// Whether a declaration is marked deprecated in the documentation and
/// attributes above it or in its header up to its body: a `@deprecated`
/// docblock tag, Java annotation or Python decorator, Rust's
/// `#[deprecated]`, C#'s `[Obsolete]`, or a comment line opening with
/// `Deprecated:` (Go, TomDoc). Without a body only what lies above it
/// counts: sqlmodel's module root, read whole, marked every class of a file
/// with one `@deprecated` method.
pub(crate) fn deprecated(node: Node<'_>, source: &str) -> bool {
    let end = node
        .child_by_field_name("definition")
        .unwrap_or(node)
        .child_by_field_name("body")
        .map_or(node.start_byte(), |body| body.start_byte());
    let header = source
        .get(leading_start(node)..end)
        .unwrap_or("")
        .to_ascii_lowercase();
    header.lines().map(str::trim_start).any(|line| {
        word(line, "@deprecated")
            || line.starts_with('@') && word(line, ".deprecated")
            || line.contains("#[deprecated")
            || line.contains("[obsolete")
            // A comment, not a parameter named `deprecated`.
            || line.strip_prefix(['/', '#', '*']).is_some_and(|comment| {
                comment
                    .trim_start_matches(['/', '*', '!'])
                    .trim_start()
                    .starts_with("deprecated:")
            })
    })
}

/// Whether `line` holds `name` as a whole word: `@deprecated`, not a mark
/// named `@deprecated_lifespan`.
fn word(line: &str, name: &str) -> bool {
    line.match_indices(name).any(|(at, _)| {
        !line[at + name.len()..].starts_with(|c: char| c.is_alphanumeric() || c == '_')
    })
}

/// Include documentation, comments and attributes directly above the definition.
fn leading_start(node: Node<'_>) -> usize {
    let mut start = node.start_byte();
    let mut previous = node.prev_named_sibling();
    // Ruby: comments above a class's first statement precede its body.
    if previous.is_none()
        && let Some(parent) = node.parent().filter(|p| p.kind() == "body_statement")
    {
        previous = parent.prev_named_sibling();
    }
    while let Some(sibling) = previous {
        if !(is_comment(sibling)
            || sibling.kind() == "attribute_item"
            || sibling.kind() == "decorator")
        {
            break;
        }
        start = sibling.start_byte();
        previous = sibling.prev_named_sibling();
    }
    start
}

/// Non-blank lines that are more than an opening or closing brace.
pub(crate) fn body_lines(body: &str) -> usize {
    body.lines()
        .map(str::trim)
        .filter(|line| {
            !line.is_empty()
                && !line
                    .chars()
                    .all(|c| matches!(c, '{' | '}' | '(' | ')' | ';' | ','))
        })
        .count()
}

#[cfg(test)]
mod tests;
