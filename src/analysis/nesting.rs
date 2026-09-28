//! How deeply a function body nests control flow, and its longest branch chain.

use super::generic::Language;
use tree_sitter::Node;

/// Control flow nested this deep, or a branch chain this long, is a flattening candidate.
pub const DEEP_NESTING: usize = 4;
pub const LONG_CHAIN: usize = 4;

const CONTROL: &[&str] = &[
    "if_statement",
    "if_expression",
    "for_statement",
    "for_in_statement",
    "foreach_statement",
    "for_expression",
    "while_statement",
    "while_expression",
    "loop_expression",
    "do_statement",
    "match_expression",
    "match_statement",
    "switch_statement",
    "expression_switch_statement",
    "type_switch_statement",
    "select_statement",
    "foreach_statement",
    "enhanced_for_statement",
    "switch_expression",
    "using_statement",
    "lock_statement",
    "try_statement",
    "try_with_resources_statement",
    "synchronized_statement",
    "with_statement",
    "conditional_expression",
    "ternary_expression",
    // Ruby: conditions, loops, `begin … rescue`, modifier forms such as
    // `return if done`, and `do … end` blocks (`items.each do |item|`).
    "if",
    "unless",
    "case",
    "case_match",
    "while",
    "until",
    "for",
    "begin",
    "conditional",
    "if_modifier",
    "unless_modifier",
    "while_modifier",
    "until_modifier",
    "do_block",
];

/// Maximum control-flow depth and longest branch chain under `node`. An `if`
/// that is the `else` branch of another `if` continues its chain instead of nesting.
pub(super) fn control(node: Node<'_>) -> (usize, usize) {
    fn chained(node: Node<'_>) -> bool {
        let parent = node.parent();
        match node.kind() {
            "if_statement" | "if_expression" => parent.is_some_and(|p| {
                p.kind() == "else_clause"
                    || (p.kind() == "if_statement"
                        && p.child_by_field_name("alternative") == Some(node))
                    || (p.kind() == "if_expression"
                        && p.child_by_field_name("alternative") == Some(node))
            }),
            "conditional_expression" | "ternary_expression" | "conditional" => {
                parent.is_some_and(|p| {
                    p.kind() == node.kind() && p.child_by_field_name("alternative") == Some(node)
                })
            }
            _ => false,
        }
    }
    fn chain(node: Node<'_>) -> usize {
        // Ruby links each `elsif` and the `else` through `alternative`.
        if matches!(node.kind(), "if" | "unless") {
            let mut length = 1;
            let mut alternative = node.child_by_field_name("alternative");
            while let Some(clause) = alternative {
                length += 1;
                alternative = clause.child_by_field_name("alternative");
            }
            return length;
        }
        // Python lists `elif` and `else` clauses as children of one
        // `if_statement`, and PHP its `elseif` clauses.
        let clauses = node
            .named_children(&mut node.walk())
            .filter(|c| matches!(c.kind(), "elif_clause" | "else_if_clause"))
            .count();
        if clauses > 0 {
            let otherwise = node
                .named_children(&mut node.walk())
                .any(|c| c.kind() == "else_clause");
            return 1 + clauses + usize::from(otherwise);
        }
        let mut length = 1;
        let mut current = node;
        loop {
            let alternative = current.child_by_field_name("alternative").map(|a| {
                if a.kind() == "else_clause" {
                    a.named_child(0).unwrap_or(a)
                } else {
                    a
                }
            });
            match alternative {
                Some(next) if next.kind() == current.kind() => {
                    length += 1;
                    current = next;
                }
                Some(_) => return length + 1,
                None => return length,
            }
        }
    }
    fn walk(node: Node<'_>, depth: usize, result: &mut (usize, usize)) {
        // A Ruby `{ … }` block passed to a call nests like `do … end`; other
        // languages' `block` is a body. A Java lambda with a block body, as
        // in `items.forEach(i -> { … })`, nests its statements; an
        // expression lambda does not.
        let control = CONTROL.contains(&node.kind())
            || node.kind() == "block" && node.parent().is_some_and(|p| p.kind() == "call")
            || node.kind() == "lambda_expression"
                && node
                    .child_by_field_name("body")
                    .is_some_and(|b| b.kind() == "block");
        let depth = if control && !chained(node) {
            if matches!(
                node.kind(),
                "if_statement"
                    | "if_expression"
                    | "conditional_expression"
                    | "ternary_expression"
                    | "if"
                    | "unless"
                    | "conditional"
            ) {
                result.1 = result.1.max(chain(node));
            }
            depth + 1
        } else {
            depth
        };
        result.0 = result.0.max(depth);
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            walk(child, depth, result);
        }
    }
    let mut result = (0, 0);
    walk(node, 0, &mut result);
    result
}

/// Maximum control-flow depth and longest branch chain under the body of a
/// language of the generic tier, from its table's kinds: a conditional in
/// another's `else` continues that chain rather than nesting, and each
/// `elif`, `elseif` or `else` it lists adds a branch.
pub(super) fn generic(body: Node<'_>, language: &Language) -> (usize, usize) {
    fn walk(node: Node<'_>, depth: usize, language: &Language, result: &mut (usize, usize)) {
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            let nests = language.control.contains(&child.kind()) && !continues(child, language);
            if nests && language.conditionals.contains(&child.kind()) {
                result.1 = result.1.max(chain(child, language));
            }
            let depth = depth + usize::from(nests);
            result.0 = result.0.max(depth);
            walk(child, depth, language, result);
        }
    }
    let mut result = (0, 0);
    walk(body, 0, language, &mut result);
    result
}

/// A conditional in the `else` of another: right after an `else` of its
/// parent conditional (Kotlin, Swift, Scala, Dart), or alone in an `else`
/// clause (C).
fn continues(node: Node<'_>, language: &Language) -> bool {
    language.conditionals.contains(&node.kind())
        && node.parent().is_some_and(|parent| {
            language.conditionals.contains(&parent.kind()) && after_else(node)
                || language.clauses.contains(&parent.kind()) && parent.named_child_count() == 1
        })
}

/// Right after an `else`, or after the `{` that opens Swift's `else` block.
fn after_else(node: Node<'_>) -> bool {
    std::iter::successors(node.prev_sibling(), Node::prev_sibling)
        .find(|previous| previous.kind() != "{")
        .is_some_and(|previous| previous.kind() == "else")
}

/// The branches of a chain of conditionals: the first, each one continuing
/// it, each other clause they list and each final `else` block.
fn chain(node: Node<'_>, language: &Language) -> usize {
    let mut length = 1;
    let mut current = node;
    loop {
        let mut next = None;
        let mut cursor = current.walk();
        for child in current.children(&mut cursor) {
            if continues(child, language) {
                next = Some(child);
            } else if language.clauses.contains(&child.kind()) {
                match child
                    .named_child(0)
                    .filter(|inner| continues(*inner, language))
                {
                    Some(inner) => next = Some(inner),
                    None => length += 1,
                }
            } else if child.is_named() && after_else(child) {
                length += 1;
            }
        }
        let Some(inner) = next else {
            return length;
        };
        length += 1;
        current = inner;
    }
}
