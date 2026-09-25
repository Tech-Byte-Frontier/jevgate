//! Functions, methods and types of one file, with the local facts used for
//! grouping, callee and subject lookup, and marking bodies too small to judge.
use super::{call_name, callee_name, is_comment, line_of, macro_calls, ruby, summary, text};
use anyhow::Result;
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
            let name = base_type(&name_of(node, source));
            let before = file.units.len();
            if let Some(body) = node.child_by_field_name("body") {
                children(body, source, &name, file);
            }
            if file.units.len() == before && !name.is_empty() {
                push(Definition::whole(node), &name, "", Kind::Type, source, file);
            }
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
            let name = name_of(node, source);
            let before = file.units.len();
            if let Some(body) = node.child_by_field_name("body") {
                children(body, source, &name, file);
            }
            if file.units.len() == before && !name.is_empty() {
                push(Definition::whole(node), &name, "", Kind::Type, source, file);
            }
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
                let Some(function) = callback(value, 2) else {
                    continue;
                };
                let name = declarator
                    .child_by_field_name("name")
                    .map(|n| text(n, source).to_string())
                    .unwrap_or_default();
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

/// A Ruby call at the level of a file, class or module: a `require` names an
/// import; `define_method(:name) do … end` defines a method; a block that
/// holds definitions (`helpers do`, `included do`) is read for them; test
/// declarations are read only for the definitions inside them; and another
/// call with a block registers it, named by its call: `get('/')`.
fn ruby_call(node: Node<'_>, source: &str, owner: &str, file: &mut FileUnits) {
    if let Some(path) = ruby::required(node, source) {
        let stem = path.rsplit('/').next().unwrap_or(path);
        let stem = stem.strip_suffix(".rb").unwrap_or(stem);
        if !stem.is_empty() {
            file.imports.insert(stem.to_string());
        }
        return;
    }
    let Some(body) = ruby::block_body(node) else {
        // `private def total` and `memoize def rates` define a method.
        if let Some(arguments) = node.child_by_field_name("arguments") {
            let mut cursor = arguments.walk();
            for argument in arguments.named_children(&mut cursor) {
                if matches!(argument.kind(), "method" | "singleton_method") {
                    walk(argument, source, owner, file);
                }
            }
        }
        return;
    };
    let method = ruby::method(node, source);
    if method == "define_method"
        && let Some(name) = ruby::first_argument(node).filter(|a| a.kind() == "simple_symbol")
    {
        let definition = Definition {
            outer: node,
            node,
            body: Some(body),
        };
        let name = text(name, source).trim_start_matches(':');
        let kind = if owner.is_empty() {
            Kind::Function
        } else {
            Kind::Method
        };
        push(definition, name, owner, kind, source, file);
        return;
    }
    if ruby::test_dsl(node, source) {
        ruby_block_definitions(body, source, owner, file);
        return;
    }
    if ruby::defines(body) {
        children(body, source, owner, file);
        return;
    }
    let receiver = node
        .child_by_field_name("receiver")
        .map(|r| format!("{}.", text(r, source)))
        .unwrap_or_default();
    let argument = match ruby::first_argument(node) {
        Some(first)
            if matches!(
                first.kind(),
                "string" | "simple_symbol" | "constant" | "scope_resolution"
            ) =>
        {
            format!("({})", text(first, source))
        }
        Some(_) => "(…)".into(),
        None => String::new(),
    };
    let definition = Definition {
        outer: node,
        node,
        body: Some(body),
    };
    let name = format!("{receiver}{method}{argument}");
    push(definition, &name, owner, Kind::Function, source, file);
}

/// Definitions inside the blocks of Ruby test declarations, such as a helper
/// method in a `describe` block; the blocks themselves are left to the test rules.
fn ruby_block_definitions(body: Node<'_>, source: &str, owner: &str, file: &mut FileUnits) {
    let mut cursor = body.walk();
    for child in body.named_children(&mut cursor) {
        match child.kind() {
            "method" | "singleton_method" | "class" | "module" | "singleton_class" => {
                walk(child, source, owner, file);
            }
            "call" => {
                if let Some(inner) = ruby::block_body(child) {
                    ruby_block_definitions(inner, source, owner, file);
                }
            }
            _ => {}
        }
    }
}

/// A Ruby constant bound to a lambda (`ROUND = ->(value) { … }`) is a
/// function; one bound to a class built with a block (`Point = Struct.new(:x)
/// do … end`) owns the methods of the block.
fn ruby_assignment(node: Node<'_>, source: &str, owner: &str, file: &mut FileUnits) {
    let (Some(left), Some(right)) = (
        node.child_by_field_name("left"),
        node.child_by_field_name("right"),
    ) else {
        return;
    };
    if !matches!(left.kind(), "constant" | "identifier") {
        return;
    }
    let name = text(left, source);
    if right.kind() == "lambda" {
        let definition = Definition {
            outer: node,
            node: right,
            body: right
                .child_by_field_name("body")
                .and_then(|b| b.child_by_field_name("body")),
        };
        push(definition, name, owner, Kind::Function, source, file);
    } else if let Some(body) = ruby::block_body(right).filter(|b| ruby::defines(*b)) {
        children(body, source, name, file);
    }
}

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
fn registered_callbacks<'t>(statement: Node<'t>, source: &str) -> Vec<(String, Node<'t>)> {
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
fn csharp_callbacks<'t>(statement: Node<'t>, source: &str) -> Vec<(String, Node<'t>)> {
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
fn callback(value: Node<'_>, depth: usize) -> Option<Node<'_>> {
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

#[derive(Default)]
struct Facts {
    calls: BTreeSet<String>,
    refs: BTreeSet<String>,
    idents: BTreeSet<String>,
}

impl Facts {
    fn visit(&mut self, node: Node<'_>, source: &str) {
        if is_comment(node) {
            return;
        }
        match node.kind() {
            "call_expression" | "call" | "invocation_expression" => {
                if let Some(name) = call_name(node, source) {
                    self.calls.insert(name);
                }
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
                    .or_else(|| super::php::callee_name(node, source))
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
            kind if super::php::CALLS.contains(&kind) => {
                self.calls.extend(super::php::callee_name(node, source));
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

/// Go imports: each package's name, its alias or the last path segment.
fn go_imports(node: Node<'_>, source: &str, names: &mut BTreeSet<String>) {
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
fn csharp_import(node: Node<'_>, source: &str, names: &mut BTreeSet<String>) {
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
fn java_import(node: Node<'_>, source: &str, names: &mut BTreeSet<String>) {
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

fn imports(node: Node<'_>, source: &str, names: &mut BTreeSet<String>) {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn names(path: &str, source: &str) -> Vec<(String, Kind)> {
        parse(Path::new(path), source)
            .unwrap()
            .units
            .into_iter()
            .map(|u| (u.name, u.kind))
            .collect()
    }

    #[test]
    fn callbacks_registered_through_calls_are_units_named_by_their_declaration() {
        let source = "export const myJourney = database.view({ public: true }, t.array(row), (ctx) => {\n  const account = activeAccount(ctx)\n  if (!account) return []\n  return [account]\n})\nconst save = useCallback(async () => {\n  await store.save()\n}, [store])\nconst Panel = memo(forwardRef((props, ref) => {\n  return render(props, ref)\n}))\nconst table = database.table({ name: 'x' })\n";
        let units = parse(Path::new("views.ts"), source).unwrap().units;
        let named: Vec<(&str, bool)> = units
            .iter()
            .map(|u| (u.name.as_str(), u.body.is_some()))
            .collect();
        assert_eq!(
            named,
            [("myJourney", true), ("save", true), ("Panel", true)]
        );
        assert!(units[0].calls.contains("activeAccount"));
    }

    #[test]
    fn callbacks_registered_at_module_level_are_units_named_by_their_registration() {
        let source = "const app = new Hono()\napp.post('/pages', validator('form', (v, c) => check(v)), async (c) => {\n  const page = await insertPage(c.env.DB)\n  return c.json(page)\n})\nrouter.route('/items').get((req, res) => {\n  res.send(list())\n}).delete(async (req, res) => {\n  await remove(req.params.id)\n})\napp.use(async (c, next) => {\n  await next()\n})\ndescribe('pages', () => {\n  it('saves', () => {})\n})\ntest.beforeEach(async () => {})\nvi.mock('./db', () => ({ query: vi.fn() }))\nexport const getStaticPaths = (async ({ paginate }) => {\n  return paginate(await posts())\n}) satisfies GetStaticPaths\nexport default {\n  async fetch(request, env) {\n    return app.fetch(request, env)\n  },\n}\n";
        let units = parse(Path::new("routes.ts"), source).unwrap().units;
        let named: Vec<(&str, usize)> = units.iter().map(|u| (u.name.as_str(), u.line)).collect();
        assert_eq!(
            named,
            [
                ("app.post('/pages')", 2),
                ("router.get(…)", 6),
                ("router.delete(…)", 8),
                ("app.use(…)", 11),
                ("getStaticPaths", 19),
                ("fetch", 23),
            ]
        );
        assert!(units[0].calls.contains("insertPage"));
    }

    #[test]
    fn go_functions_methods_types_and_their_facts_are_units() {
        let source = "package store\n\nimport (\n\t\"database/sql\"\n\t\"fmt\"\n)\n\nconst maxRows = 500\n\n// Store keeps users.\ntype Store struct {\n\tdb *sql.DB\n}\n\n// Find loads a user.\nfunc (s *Store) Find(name string) (*User, error) {\n\tq := fmt.Sprintf(\"SELECT * FROM users WHERE name = '%s'\", name)\n\tif name == \"\" {\n\t\treturn nil, fmt.Errorf(\"empty name: %w\", ErrInvalid)\n\t} else if name == \"root\" {\n\t\treturn nil, errors.New(\"reserved\")\n\t}\n\tfor i := 0; i < 3; i++ {\n\t\tswitch i {\n\t\tcase 1:\n\t\t\ts.db.Query(q)\n\t\t}\n\t}\n\treturn nil, nil\n}\n\nfunc New(db *sql.DB) *Store { return &Store{db: db} }\n";
        let file = parse(Path::new("store.go"), source).unwrap();
        let named: Vec<(&str, &str, usize)> = file
            .units
            .iter()
            .map(|u| (u.name.as_str(), u.owner.as_str(), u.line))
            .collect();
        assert_eq!(
            named,
            [
                ("Store", "", 11),
                ("Store::Find", "Store", 16),
                ("New", "", 32)
            ]
        );
        let find = &file.units[1];
        assert!(file.imports.contains("fmt") && file.imports.contains("sql"));
        assert!(find.calls.contains("Sprintf") && find.calls.contains("Query"));
        assert_eq!((find.nesting, find.branch_chain), (2, 2));
        assert_eq!(
            find.sites[0].text,
            "q := fmt.Sprintf(\"SELECT * FROM users WHERE name = '%s'\", name)"
        );
        let errors: Vec<&str> = find.errors.iter().map(|e| e.error.as_str()).collect();
        assert_eq!(errors, ["fmt.Errorf", "errors.New"]);
        assert!(find.literals.iter().any(|l| l.text == "\"root\""));
        assert_eq!(file.constants[0].name, "maxRows");
        assert!(crate::syntax::supported(Path::new("store.go")));
    }

    const BILLING: &str = "require 'json'\nrequire_relative 'billing/tax'\n\nmodule Billing\n  RETRIES = 3\n  ROUND = ->(value) { value.round(2) }\n\n  # Builds invoices.\n  class Invoice < Base\n    TAX = 0.21\n\n    def self.build(rows)\n      new(rows).tap(&:validate)\n    end\n\n    class << self\n      def empty\n        new([])\n      end\n    end\n\n    def total(discount = 0)\n      raise ArgumentError, \"negative discount\" if discount.negative?\n      sum = rows.sum { |row| row.price }\n      if sum > 1000\n        sum * (1 - TAX)\n      elsif sum > 100\n        sum\n      else\n        raise Billing::Error.new(\"too small\")\n      end\n    end\n\n    private def rows\n      @rows\n    end\n\n    define_method(:currency) do\n      Currency.new(\"EUR\").code\n    end\n  end\nend\n\nget '/invoices' do\n  Billing::Invoice.build(params).to_json\nend\n\nRSpec.configure do |config|\n  config.order = :random\nend\n";

    #[test]
    fn ruby_methods_follow_their_class_or_module_and_blocks_are_named_by_their_call() {
        let file = parse(Path::new("billing.rb"), BILLING).unwrap();
        let named: Vec<(&str, &str, usize)> = file
            .units
            .iter()
            .map(|u| (u.name.as_str(), u.owner.as_str(), u.line))
            .collect();
        assert_eq!(
            named,
            [
                ("Billing::ROUND", "Billing", 6),
                ("Invoice::build", "Invoice", 12),
                ("Invoice::empty", "Invoice", 17),
                ("Invoice::total", "Invoice", 22),
                ("Invoice::rows", "Invoice", 34),
                ("Invoice::currency", "Invoice", 38),
                ("get('/invoices')", "", 44),
            ],
            "`RSpec.configure` sets up tests and is not a unit"
        );
        assert!(file.imports.contains("json") && file.imports.contains("tax"));
        let total = &file.units[3];
        assert_eq!(total.signature, "def total(discount = 0)");
        assert_eq!((total.nesting, total.branch_chain), (1, 3));
        let errors: Vec<(&str, &str)> = total
            .errors
            .iter()
            .map(|e| (e.error.as_str(), e.message.as_str()))
            .collect();
        assert_eq!(
            errors,
            [
                ("ArgumentError", "\"negative discount\""),
                ("Billing::Error", "\"too small\"")
            ]
        );
        assert!(total.calls.contains("sum") && total.refs.contains("TAX"));
        assert!(
            file.units[5].calls.contains("Currency"),
            "`Currency.new` builds a Currency"
        );
        let constants: Vec<&str> = file.constants.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(constants, ["RETRIES", "TAX"]);
        assert!(
            file.units
                .iter()
                .all(|u| u.literals.iter().all(|l| !l.text.contains("tax"))),
            "required paths are not values"
        );
    }

    #[test]
    fn ruby_test_blocks_are_left_to_the_test_rules_but_their_helpers_are_units() {
        let source = "describe Invoice do\n  let(:invoice) { Invoice.new }\n\n  def build_rows(count)\n    Array.new(count) { Row.new }\n  end\n\n  it \"totals\" do\n    expect(invoice.total).to eq 0\n  end\nend\n";
        let names: Vec<String> = parse(Path::new("invoice_spec.rb"), source)
            .unwrap()
            .units
            .into_iter()
            .map(|u| u.name)
            .collect();
        assert_eq!(names, ["build_rows"]);
    }

    #[test]
    fn php_functions_methods_closures_and_their_facts_are_units() {
        let source = "<?php\nnamespace App\\Store;\n\nuse App\\Domain\\User\\UserRepository;\nuse Psr\\Log\\{LoggerInterface, NullLogger as Quiet};\n\nconst MAX_ROWS = 500;\n\n/** Finds a user. */\nfunction find_user($db, $name) {\n    $sql = \"SELECT * FROM users WHERE name = '$name'\";\n    if ($name === '') {\n        throw new InvalidArgumentException('empty name');\n    } elseif ($name === 'root') {\n        return null;\n    } elseif ($name === 'admin') {\n        return null;\n    }\n    foreach ([1, 2] as $i) {\n        $db->query($sql);\n    }\n    return Row::from(new Cursor($db));\n}\n\nabstract class Store extends Base {\n    public function __construct(private PDO $pdo) {}\n    public function load(int $id): ?User { return $this->pdo->prepare('x')->execute([$id]); }\n}\ntrait Cached { public function flush() { cache_clear(); } }\ninterface Finder { public function find(int $id): ?array; }\nenum Suit: string { case Hearts = 'H'; }\n$app->get('/users/{id}', function (Request $request, Response $response) {\n    return $response;\n});\nRoute::post('/pages', fn () => save());\n$handler = function ($e) { report($e); };\n";
        let file = parse(Path::new("store.php"), source).unwrap();
        let named: Vec<(&str, Kind, usize)> = file
            .units
            .iter()
            .map(|u| (u.name.as_str(), u.kind, u.line))
            .collect();
        assert_eq!(
            named,
            [
                ("find_user", Kind::Function, 10),
                ("Store::__construct", Kind::Method, 26),
                ("Store::load", Kind::Method, 27),
                ("Cached::flush", Kind::Method, 29),
                ("Finder", Kind::Type, 30),
                ("Suit", Kind::Type, 31),
                ("$app->get('/users/{id}')", Kind::Function, 32),
                ("Route::post('/pages')", Kind::Function, 35),
                ("$handler", Kind::Function, 36),
            ]
        );
        let find = &file.units[0];
        assert_eq!(find.doc, "Finds a user.");
        for callee in ["query", "from", "Cursor", "InvalidArgumentException"] {
            assert!(find.calls.contains(callee), "{callee}");
        }
        assert_eq!((find.nesting, find.branch_chain), (1, 3));
        assert_eq!(find.errors[0].error, "InvalidArgumentException");
        assert_eq!(find.errors[0].message, "'empty name'");
        assert!(find.literals.iter().any(|l| l.text == "'root'"));
        assert!(file.units[2].refs.contains("User"));
        for name in ["UserRepository", "LoggerInterface", "Quiet"] {
            assert!(file.imports.contains(name), "{name}");
        }
        assert_eq!(file.constants[0].name, "MAX_ROWS");
        assert!(crate::syntax::supported(Path::new("store.php")));
        let config = parse(
            Path::new("app/routes.php"),
            "<?php\nreturn function (App $app) {\n    $app->get('/', fn () => home());\n};\n",
        )
        .unwrap();
        assert_eq!(config.units[0].name, "returned closure");
        assert!(config.units[0].calls.contains("get"));
    }

    #[test]
    fn nesting_and_branch_chains_are_measured_without_counting_else_if_as_depth() {
        let facts = |path: &str, source: &str| {
            let unit = parse(Path::new(path), source).unwrap().units.remove(0);
            (unit.nesting, unit.branch_chain, unit.deeply_nested())
        };
        let ternary = "function icon(kind) {\n  const name = kind === 'a' ? 'x' : kind === 'b' ? 'y' : kind === 'c' ? 'z' : kind === 'd' ? 'w' : 'v'\n  return name\n}\n";
        assert_eq!(facts("a.ts", ternary), (1, 5, true));
        let chain = "function f(x) {\n  if (x === 1) return 1\n  else if (x === 2) return 2\n  else return 3\n}\n";
        assert_eq!(facts("b.ts", chain), (1, 3, false));
        let deep = "fn f(xs: &[Vec<u8>]) {\n    for x in xs {\n        if x.len() > 1 {\n            for y in x {\n                if *y > 2 {\n                    println!(\"{y}\");\n                }\n            }\n        }\n    }\n}\n";
        assert_eq!(facts("c.rs", deep), (4, 1, true));
        let python = "def f(x):\n    if x == 1:\n        return 1\n    elif x == 2:\n        return 2\n    elif x == 3:\n        return 3\n    else:\n        return 4\n";
        assert_eq!(facts("d.py", python), (1, 4, true));
        let open = "def f(x):\n    if x == 1:\n        return 1\n    elif x == 2:\n        return 2\n    return 3\n";
        assert_eq!(facts("e.py", open), (1, 2, false));
    }

    const STORE: &str = "use std::path::PathBuf;\n\n/// Stored records.\npub struct Store { root: PathBuf }\n\nimpl Store {\n    /// Opens the store.\n    pub fn open(root: PathBuf, strict: bool) -> Self {\n        if strict {\n            for _ in 0..2 {\n                check(&root);\n            }\n        }\n        let store = Self { root };\n        store.touch();\n        store\n    }\n    fn touch(&self) {}\n}\n\nfn check(path: &PathBuf) {}\n\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn opens() {}\n}\n";

    fn store_unit(name: &str) -> Unit {
        parse(Path::new("store.rs"), STORE)
            .unwrap()
            .units
            .into_iter()
            .find(|u| u.name == name)
            .unwrap()
    }

    #[test]
    fn rust_units_are_types_methods_and_functions() {
        assert_eq!(
            names("store.rs", STORE),
            [
                ("Store".into(), Kind::Type),
                ("Store::open".into(), Kind::Method),
                ("Store::touch".into(), Kind::Method),
                ("check".into(), Kind::Function),
                ("opens".into(), Kind::Function),
            ]
        );
    }

    #[test]
    fn a_rust_method_keeps_its_signature_doc_and_leading_comment() {
        let open = store_unit("Store::open");
        assert_eq!(
            open.signature,
            "pub fn open(root: PathBuf, strict: bool) -> Self"
        );
        assert_eq!(open.doc, "Opens the store.");
        assert!(open.source(STORE).starts_with("/// Opens"));
    }

    #[test]
    fn empty_bodies_are_too_small() {
        assert!(!store_unit("Store::open").too_small());
        assert!(store_unit("Store::touch").too_small());
    }

    #[test]
    fn rust_imports_are_recorded() {
        assert!(
            parse(Path::new("store.rs"), STORE)
                .unwrap()
                .imports
                .contains("PathBuf")
        );
    }

    const LOADER: &str = "import os\n\nclass Loader:\n    def load(self, name):\n        \"\"\"Read one file.\"\"\"\n        return os.path.join(name)\n\n@cache\ndef helper(value):\n    return value\n";
    const VIEW: &str = "export interface Row { id: string }\nexport const label = (row: Row): string => row.id.trim();\nexport class View {\n  render(row: Row) { return <Cell value={label(row)} />; }\n}\nfunction plain() { return 1; }\n";

    #[test]
    fn python_methods_follow_their_class_and_decorated_functions_count() {
        assert_eq!(
            names("loader.py", LOADER),
            [
                ("Loader::load".into(), Kind::Method),
                ("helper".into(), Kind::Function)
            ]
        );
    }

    #[test]
    fn a_python_docstring_is_the_unit_doc() {
        let load = parse(Path::new("loader.py"), LOADER)
            .unwrap()
            .units
            .remove(0);
        assert_eq!(load.doc, "Read one file.");
    }

    #[test]
    fn typescript_units_include_interfaces_bound_arrows_and_methods() {
        assert_eq!(
            names("view.tsx", VIEW),
            [
                ("Row".into(), Kind::Type),
                ("label".into(), Kind::Function),
                ("View::render".into(), Kind::Method),
                ("plain".into(), Kind::Function),
            ]
        );
    }

    #[test]
    fn jsx_components_count_as_calls() {
        let render = parse(Path::new("view.tsx"), VIEW).unwrap().units.remove(2);
        assert!(render.calls.contains("Cell") && render.calls.contains("label"));
    }

    #[test]
    fn csharp_classes_records_and_their_members_are_units_with_facts() {
        let source = "using System.Data.SqlClient;\nusing Db = Microsoft.EntityFrameworkCore;\n\nnamespace Shop.Orders;\n\n/// <summary>Stores orders.</summary>\n[Route(\"api/[controller]\")]\npublic class OrderStore : Controller\n{\n    private const int MaxRows = 500;\n    private static readonly string Host = \"https://api.example.com\";\n    private readonly IOrderRepository _repository;\n\n    public OrderStore(IOrderRepository repository)\n    {\n        _repository = repository;\n    }\n\n    public int Count { get { return _repository.Count(); } }\n    public string Name { get; set; }\n\n    [HttpGet(\"search\")]\n    public async Task<IActionResult> Search(string keyword)\n    {\n        var query = $\"SELECT * FROM Products WHERE name LIKE '%{keyword}%'\";\n        using var command = new SqlCommand(query, _connection);\n        if (keyword == null)\n        {\n            throw new ArgumentException(\"empty keyword\");\n        }\n        else if (keyword == \"root\")\n        {\n            return BadRequest();\n        }\n        foreach (var item in await _repository.ListAsync<Order>())\n        {\n            switch (item.Kind) { case 3: break; }\n        }\n        return Ok(string.Format(\"{0} rows\", MaxRows));\n    }\n}\n\npublic record Person(string First, string Last);\n\npublic interface IOrderRepository { int Count(); }\n";
        let file = parse(Path::new("OrderStore.cs"), source).unwrap();
        let named: Vec<(&str, Kind, usize)> = file
            .units
            .iter()
            .map(|u| (u.name.as_str(), u.kind, u.line))
            .collect();
        assert_eq!(
            named,
            [
                ("OrderStore::OrderStore", Kind::Method, 14),
                ("OrderStore::Count", Kind::Method, 19),
                ("OrderStore::Search", Kind::Method, 22),
                ("Person", Kind::Type, 43),
                ("IOrderRepository", Kind::Type, 45),
            ]
        );
        assert!(file.imports.contains("SqlClient") && file.imports.contains("Db"));
        let search = &file.units[2];
        assert_eq!(
            search.signature,
            "public async Task<IActionResult> Search(string keyword)"
        );
        assert!(search.calls.contains("ListAsync") && search.calls.contains("SqlCommand"));
        assert!(search.refs.contains("IActionResult") && search.refs.contains("Order"));
        assert_eq!((search.nesting, search.branch_chain), (2, 2));
        assert!(
            search.sites[0].text.starts_with("var query = $\"SELECT"),
            "{:?}",
            search.sites
        );
        let errors: Vec<(&str, &str)> = search
            .errors
            .iter()
            .map(|e| (e.error.as_str(), e.message.as_str()))
            .collect();
        assert_eq!(errors, [("ArgumentException", "\"empty keyword\"")]);
        assert!(search.literals.iter().any(|l| l.text == "\"root\""));
        assert!(!search.literals.iter().any(|l| l.text.contains("search")));
        let constants: Vec<&str> = file.constants.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(constants, ["OrderStore.MaxRows", "OrderStore.Host"]);
        let constructor = &file.units[0];
        assert!(constructor.too_small());
        assert_eq!(constructor.doc, "");
        let documented = parse(Path::new("A.cs"), "public class A\n{\n    /// <summary>\n    /// Loads orders.\n    /// </summary>\n    void Run() { }\n}\n").unwrap();
        assert_eq!(documented.units[0].doc, "Loads orders.");
        assert!(crate::syntax::supported(Path::new("OrderStore.cs")));
    }

    #[test]
    fn csharp_top_level_statements_register_route_handlers_and_keep_setup() {
        let source = "var builder = WebApplication.CreateBuilder(args);\nbuilder.Services.AddCors(o => o.AddPolicy(\"all\", p => p.AllowAnyOrigin()));\nvar app = builder.Build();\napp.MapGet(\"/orders/{id}\", async (int id, OrderDb db) =>\n{\n    var order = await db.Orders.FindAsync(id);\n    return order is null ? Results.NotFound() : Results.Ok(order);\n}).RequireAuthorization();\napp.Use(async (context, next) => { await next(context); });\nstatic string Greet(string name)\n{\n    return $\"Hello {name}\";\n}\napp.Run();\n";
        let file = parse(Path::new("Program.cs"), source).unwrap();
        let named: Vec<(&str, usize)> = file
            .units
            .iter()
            .map(|u| (u.name.as_str(), u.line))
            .collect();
        assert_eq!(
            named,
            [
                ("app.MapGet(\"/orders/{id}\")", 4),
                ("app.Use(…)", 9),
                ("Greet", 10)
            ]
        );
        assert!(file.units[0].calls.contains("FindAsync"));
        let setup: Vec<usize> = file.setup.statements.iter().map(|s| s.1).collect();
        assert_eq!(setup, [1, 2, 3, 14]);
        assert!(
            file.setup.sites[1]
                .text
                .starts_with("builder.Services.AddCors(")
        );
    }

    #[test]
    fn unsupported_languages_are_unparsed() {
        assert!(!parse(Path::new("Main.kt"), "class Main {}").unwrap().parsed);
    }

    #[test]
    fn syntax_errors_fail_instead_of_returning_no_units() {
        assert!(parse(Path::new("broken.rs"), "fn broken( {").is_err());
    }

    const OWNERS: &str = "package app.owner;\n\nimport java.util.*;\nimport java.util.List;\nimport static app.Checks.requireName;\n\n/** Owners of pets. */\npublic class OwnerService {\n\n\tprivate static final String DEFAULT_CITY = \"Madison\";\n\n\tstatic int retries = 5;\n\n\tprivate final int pageSize = 25;\n\n\tprivate final OwnerRepository owners;\n\n\tpublic OwnerService(OwnerRepository owners) {\n\t\tthis.owners = owners;\n\t}\n\n\t/**\n\t * Finds owners by last name.\n\t */\n\t@Transactional\n\tpublic List<Owner> find(String name, int page) {\n\t\tif (name == null) {\n\t\t\tthrow new IllegalArgumentException(\"name is required\");\n\t\t} else if (name.isBlank()) {\n\t\t\treturn new ArrayList<>();\n\t\t} else if (page > 100) {\n\t\t\treturn List.of();\n\t\t}\n\t\tString query = String.format(\"last_name = '%s'\", name);\n\t\tfor (Owner owner : owners.findAll(query)) {\n\t\t\towner.getPets().forEach(pet -> {\n\t\t\t\tswitch (pet.getKind()) {\n\t\t\t\t\tcase \"cat\" -> requireName(pet);\n\t\t\t\t\tdefault -> log(\"skipped \" + pet.getName());\n\t\t\t\t}\n\t\t\t});\n\t\t}\n\t\treturn owners.page(query, page * 3);\n\t}\n\n\t@Override\n\tpublic boolean equals(Object other) {\n\t\tif (this == other) return true;\n\t\tif (!(other instanceof OwnerService)) return false;\n\t\tOwnerService that = (OwnerService) other;\n\t\treturn owners.equals(that.owners);\n\t}\n\n\t@Override\n\tpublic int hashCode() {\n\t\tint result = 17;\n\t\tresult = 31 * result + owners.hashCode();\n\t\tresult = 31 * result + 7;\n\t\treturn result;\n\t}\n\n\tstatic class Page {\n\t\tint size() { return 10; }\n\t}\n}\n\ninterface OwnerRepository {\n\tString TABLE = \"owners\";\n\n\tList<Owner> findAll(String query);\n\n\tdefault List<Owner> page(String query, int size) {\n\t\treturn findAll(query).subList(0, size);\n\t}\n}\n\nenum State {\n\tOPEN {\n\t\tvoid enter(Owner owner) {\n\t\t\towner.open();\n\t\t}\n\t},\n\tCLOSED;\n\n\tprivate static final int LIMIT = 42;\n\n\tvoid enter(Owner owner) {}\n}\n\nrecord Visit(String date, String description) {\n\tVisit {\n\t\trequireName(description);\n\t}\n}\n\n@interface Audited {}\n";

    #[test]
    fn java_methods_belong_to_their_class_interface_enum_constant_or_record() {
        let file = parse(Path::new("OwnerService.java"), OWNERS).unwrap();
        let named: Vec<(&str, Kind, usize)> = file
            .units
            .iter()
            .map(|u| (u.name.as_str(), u.kind, u.line))
            .collect();
        assert_eq!(
            named,
            [
                ("OwnerService::OwnerService", Kind::Method, 18),
                ("OwnerService::find", Kind::Method, 25),
                ("OwnerService::equals", Kind::Method, 46),
                ("OwnerService::hashCode", Kind::Method, 54),
                ("Page::size", Kind::Method, 63),
                ("OwnerRepository::findAll", Kind::Method, 70),
                ("OwnerRepository::page", Kind::Method, 72),
                ("OPEN::enter", Kind::Method, 79),
                ("State::enter", Kind::Method, 87),
                ("Visit::Visit", Kind::Method, 91),
                ("Audited", Kind::Type, 96),
            ]
        );
        assert!(crate::syntax::supported(Path::new("OwnerService.java")));
        // A wildcard import names no class; a static import names its member.
        let imports: Vec<&str> = file.imports.iter().map(String::as_str).collect();
        assert_eq!(imports, ["List", "requireName"]);
    }

    #[test]
    fn a_java_method_has_its_calls_nesting_values_sites_and_errors() {
        let file = parse(Path::new("OwnerService.java"), OWNERS).unwrap();
        let find = &file.units[1];
        assert_eq!(
            find.signature,
            "public List<Owner> find(String name, int page)"
        );
        assert_eq!(find.doc, "Finds owners by last name.");
        assert!(find.source(OWNERS).starts_with("/**\n\t * Finds"));
        for call in [
            "findAll",
            "forEach",
            "requireName",
            "format",
            "ArrayList",
            "page",
        ] {
            assert!(find.calls.contains(call), "{call}");
        }
        assert!(find.refs.contains("Owner") && find.refs.contains("OwnerService"));
        // The else-if chain is one level; the loop, lambda block and switch nest.
        assert_eq!((find.nesting, find.branch_chain), (3, 3));
        let literals: Vec<&str> = find.literals.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(
            literals,
            [
                "\"name is required\"",
                "100",
                "\"last_name = '%s'\"",
                "\"cat\"",
                "\"skipped \"",
                "3"
            ]
        );
        let sites: Vec<&str> = find.sites.iter().map(|s| s.text.as_str()).collect();
        assert!(
            sites.contains(&"String query = String.format(\"last_name = '%s'\", name);"),
            "{sites:?}"
        );
        assert!(
            sites.contains(&"log(\"skipped \" + pet.getName());"),
            "{sites:?}"
        );
        let errors: Vec<(&str, &str)> = find
            .errors
            .iter()
            .map(|e| (e.error.as_str(), e.message.as_str()))
            .collect();
        assert_eq!(
            errors,
            [("IllegalArgumentException", "\"name is required\"")]
        );
        assert_eq!(find.blocks.len(), 3);
        // Static fields and interface constants are constants; instance fields are not.
        let constants: Vec<&str> = file.constants.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(constants, ["DEFAULT_CITY", "retries", "TABLE", "LIMIT"]);
    }

    #[test]
    fn java_capacity_hints_and_numbers_a_method_returns_are_not_values() {
        let java = "class Costs {\n\tint cost() {\n\t\treturn 7;\n\t}\n\n\tString host() {\n\t\treturn \"db.internal\";\n\t}\n\n\tList<String> names() {\n\t\tList<String> names = new ArrayList<>(16);\n\t\tStringBuilder text = new StringBuilder(64);\n\t\tnames.add(text.append(new Timeout(30)).toString());\n\t\treturn names.subList(0, 5);\n\t}\n}\n";
        let file = parse(Path::new("Costs.java"), java).unwrap();
        let values: Vec<(&str, Vec<&str>)> = file
            .units
            .iter()
            .map(|u| {
                (
                    u.name.as_str(),
                    u.literals.iter().map(|l| l.text.as_str()).collect(),
                )
            })
            .collect();
        assert_eq!(
            values,
            [
                ("Costs::cost", vec![]),
                ("Costs::host", vec!["\"db.internal\""]),
                ("Costs::names", vec!["30", "5"]),
            ]
        );
    }

    #[test]
    fn java_equals_and_hash_code_offer_no_values() {
        let file = parse(Path::new("OwnerService.java"), OWNERS).unwrap();
        let equality: Vec<&str> = file
            .units
            .iter()
            .filter(|u| u.equality)
            .map(|u| u.name.as_str())
            .collect();
        assert_eq!(equality, ["OwnerService::equals", "OwnerService::hashCode"]);
        assert!(file.units[3].literals.is_empty());
    }
}
