//! How deeply a function body nests control flow, and its longest branch chain.

use tree_sitter::Node;

/// Control flow nested this deep, or a branch chain this long, is a flattening candidate.
pub const DEEP_NESTING: usize = 4;
pub const LONG_CHAIN: usize = 4;

const CONTROL: &[&str] = &[
    "if_statement",
    "if_expression",
    "for_statement",
    "for_in_statement",
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
    "try_statement",
    "with_statement",
    "conditional_expression",
    "ternary_expression",
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
            "conditional_expression" | "ternary_expression" => parent.is_some_and(|p| {
                p.kind() == node.kind() && p.child_by_field_name("alternative") == Some(node)
            }),
            _ => false,
        }
    }
    fn chain(node: Node<'_>) -> usize {
        // Python lists `elif` and `else` clauses as children of one `if_statement`.
        let clauses = node
            .named_children(&mut node.walk())
            .filter(|c| matches!(c.kind(), "elif_clause"))
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
        let control = CONTROL.contains(&node.kind());
        let depth = if control && !chained(node) {
            if matches!(
                node.kind(),
                "if_statement" | "if_expression" | "conditional_expression" | "ternary_expression"
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
