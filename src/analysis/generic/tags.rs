//! What a language's tag query captures: its definitions, with their names,
//! bodies and scopes, and the names its calls call.
use super::Language;
use tree_sitter::{Node, QueryCursor, StreamingIterator};

/// What a definition is to the rules: code that runs, or a type that holds it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Defines {
    Function,
    Type,
}

/// One definition a tag query captured.
#[derive(Clone, Copy)]
pub(crate) struct Tag<'t> {
    pub node: Node<'t>,
    pub name: Node<'t>,
    pub defines: Defines,
    /// The node holding its body, when the grammar has no `body` field.
    pub body: Option<Node<'t>>,
    /// The type or table it is written in, as `Cart` in `Cart::add`.
    pub scope: Option<Node<'t>>,
}

/// A file's definitions and the calls it makes, in source order.
pub(crate) struct Tags<'t> {
    pub definitions: Vec<Tag<'t>>,
    /// The name each call names.
    pub calls: Vec<Node<'t>>,
}

/// The definitions and calls a language's tag query captures under `root`.
pub(crate) fn tags<'t>(language: &Language, root: Node<'t>, source: &'t str) -> Tags<'t> {
    let query = language.query();
    let names = query.capture_names();
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(query, root, source.as_bytes());
    let mut tags = Tags {
        definitions: Vec::new(),
        calls: Vec::new(),
    };
    while let Some(found) = matches.next() {
        let mut captured = Captured::default();
        for capture in found.captures() {
            captured.add(names[capture.index as usize], capture.node);
        }
        captured.record(&mut tags);
    }
    // A node that two patterns capture is one definition.
    tags.definitions
        .sort_by_key(|d| (d.node.start_byte(), d.node.id()));
    tags.definitions.dedup_by_key(|d| d.node.id());
    tags.calls.sort_by_key(Node::start_byte);
    tags
}

/// The captures of one match, by their role.
#[derive(Default)]
struct Captured<'t> {
    definition: Option<(Node<'t>, Defines)>,
    call: bool,
    name: Option<Node<'t>>,
    body: Option<Node<'t>>,
    scope: Option<Node<'t>>,
}

impl<'t> Captured<'t> {
    fn add(&mut self, capture: &str, node: Node<'t>) {
        match capture {
            "name" => self.name = Some(node),
            "body" => self.body = Some(node),
            "scope" => self.scope = Some(node),
            "reference.call" => self.call = true,
            "definition.function" | "definition.method" => {
                self.definition = Some((node, Defines::Function));
            }
            _ if capture.starts_with("definition.") => {
                self.definition = Some((node, Defines::Type));
            }
            _ => {}
        }
    }

    fn record(self, tags: &mut Tags<'t>) {
        let Some(name) = self.name else {
            return;
        };
        if let Some((node, defines)) = self.definition {
            tags.definitions.push(Tag {
                node,
                name,
                defines,
                body: self.body,
                scope: self.scope,
            });
        } else if self.call {
            tags.calls.push(name);
        }
    }
}
