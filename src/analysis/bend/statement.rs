//! A law's statement: whether it is a claim a proof must hold, what it
//! quantifies over, and how it reads in words.
use super::defs::{head_name, holds_equality};
use crate::analysis::{is_comment, text};
use std::collections::BTreeSet;
use tree_sitter::Node;

/// What a law states. A law is a claim when it states an equality or its
/// negation, asks for a witness, or applies a def that computes a type
/// (`Sorted(sort(xs))`, with `def Sorted(xs) -> Type`): a proof must hold
/// it. Otherwise its statement is an ordinary type, and the law declares a
/// signature that a def of its name fills (`law main: U32`) or a postulate,
/// such as Base's opaque handles and native arithmetic.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Statement {
    /// An equality, its negation or a witness is stated: a claim.
    pub equality: bool,
    /// The name the statement applies: `Sort.Sorted` of `Sort.Sorted(Sort.sort(xs))`.
    pub head: Option<String>,
    /// Its `for` and `exs` clauses and the values it binds, in order.
    pub clauses: Vec<Clause>,
    /// The statement in words, its terms as code: `a == b`, `A, and B`, `if A, then B`.
    pub words: String,
}

/// One clause of a law, before its statement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Clause {
    /// `for x: T` or `for x: T where P`: every `x`, or with a proposition
    /// as `T`, a hypothesis.
    Every {
        name: String,
        kind: String,
        /// `T` states an equality, or applies `head`.
        equality: bool,
        head: Option<String>,
        condition: Option<String>,
    },
    /// `exs y: T`: a witness.
    Some { name: String, kind: String },
    /// `x = v`: a value the statement names.
    Let { pattern: String, value: String },
}

fn proposition(head: Option<&str>, propositions: &BTreeSet<String>) -> bool {
    head.is_some_and(|head| {
        propositions.contains(head)
            || head
                .split_once('.')
                .is_some_and(|(_, rest)| propositions.contains(rest))
    })
}

impl Statement {
    /// Whether the law is a claim, given the defs that compute a type, by
    /// their names as their files declare them.
    pub(crate) fn claim(&self, propositions: &BTreeSet<String>) -> bool {
        self.equality || proposition(self.head.as_deref(), propositions)
    }

    /// Whether the law quantifies over something: one without `for` or `exs`
    /// states a fact about fixed values, a spot check such as
    /// `{name(code("main")) == "main" : String}`.
    pub(crate) fn general(&self) -> bool {
        self.clauses
            .iter()
            .any(|c| matches!(c, Clause::Every { .. } | Clause::Some { .. }))
    }

    /// The law in words: "for every x: T", "assuming P" for a clause that
    /// names a proof of a proposition, "there is some y: T such that", "with
    /// x = v", then the statement.
    pub(crate) fn reading(&self, propositions: &BTreeSet<String>) -> String {
        let mut parts: Vec<String> = Vec::new();
        let mut witness = false;
        for clause in &self.clauses {
            let part = match clause {
                Clause::Every {
                    kind,
                    equality,
                    head,
                    ..
                } if *equality || proposition(head.as_deref(), propositions) => {
                    format!("assuming {kind}")
                }
                Clause::Every {
                    name,
                    kind,
                    condition,
                    ..
                } => match condition {
                    Some(condition) => format!("for every {name}: {kind} with {condition}"),
                    None => format!("for every {name}: {kind}"),
                },
                Clause::Some { name, kind } => format!("there is some {name}: {kind}"),
                Clause::Let { pattern, value } => format!("with {pattern} = {value}"),
            };
            witness = matches!(clause, Clause::Some { .. });
            parts.push(part);
        }
        let words = &self.words;
        match parts.len() {
            0 => format!("{words}."),
            _ if witness => format!("{} such that {words}.", parts.join(", ")),
            _ => format!("{}: {words}.", parts.join(", ")),
        }
    }
}

/// What a `law_declaration` states: its last part after the `for` and `exs`
/// clauses.
pub(crate) fn statement(node: Node<'_>, source: &str) -> Option<Statement> {
    if node.kind() != "law_declaration" {
        return None;
    }
    let body = node.child_by_field_name("body")?;
    let mut cursor = body.walk();
    let parts: Vec<Node<'_>> = body
        .named_children(&mut cursor)
        .filter(|c| !is_comment(*c))
        .collect();
    let witness = parts.iter().any(|c| c.kind() == "exists_clause");
    let (stated, before) = parts.split_last()?;
    let clauses = before.iter().filter_map(|c| clause(*c, source)).collect();
    Some(Statement {
        equality: witness || holds_equality(*stated),
        head: head_name(*stated, source),
        clauses,
        words: words(*stated, source),
    })
}

/// One clause before a law's statement: `for`, `exists` or `let`.
fn clause(clause: Node<'_>, source: &str) -> Option<Clause> {
    let code = |node: Option<Node<'_>>| node.map_or(String::new(), |n| squeezed(text(n, source)));
    let field = |name: &str| clause.child_by_field_name(name);
    match clause.kind() {
        "for_clause" => {
            let kind = field("type");
            Some(Clause::Every {
                name: code(field("name")),
                // A hypothesis reads as the equality it assumes.
                kind: match kind {
                    Some(k) if k.kind() == "equality_type" => words(k, source),
                    _ => code(kind),
                },
                equality: kind.is_some_and(holds_equality),
                head: kind.and_then(|k| head_name(k, source)),
                condition: field("condition").map(|c| squeezed(text(c, source))),
            })
        }
        "exists_clause" => Some(Clause::Some {
            name: code(field("name")),
            kind: code(field("type")),
        }),
        "let_statement" => Some(Clause::Let {
            pattern: code(field("pattern")),
            value: code(field("value")),
        }),
        _ => None,
    }
}

/// A statement in words, its terms as code.
fn words(node: Node<'_>, source: &str) -> String {
    let field = |name: &str| node.child_by_field_name(name);
    let part = |name: &str| field(name).map_or(String::new(), |n| words(n, source));
    match node.kind() {
        "equality_type" => {
            let operator = field("operator").map_or("==", |o| text(o, source));
            let side =
                |name: &str| field(name).map_or(String::new(), |n| squeezed(text(n, source)));
            format!("{} {operator} {}", side("left"), side("right"))
        }
        "parenthesized_expression" if node.named_child_count() == 1 => node
            .named_child(0)
            .map_or(String::new(), |inner| words(inner, source)),
        "binary_expression" => match field("operator").map(|o| text(o, source)) {
            Some("&") => format!("{}, and {}", part("left"), part("right")),
            Some("|") => format!("{}, or {}", part("left"), part("right")),
            _ => squeezed(text(node, source)),
        },
        "function_type" => format!("if {}, then {}", part("parameter"), part("result")),
        "dependent_function_type" | "exists_type" => {
            let name = field("name").map_or("", |n| text(n, source));
            let kind = field("parameter").map_or(String::new(), |n| squeezed(text(n, source)));
            let quantifier = if node.kind() == "exists_type" {
                "there is some"
            } else {
                "for every"
            };
            let joined = if node.kind() == "exists_type" {
                "such that"
            } else {
                ","
            };
            format!("{quantifier} {name}: {kind} {joined} {}", part("result"))
        }
        _ => format!("{} holds", squeezed(text(node, source))),
    }
}

/// Code on one line, its runs of spaces and line breaks as one space.
fn squeezed(code: &str) -> String {
    code.split_whitespace().collect::<Vec<_>>().join(" ")
}
