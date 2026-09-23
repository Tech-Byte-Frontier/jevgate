//! Functions, methods and types of one file, with the local facts used for
//! grouping, callee and subject lookup, and marking bodies too small to judge.
use super::{callee_name, is_comment, line_of, macro_calls, text};
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
    pub calls: BTreeSet<String>,
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
    /// False when no parser supports this language.
    pub parsed: bool,
}

/// Units of a supported language. Unsupported languages return an unparsed,
/// empty result; syntax errors are an error, never an empty clear file.
pub fn parse(path: &Path, source: &str) -> Result<FileUnits> {
    let Some(tree) = crate::syntax::parse(path, source)? else {
        return Ok(FileUnits::default());
    };
    let mut file = FileUnits {
        parsed: true,
        ..Default::default()
    };
    walk(tree.root_node(), source, "", &mut file);
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

fn walk(node: Node<'_>, source: &str, owner: &str, file: &mut FileUnits) {
    match node.kind() {
        "use_declaration" | "import_statement" | "import_from_statement" => {
            imports(node, source, &mut file.imports);
        }
        "source_file" | "program" | "module" | "declaration_list" | "class_body"
        | "export_statement" | "statement_block" => children(node, source, owner, file),
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
        "class_declaration" | "class_definition" | "class" | "abstract_class_declaration" => {
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
        | "interface_declaration"
        | "type_alias_declaration"
        | "enum_declaration" => {
            let name = name_of(node, source);
            if !name.is_empty() {
                push(Definition::whole(node), &name, "", Kind::Type, source, file);
            }
        }
        _ => {}
    }
}

/// The function a declaration defines: the value itself, or the last function
/// argument of a call (as in `useCallback(fn, deps)`), looking through up to
/// `depth` nested calls.
fn callback(value: Node<'_>, depth: usize) -> Option<Node<'_>> {
    match value.kind() {
        "arrow_function" | "function_expression" | "function" => Some(value),
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
    let signature_end = body.map_or(outer.end_byte(), |b| b.start_byte());
    let signature = clip(
        source[outer.start_byte()..signature_end]
            .lines()
            .map(str::trim)
            .filter(|line| !line.starts_with("#[") && !line.starts_with('@'))
            .collect::<Vec<_>>()
            .join(" ")
            .trim_end_matches(['{', ':', ' ', '='])
            .trim_end_matches("=>")
            .trim(),
        240,
    );
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
        signature,
        doc: doc_line(&source[start..outer.start_byte()], outer, source),
        body_lines: body.map_or(0, |b| body_lines(text(b, source))),
        nesting: body.map_or(0, |b| super::nesting::control(b).0),
        branch_chain: body.map_or(0, |b| super::nesting::control(b).1),
        blocks: body.map_or_else(Vec::new, |b| super::blocks::blocks(b, source)),
        calls: facts.calls,
        refs,
        mentions: facts.idents,
    });
}

/// Include documentation, comments and attributes directly above the definition.
fn leading_start(node: Node<'_>) -> usize {
    let mut start = node.start_byte();
    let mut previous = node.prev_named_sibling();
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

fn doc_line(leading: &str, node: Node<'_>, source: &str) -> String {
    let comment = leading
        .lines()
        .map(clean_comment)
        .find(|line| !line.is_empty());
    let docstring = || {
        // A Python docstring is the first statement of the body.
        let body = node.child_by_field_name("body")?;
        let first = body.named_child(0)?;
        let string = (first.kind() == "expression_statement")
            .then(|| first.named_child(0))
            .flatten()
            .filter(|n| n.kind() == "string")?;
        text(string, source)
            .trim_matches(['"', '\'', 'r', 'b', 'f'])
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .map(str::to_string)
    };
    clip(&comment.or_else(docstring).unwrap_or_default(), 160)
}

fn clean_comment(line: &str) -> String {
    let line = line.trim();
    if line.starts_with("#[") {
        return String::new();
    }
    line.trim_start_matches(['/', '*', '!', '#'])
        .trim_end_matches("*/")
        .trim()
        .to_string()
}

fn clip(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    format!("{}…", text.chars().take(limit).collect::<String>())
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
            "call_expression" | "call" => {
                if let Some(name) = node
                    .child_by_field_name("function")
                    .and_then(|f| callee_name(f, source))
                {
                    self.calls.insert(name);
                }
            }
            "new_expression" => {
                if let Some(name) = node
                    .child_by_field_name("constructor")
                    .and_then(|c| callee_name(c, source))
                {
                    self.refs.insert(name.clone());
                    self.calls.insert(name);
                }
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
            "type_identifier" | "field_identifier" | "property_identifier" => {
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

    #[test]
    fn rust_units_include_methods_types_and_local_facts() {
        let source = "use std::path::PathBuf;\n\n/// Stored records.\npub struct Store { root: PathBuf }\n\nimpl Store {\n    /// Opens the store.\n    pub fn open(root: PathBuf, strict: bool) -> Self {\n        if strict {\n            for _ in 0..2 {\n                check(&root);\n            }\n        }\n        let store = Self { root };\n        store.touch();\n        store\n    }\n    fn touch(&self) {}\n}\n\nfn check(path: &PathBuf) {}\n\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn opens() {}\n}\n";
        let file = parse(Path::new("store.rs"), source).unwrap();
        let open = file.units.iter().find(|u| u.name == "Store::open").unwrap();
        assert_eq!(open.kind, Kind::Method);
        assert_eq!(open.doc, "Opens the store.");
        assert_eq!(
            open.signature,
            "pub fn open(root: PathBuf, strict: bool) -> Self"
        );
        assert!(open.calls.contains("check") && open.calls.contains("touch"));
        assert!(open.refs.contains("PathBuf") && open.refs.contains("Store"));
        assert_eq!(open.body_lines, 6);
        assert!(!open.too_small());
        assert!(open.source(source).starts_with("/// Opens"));
        let touch = file
            .units
            .iter()
            .find(|u| u.name == "Store::touch")
            .unwrap();
        assert!(touch.too_small());
        assert_eq!(
            names("store.rs", source),
            [
                ("Store".into(), Kind::Type),
                ("Store::open".into(), Kind::Method),
                ("Store::touch".into(), Kind::Method),
                ("check".into(), Kind::Function),
                ("opens".into(), Kind::Function),
            ]
        );
        assert!(file.imports.contains("PathBuf"));
    }

    #[test]
    fn python_and_typescript_units_follow_classes_and_bindings() {
        let python = "import os\n\nclass Loader:\n    def load(self, name):\n        \"\"\"Read one file.\"\"\"\n        return os.path.join(name)\n\n@cache\ndef helper(value):\n    return value\n";
        assert_eq!(
            names("loader.py", python),
            [
                ("Loader::load".into(), Kind::Method),
                ("helper".into(), Kind::Function)
            ]
        );
        let load = parse(Path::new("loader.py"), python)
            .unwrap()
            .units
            .remove(0);
        assert_eq!(load.doc, "Read one file.");
        let typescript = "export interface Row { id: string }\nexport const label = (row: Row): string => row.id.trim();\nexport class View {\n  render(row: Row) { return <Cell value={label(row)} />; }\n}\nfunction plain() { return 1; }\n";
        assert_eq!(
            names("view.tsx", typescript),
            [
                ("Row".into(), Kind::Type),
                ("label".into(), Kind::Function),
                ("View::render".into(), Kind::Method),
                ("plain".into(), Kind::Function),
            ]
        );
        let render = parse(Path::new("view.tsx"), typescript)
            .unwrap()
            .units
            .remove(2);
        assert!(render.calls.contains("Cell") && render.calls.contains("label"));
    }

    #[test]
    fn unsupported_languages_are_unparsed() {
        assert!(!parse(Path::new("main.go"), "package main").unwrap().parsed);
    }

    #[test]
    fn syntax_errors_fail_instead_of_returning_no_units() {
        assert!(parse(Path::new("broken.rs"), "fn broken( {").is_err());
    }
}
