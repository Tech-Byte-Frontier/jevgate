//! What a Bend 2 def is: a proof, a def that computes a type, or code, and
//! whether that code performs effects or builds text.
use crate::analysis::text;
use tree_sitter::Node;

/// Whether a type holds an equality (`{a == b : T}`) or its negation, as a
/// pair of equalities or an implication ending in one does.
pub(super) fn holds_equality(node: Node<'_>) -> bool {
    if node.kind() == "equality_type" {
        return true;
    }
    let mut cursor = node.walk();
    node.named_children(&mut cursor).any(|child| {
        // An equality inside a call's arguments is a value it passes.
        child.kind() != "arguments" && holds_equality(child)
    })
}

/// The name a statement applies: `Sort.Sorted` of `Sort.Sorted(Sort.sort(xs))`.
pub(super) fn head_name(node: Node<'_>, source: &str) -> Option<String> {
    match node.kind() {
        "call" => head_name(node.child_by_field_name("function")?, source),
        "type_application" => node
            .child_by_field_name("name")
            .map(|n| text(n, source).to_string()),
        "identifier" | "scoped_identifier" => Some(text(node, source).to_string()),
        "parenthesized_expression" => head_name(node.named_child(0)?, source),
        _ => None,
    }
}

/// Whether a def computes a type (`-> Type`, `-> Data`, `-> Kind(a)`):
/// applied, it is a proposition or a type family, not a runtime value.
pub(crate) fn type_level(definition: Node<'_>) -> bool {
    definition
        .child_by_field_name("return_type")
        .is_some_and(|t| t.kind() == "kind")
}

/// Whether a def performs effects: it returns `IO(…)`, runs a `do IO<…>:`
/// block or is a foreign effect whose body imports its C and JS host code.
/// Other defs are pure: no input reaches them from outside the program
/// except through their callers, and they log, store and send nothing.
pub(crate) fn effectful(definition: Node<'_>, source: &str) -> bool {
    let returns_io = definition
        .child_by_field_name("return_type")
        .is_some_and(|t| head_name(t, source).is_some_and(|h| h == "IO"));
    returns_io
        || definition
            .child_by_field_name("body")
            .is_some_and(|body| runs_io(body, source))
}

/// Whether a def joins text with `++`, as code that builds a request, a
/// query or markup for an effect to send does.
pub(crate) fn joins_text(definition: Node<'_>, source: &str) -> bool {
    fn joins(node: Node<'_>, source: &str) -> bool {
        if node.kind() == "binary_expression"
            && node
                .child_by_field_name("operator")
                .is_some_and(|o| text(o, source) == "++")
        {
            return true;
        }
        let mut cursor = node.walk();
        node.named_children(&mut cursor).any(|c| joins(c, source))
    }
    definition
        .child_by_field_name("body")
        .is_some_and(|body| joins(body, source))
}

fn runs_io(node: Node<'_>, source: &str) -> bool {
    match node.kind() {
        "import_statement" => true,
        "do_block" => node
            .child_by_field_name("monad")
            .is_some_and(|m| text(m, source) == "IO"),
        _ => {
            let mut cursor = node.walk();
            node.named_children(&mut cursor)
                .any(|child| runs_io(child, source))
        }
    }
}

/// Whether a def is a proof: it fills a claim of its file, or a law of a
/// file it imports (`def Laws.sort_perm(x, xs)` beside `import ./LAWS.bend
/// as Laws`), or states an equality in its return type. Every def of a
/// `PROOF.bend` proves a law or a lemma by convention (`units::parse`).
pub(crate) fn proof(
    definition: Node<'_>,
    claims: &[String],
    aliases: &[String],
    source: &str,
) -> bool {
    let Some(name) = definition.child_by_field_name("name") else {
        return false;
    };
    let name = text(name, source);
    let fills = claims.iter().any(|c| c == name)
        || name.split_once('.').is_some_and(|(alias, _)| {
            aliases.iter().any(|a| a == alias)
                && definition.child_by_field_name("return_type").is_none()
        });
    fills
        || definition
            .child_by_field_name("return_type")
            .is_some_and(holds_equality)
}
