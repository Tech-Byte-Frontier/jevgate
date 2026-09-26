//! Functions, methods and types of one file, with the local facts used for
//! grouping, callee and subject lookup, and marking bodies too small to judge.
mod callbacks;
mod facts;
mod import_names;
mod ruby_definitions;

use super::{is_comment, line_of, summary, text};
use anyhow::Result;
use callbacks::{callback, csharp_callbacks, registered_callbacks};
use facts::Facts;
use import_names::{csharp_import, go_imports, imports, java_import};
use ruby_definitions::{ruby_assignment, ruby_call};
use std::{collections::BTreeSet, ops::Range, path::Path};
use tree_sitter::Node;

/// Bodies with fewer non-brace lines are too small to judge. They are never clear.
pub const MIN_BODY_LINES: usize = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Function,
    Method,
    Type,
}

#[derive(Clone, Debug)]
pub struct Unit {
    /// `Owner::name` for methods; the bare name otherwise.
    pub name: String,
    pub short_name: String,
    pub owner: String,
    pub kind: Kind,
    /// Definition including leading documentation, comments and attributes.
    pub span: Range<usize>,
    pub line: usize,
    pub end_line: usize,
    pub body: Option<Range<usize>>,
    pub signature: String,
    pub doc: String,
    pub body_lines: usize,
    /// Deepest nesting of control flow in the body, and the longest chain of
    /// `else if`, `elif` or nested conditional-expression branches.
    pub nesting: usize,
    pub branch_chain: usize,
    /// Top-level statement blocks of the body, as byte ranges; empty when
    /// there is no choice of block to extract.
    pub blocks: Vec<Range<usize>>,
    /// Eligible literal values in the body, for hardcoded-value questions.
    pub literals: Vec<super::literals::Literal>,
    /// Calls, built text and field assignments, for locating security findings.
    pub sites: Vec<super::sites::Site>,
    /// Errors the body creates with their message arguments, for error-detail questions.
    pub errors: Vec<super::errors::CreatedError>,
    pub calls: BTreeSet<String>,
    /// A Java `equals(Object)` or `hashCode()` override: boilerplate whose
    /// field-by-field copies and hash multipliers are the idiom, so it offers
    /// no copies or literal values to judge.
    pub equality: bool,
    /// Spring MVC routes a Java controller method maps, so tests that send
    /// requests to them are linked to it.
    pub routes: Vec<super::routes::Route>,
    /// Type, field and imported names this unit mentions, including its own name.
    pub refs: BTreeSet<String>,
    /// Plain identifiers, used only while parsing to find functions passed by name.
    mentions: BTreeSet<String>,
}

impl Unit {
    pub fn source<'a>(&self, source: &'a str) -> &'a str {
        &source[self.span.clone()]
    }

    pub fn callable(&self) -> bool {
        self.kind != Kind::Type
    }

    pub fn too_small(&self) -> bool {
        self.body_lines < MIN_BODY_LINES
    }

    /// Control flow deep or long enough that flattening it is worth asking about.
    pub fn deeply_nested(&self) -> bool {
        self.nesting >= super::nesting::DEEP_NESTING
            || self.branch_chain >= super::nesting::LONG_CHAIN
    }

    pub fn lines(&self) -> usize {
        self.end_line + 1 - self.line
    }

    pub fn overlaps(&self, lines: &Range<usize>) -> bool {
        self.line < lines.end && lines.start <= self.end_line
    }
}

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
    /// Whether it is Django code: Python that imports Django or Django REST
    /// framework, or a Django settings module.
    pub django: bool,
    /// In Django code, the URL routes it declares.
    pub routes: Vec<super::django::Route>,
    /// In Django code, its upper-case module constants with their
    /// assignments as shown.
    pub module_constants: Vec<(String, String)>,
}

/// Units of a supported language. Unsupported languages return an unparsed,
/// empty result; syntax errors are an error, never an empty clear file.
pub fn parse(path: &Path, source: &str) -> Result<FileUnits> {
    let Some(tree) = crate::syntax::parse(path, source)? else {
        return Ok(FileUnits::default());
    };
    let settings = super::django::settings_module(path, tree.root_node(), source);
    let mut file = FileUnits {
        parsed: true,
        django: settings || super::django::imports_django(path, tree.root_node(), source),
        ..Default::default()
    };
    walk(tree.root_node(), source, "", &mut file);
    if file.django {
        file.routes = super::django::routes(tree.root_node(), source);
        if !settings {
            file.module_constants = super::django::module_constants(tree.root_node(), source);
        }
    }
    file.constants = super::literals::constants(tree.root_node(), source);
    let spans: Vec<Range<usize>> = file.units.iter().map(|u| u.span.clone()).collect();
    file.setup = if framework_config(path) {
        super::sites::config_setup(tree.root_node(), source)
    } else if super::php::file(path) {
        super::sites::script(tree.root_node(), source, &spans)
    } else {
        super::sites::setup(tree.root_node(), source, &spans, settings)
    };
    // A function passed by name, such as `map(parse)`, is used like a call.
    let names: BTreeSet<String> = file.units.iter().map(|u| u.short_name.clone()).collect();
    for unit in &mut file.units {
        let used: Vec<String> = unit
            .mentions
            .intersection(&names)
            .filter(|name| **name != unit.short_name)
            .cloned()
            .collect();
        unit.calls.extend(used);
        unit.mentions.clear();
    }
    Ok(file)
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
/// the type owns, and a type without any is one unit itself.
fn owning_type(node: Node<'_>, name: &str, source: &str, file: &mut FileUnits) {
    let before = file.units.len();
    if let Some(body) = node.child_by_field_name("body") {
        children(body, source, name, file);
    }
    if file.units.len() == before && !name.is_empty() {
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
    // A definition holding a syntax error is not judged: the rest of its
    // file can be, when its errors are few (`syntax::parse`).
    if short_name.is_empty() || definition.outer.has_error() {
        return;
    }
    let Definition { outer, node, body } = definition;
    let start = leading_start(outer);
    // Parameters and return types count as references, not only the body.
    let mut facts = Facts::default();
    facts.visit(node, source);
    if kind == Kind::Type {
        facts.calls.clear();
    }
    let mut refs = facts.refs;
    refs.extend(facts.idents.intersection(&file.imports).cloned());
    if kind == Kind::Type {
        facts.idents.clear();
    }
    refs.insert(short_name.to_string());
    if !owner.is_empty() {
        refs.insert(owner.to_string());
    }
    let equality = equality_override(node, short_name, source);
    file.units.push(Unit {
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
        nesting: body.map_or(0, |b| super::nesting::control(b).0),
        branch_chain: body.map_or(0, |b| super::nesting::control(b).1),
        blocks: body.map_or_else(Vec::new, |b| super::blocks::blocks(b, source)),
        literals: body
            .filter(|_| !equality && !super::literals::returns_constant(node))
            .map_or_else(Vec::new, |b| super::literals::in_node(b, source)),
        sites: body.map_or_else(Vec::new, |b| super::sites::in_node(b, source, file.django)),
        errors: body.map_or_else(Vec::new, |b| super::errors::created_errors(b, source)),
        calls: facts.calls,
        equality,
        routes: super::routes::spring(node, source),
        refs,
        mentions: facts.idents,
    });
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
