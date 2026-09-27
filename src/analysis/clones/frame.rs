//! The frame of a tree walk: the calls a function makes of itself and the
//! branches that only leave, which every walk repeats whatever work it does.
use super::*;

/// The frame of a tree walk: the statements every walk has, whatever it
/// does at each node.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Frame {
    /// A branch that only leaves, as `if node.kind() == "call" { return; }`.
    Exit,
    /// A call of the function itself and nothing else, or a loop or branch
    /// that only does that or leaves, as
    /// `for child in node.named_children(&mut cursor) { bound(child, names); }`.
    Recursion,
}

/// Objects a method calls itself on, as in `self.walk(`, `this.walk(`,
/// `Self::walk(`, `cls.walk(` or PHP's `$this->walk(` and `static::walk(`.
pub(super) const RECEIVERS: [&str; 5] = ["self", "Self", "this", "cls", "static"];

/// Mark each call of the function it sits in: its name followed by its
/// arguments inside the body of the innermost callable of that name. A
/// method calls itself on the object itself (`self.walk(`, `this.walk(`,
/// `Self::walk(`); a bare `walk(` inside it calls a free or imported
/// function, except in Java, C# and Ruby, where a bare call reaches the
/// method through its object. A call through another path, as
/// `native::get()` inside `get`, names a different function.
pub(super) fn mark_recursion(tokens: &mut [Token<'_>], units: &[Unit], path: &Path) {
    let implicit = matches!(
        path.extension().and_then(|x| x.to_str()),
        Some("java" | "cs" | "rb")
    );
    let callables: Vec<&Unit> = units.iter().filter(|u| u.callable()).collect();
    let names: BTreeSet<&str> = callables.iter().map(|u| u.short_name.as_str()).collect();
    for i in 1..tokens.len() {
        let token = &tokens[i - 1];
        if token.kind != TokenKind::Identifier
            || tokens[i].text != "("
            || !names.contains(token.text)
        {
            continue;
        }
        let Some(unit) = callables
            .iter()
            .filter(|u| u.body.as_ref().is_some_and(|b| b.contains(&token.start)))
            .min_by_key(|u| u.span.len())
            .filter(|u| u.short_name == token.text)
        else {
            continue;
        };
        let qualifier = i
            .checked_sub(2)
            .filter(|&q| matches!(tokens[q].text, "." | "::" | "->" | "?."));
        tokens[i - 1].own = match qualifier {
            None => implicit || unit.kind == Kind::Function,
            Some(q) => q
                .checked_sub(1)
                .is_some_and(|o| RECEIVERS.contains(&tokens[o].text)),
        };
    }
}

/// The tokens of one node.
pub(super) fn tokens_of<'t, 'a>(tokens: &'t [Token<'a>], node: Node<'_>) -> &'t [Token<'a>] {
    let start = tokens.partition_point(|t| t.start < node.start_byte());
    let end = tokens.partition_point(|t| t.start < node.end_byte());
    &tokens[start..end]
}

/// Part of a walk's frame rather than its work, if it is.
pub(super) fn frame(statement: Node<'_>, tokens: &[Token<'_>]) -> Option<Frame> {
    if exit_guard(statement, tokens) {
        Some(Frame::Exit)
    } else if recursion(statement, tokens) {
        Some(Frame::Recursion)
    } else {
        None
    }
}

/// A call of the function itself and nothing else, as `bound(child, names);`
/// or `return self.walk(node.parent)`, or a loop or branch whose statements
/// only do that or leave early, with at least one call. Work anywhere else
/// in its body, as in a match arm, a conditional expression or a call that
/// wraps the recursion, makes it more than the frame.
pub(super) fn recursion(statement: Node<'_>, tokens: &[Token<'_>]) -> bool {
    if own_call(statement, tokens) {
        return true;
    }
    let node = expression(statement);
    let ruby_block = node.kind() == "call" && node.child_by_field_name("block").is_some();
    if !ruby_block
        && !matches!(
            node.kind(),
            "for_expression"
                | "for_statement"
                | "for_in_statement"
                | "enhanced_for_statement"
                | "foreach_statement"
                | "for"
                | "while_expression"
                | "while_statement"
                | "while"
                | "until"
                | "loop_expression"
                | "do_statement"
                | "if_expression"
                | "if_statement"
                | "if"
                | "unless"
        )
    {
        return false;
    }
    let mut body = Vec::new();
    branch_statements(node, &mut body);
    body.iter().any(|&s| recursion(s, tokens))
        && body
            .iter()
            .all(|&s| exits(s, tokens) || exit_guard(s, tokens) || recursion(s, tokens))
}

/// A statement that is only a call of the function it sits in, as
/// `walk(child);`, `return self.walk(node.parent)`, `await this.walk(child);`
/// or `walk(child)?;`.
pub(super) fn own_call(statement: Node<'_>, tokens: &[Token<'_>]) -> bool {
    let mut words = tokens_of(tokens, statement);
    if let [first, rest @ ..] = words
        && matches!(first.text, "return" | "await")
    {
        words = rest;
    }
    while let [rest @ .., last] = words
        && matches!(last.text, ";" | "?")
    {
        words = rest;
    }
    if let [receiver, separator, rest @ ..] = words
        && RECEIVERS.contains(&receiver.text)
        && matches!(separator.text, "." | "::" | "->" | "?.")
    {
        words = rest;
    }
    let [name, arguments @ ..] = words else {
        return false;
    };
    if !name.own || arguments.first().is_none_or(|t| t.text != "(") {
        return false;
    }
    // The call's closing parenthesis ends the statement.
    let mut depth = 0usize;
    for (i, token) in arguments.iter().enumerate() {
        match token.text {
            "(" => depth += 1,
            ")" => {
                depth -= 1;
                if depth == 0 {
                    return i + 1 == arguments.len();
                }
            }
            _ => {}
        }
    }
    false
}

/// A branch that only leaves, as `if node.kind() == "call" { return; }`,
/// `if (done) return;` or `return if done`.
pub(super) fn exit_guard(statement: Node<'_>, tokens: &[Token<'_>]) -> bool {
    let node = expression(statement);
    if !matches!(
        node.kind(),
        "if_expression" | "if_statement" | "if" | "unless" | "if_modifier" | "unless_modifier"
    ) {
        return false;
    }
    let mut body = Vec::new();
    branch_statements(node, &mut body);
    !body.is_empty()
        && body
            .into_iter()
            .all(|s| exits(s, tokens) || exit_guard(s, tokens))
}

/// The expression a Rust or JavaScript statement wraps, as the `for` loop
/// of a Rust `expression_statement`.
pub(super) fn expression(statement: Node<'_>) -> Node<'_> {
    statement
        .named_child(0)
        .filter(|_| {
            statement.kind() == "expression_statement" && statement.named_child_count() == 1
        })
        .unwrap_or(statement)
}

/// The statements a loop or branch runs, in all its branches: the
/// statements of its blocks, or the one statement of a branch without
/// braces. Its header, as a condition or the collection a loop walks, is
/// not a statement.
pub(super) fn branch_statements<'t>(node: Node<'t>, statements: &mut Vec<Node<'t>>) {
    let before = statements.len();
    let mut cursor = node.walk();
    for field in ["body", "consequence", "alternative", "block"] {
        for part in node.children_by_field_name(field, &mut cursor) {
            statements_in(part, statements);
        }
    }
    // A JavaScript or Rust `else` holds its statement or block without a field.
    if statements.len() == before && node.kind() == "else_clause" {
        let mut cursor = node.walk();
        for part in node.named_children(&mut cursor) {
            statements_in(part, statements);
        }
    }
}

/// The statements of one part of a loop or branch.
pub(super) fn statements_in<'t>(part: Node<'t>, statements: &mut Vec<Node<'t>>) {
    if is_comment(part) {
        return;
    }
    if holds_statements(part) {
        let mut cursor = part.walk();
        for child in part.named_children(&mut cursor) {
            statements_in_block(child, statements);
        }
    } else if matches!(
        part.kind(),
        "else_clause" | "elif_clause" | "else_if_clause" | "elsif" | "block" | "do_block"
    ) {
        // Further branches, and the `{ … }` or `do … end` a Ruby call runs.
        branch_statements(part, statements);
    } else {
        statements.push(part);
    }
}

/// One statement of a block; Go holds a block's statements in a list.
pub(super) fn statements_in_block<'t>(child: Node<'t>, statements: &mut Vec<Node<'t>>) {
    if is_comment(child) {
        return;
    }
    if holds_statements(child) {
        statements_in(child, statements);
    } else {
        statements.push(child);
    }
}

/// `return`, `break`, `continue` or `next` with no value, or with nothing.
pub(super) fn exits(statement: Node<'_>, tokens: &[Token<'_>]) -> bool {
    match tokens_of(tokens, statement) {
        [first, rest @ ..] => {
            matches!(first.text, "return" | "break" | "continue" | "next")
                && rest
                    .iter()
                    .all(|t| matches!(t.text, ";" | "None" | "nil" | "null"))
        }
        [] => false,
    }
}
