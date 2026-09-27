//! Bend 2, the language of bendlang/bend (2.0.x): pure, affine and
//! dependently typed, with Python-shaped `def`s, `type`s and `law`s. A law
//! states a claim that a `def` of the same name proves; Base also declares
//! its primitives and opaque handles as laws without one. A test is a
//! program whose file ends in the `#|` lines its run must print.
//!
//! Bend 1, HigherOrderCO's 2024 language, shares the `.bend` extension and
//! not the syntax, so its files are told apart before parsing and skipped.
use super::text;
use std::{collections::BTreeSet, path::Path};
use tree_sitter::Node;

/// The language's name in reports and in what Jev is asked, so a model
/// that knows Bend 1 does not read Bend 2 code as it.
pub(crate) const LANGUAGE: &str = "Bend 2";

pub(crate) fn file(path: &Path) -> bool {
    path.extension().is_some_and(|e| e == "bend")
}

/// Whether `.bend` source is Bend 1: it has no line only Bend 2 writes, and
/// a top-level line only Bend 1 writes (`main = …`, `(Sum Leaf) = 0`,
/// `data`, `object`, `from lib import f`, `import lib/a` without an alias,
/// `def main:` without parentheses, `type Tree:` without `is`), a
/// Bend 1 statement such as `fold t:` or `bend x = 0:`, or a name with a
/// slash, such as `List/Cons`. Bend 2 opens its top-level lines only with
/// `def`, `law`, `type … is`, `import`, `@`, a comment or the `)` closing a
/// long parameter list, spells its names with dots, and has none of those
/// statements; its own tests of refused syntax (`type Kk:`) import Base and
/// state laws. Checked before parsing: Bend 2's grammar took over ten
/// minutes on a Bend 1 test of 1 MB.
pub(crate) fn bend1(source: &str) -> bool {
    !bend2_only(source)
        && source.lines().any(|line| {
            let code = line.trim_end();
            let trimmed = code.trim_start();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                return false;
            }
            let top_level = trimmed.len() == code.len();
            (top_level && bend1_declaration(trimmed))
                || bend1_statement(trimmed)
                || slash_name(trimmed)
        })
}

/// A line only Bend 2 writes: `import Base`, a law, a datatype's kind, a
/// test's expected output, reflexivity or a `do IO<…>` block.
fn bend2_only(source: &str) -> bool {
    source.lines().any(|line| {
        let trimmed = line.trim();
        trimmed == "import Base"
            || line.starts_with("law ")
            || trimmed.starts_with("#|")
            || [" is Data", " is Type", " is Kind(", "{==}", "do IO<"]
                .iter()
                .any(|marker| trimmed.contains(marker))
    })
}

fn bend1_declaration(line: &str) -> bool {
    let first = line
        .split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '@'))
        .next()
        .unwrap_or("");
    match first {
        "law" => false,
        // Bend 2 imports a file with an alias (`import ./x.bend as X`), and
        // every def of it takes parentheses.
        "import" => line != "import Base" && !line.contains(" as "),
        "def" => !line.contains('('),
        // A long Bend 2 header names its kind on a later line.
        "type" => line.ends_with(':') && !line.contains(" is "),
        "data" | "object" | "from" | "hvm" => true,
        _ if line.starts_with('@') || line.starts_with(')') => false,
        // `(Main) = …`, `main = …` and `Foo a b = 0` define by equations.
        _ => line.starts_with('(') || !first.is_empty() && line.contains('='),
    }
}

/// A Bend 1 statement that opens a block: `bend x = 0:`, `fold t:`,
/// `switch n:`, `when c:`, `with IO:`, `if c:`, `elif c:` or `else:`.
fn bend1_statement(line: &str) -> bool {
    const OPENERS: [&str; 7] = [
        "bend ", "fold ", "switch ", "when ", "with ", "if ", "elif ",
    ];
    line.ends_with(':') && (line == "else:" || OPENERS.iter().any(|o| line.starts_with(o)))
}

/// A slash inside a name, as Bend 1 spells `List/Cons` and `IO/print`,
/// outside strings: Bend 2 divides only with spaces around the slash.
fn slash_name(line: &str) -> bool {
    if line.starts_with("import ") {
        return false;
    }
    let mut quoted = false;
    let bytes = line.as_bytes();
    for (at, &byte) in bytes.iter().enumerate() {
        match byte {
            b'"' => quoted = !quoted,
            b'#' if !quoted => return false,
            b'/' if !quoted && at > 0 => {
                let name = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
                if name(bytes[at - 1]) && bytes.get(at + 1).is_some_and(|&b| name(b)) {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

/// A project's `LAWS.bend` or `PROOF.bend`: by Bend 2's convention its laws
/// and their proofs live in these two files at its root, and `bend
/// PROOF.bend` checks them all, so they are not split into modules. Five
/// of 32 file-organization findings on twelve Bend 2 projects asked to.
pub(crate) fn law_file(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|name| name == "LAWS.bend" || name == "PROOF.bend")
}

/// A file of proofs: a `PROOF.bend`, a file named after what it proves
/// (`padding_proof.bend`, mylsm's `BloomSafeProof.bend`) or one under a
/// `proof` or `proofs` directory, as bend-collections keeps its lemmas.
/// Its defs are steps of proofs, and the comments of its laws say how a
/// proof goes: on the 16 projects where such files were first judged, 12
/// of 14 function-simplification findings in them were wrong and the rest
/// debatable, as were 14 of 15 law findings, and 36 of 40 hardcoded-value
/// findings were wrong.
pub(crate) fn proof_file(path: &Path) -> bool {
    let named = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().to_ascii_lowercase())
        .is_some_and(|stem| stem.ends_with("proof") || stem.ends_with("proofs"));
    let placed = path.parent().is_some_and(|dir| {
        dir.iter().any(|part| {
            let part = part.to_string_lossy().to_ascii_lowercase();
            part == "proof" || part == "proofs"
        })
    });
    file(path) && (named || placed)
}

/// The comment lines of a file that rule off a section: `# ----`,
/// `# === Title ===` or `# --- IO helpers`, three dashes or equals signs
/// or more after the `#`.
pub(crate) fn section_rules(source: &str) -> usize {
    source
        .lines()
        .filter_map(|line| line.trim_start().strip_prefix('#'))
        .filter(|text| {
            let text = text.trim_start();
            let rule = text.chars().take_while(|c| matches!(c, '-' | '=')).count();
            rule >= 3
        })
        .count()
}

/// Whether a Bend 2 file defines `main`, the def its run starts from.
pub(crate) fn defines_main(source: &str) -> bool {
    source.lines().any(|line| {
        line.strip_prefix("def main")
            .is_some_and(|rest| rest.starts_with(['(', ':', ' ']))
    })
}

/// The byte where a test's expected output starts: the `#|` lines that end
/// the file, which its run must print. Blank lines may follow them.
pub(crate) fn expected_output(source: &str) -> Option<usize> {
    let body = source.trim_end();
    let mut start = None;
    let mut end = body.len();
    loop {
        let line_start = body[..end].rfind('\n').map_or(0, |newline| newline + 1);
        if !body[line_start..end].trim_start().starts_with("#|") {
            return start;
        }
        start = Some(line_start);
        if line_start == 0 {
            return start;
        }
        end = line_start - 1;
    }
}

/// A comment line of a test's expected output, which is no prose to judge.
pub(crate) fn output_line(comment: &str) -> bool {
    comment.starts_with("#|")
}

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
        .filter(|c| !super::is_comment(*c))
        .collect();
    let witness = parts.iter().any(|c| c.kind() == "exists_clause");
    let (stated, before) = parts.split_last()?;
    let code = |node: Option<Node<'_>>| node.map_or(String::new(), |n| squeezed(text(n, source)));
    let clauses = before
        .iter()
        .filter_map(|clause| match clause.kind() {
            "for_clause" => {
                let kind = clause.child_by_field_name("type");
                Some(Clause::Every {
                    name: code(clause.child_by_field_name("name")),
                    // A hypothesis reads as the equality it assumes.
                    kind: match kind {
                        Some(k) if k.kind() == "equality_type" => words(k, source),
                        _ => code(kind),
                    },
                    equality: kind.is_some_and(holds_equality),
                    head: kind.and_then(|k| head_name(k, source)),
                    condition: clause
                        .child_by_field_name("condition")
                        .map(|c| squeezed(text(c, source))),
                })
            }
            "exists_clause" => Some(Clause::Some {
                name: code(clause.child_by_field_name("name")),
                kind: code(clause.child_by_field_name("type")),
            }),
            "let_statement" => Some(Clause::Let {
                pattern: code(clause.child_by_field_name("pattern")),
                value: code(clause.child_by_field_name("value")),
            }),
            _ => None,
        })
        .collect();
    Some(Statement {
        equality: witness || holds_equality(*stated),
        head: head_name(*stated, source),
        clauses,
        words: words(*stated, source),
    })
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

/// Whether a type holds an equality (`{a == b : T}`) or its negation, as a
/// pair of equalities or an implication ending in one does.
fn holds_equality(node: Node<'_>) -> bool {
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
fn head_name(node: Node<'_>, source: &str) -> Option<String> {
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

/// The alias of each module a file imports: `Sort` of `import ./main.bend as Sort`.
pub(crate) fn aliases(root: Node<'_>, source: &str) -> Vec<String> {
    let mut cursor = root.walk();
    root.named_children(&mut cursor)
        .filter(|c| c.kind() == "import_declaration")
        .filter_map(|c| c.child_by_field_name("alias"))
        .map(|alias| text(alias, source).to_string())
        .collect()
}

/// The file an import names, relative to the importing file: `./main.bend`
/// or `../lib/list.bend`. Base, hub packages (`0x…/main.bend`,
/// `name@version/main.bend`) and other paths name no file of the project.
pub(crate) fn imported_file(importer: &Path, path: &str) -> Option<std::path::PathBuf> {
    if !(path.starts_with("./") || path.starts_with("../")) {
        return None;
    }
    let mut resolved = importer.parent()?.to_path_buf();
    for part in path.split('/') {
        match part {
            "." | "" => {}
            ".." => {
                if !resolved.pop() {
                    return None;
                }
            }
            part => resolved.push(part),
        }
    }
    Some(resolved)
}

/// The import paths of a file's source, read from its lines.
pub(crate) fn import_paths(source: &str) -> impl Iterator<Item = &str> {
    source.lines().filter_map(|line| {
        let rest = line.strip_prefix("import ")?.trim();
        let path = rest.split_whitespace().next()?;
        Some(path)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bend_1_is_told_apart_from_bend_2() {
        let bend2 = "# LAW: sorted\nimport Base\nimport ./main.bend as Sort\n\ntype Shape is Data:\n  Circle{r: U32}\n\ndef Pong.inside(\n  +x0: U32, +y0: U32\n) -> Bool:\n  ((x0 <= y0) : U32)\n\n@unsafe def loop(n: Nat) -> Nat:\n  loop(n)\n\nlaw sort_sorted:\n  for +xs: List<&2, Nat>\n  Sort.Sorted(Sort.sort(xs))\n\ndef main() -> IO(Unit):\n  do IO<Unit>:\n    x : U32 = (8 / 2 : U32)\n    return Unit{}\n\n#|4\n";
        assert!(!bend1(bend2));
        for bend1_source in [
            "type MyTree(t):\n  Node { val: t }\n  Leaf\n\ndef main() -> u24:\n  return 1\n",
            "def sum(tree):\n  fold tree:\n    case MyTree/Node:\n      return tree.val\n",
            "main = (Sum (Gen 4))\n",
            "(Main) = λx x\n",
            "import lib/import_entry\n\ndef main():\n  return *\n",
            "def main:\n  y = 89\n",
            "data Tree = (Leaf a) | (Both a b)\n",
            "from lib import myFun\n",
            "def gen(d):\n  bend height=0:\n    when height < d:\n      x = 1\n",
            "def main():\n  with IO:\n    * <- IO/print(\"hi\")\n",
        ] {
            assert!(bend1(bend1_source), "{bend1_source}");
        }
        // Division needs spaces in Bend 2; a path or a comment is no name.
        assert!(!bend1("def half(x: U32) -> U32:\n  (x / 2 : U32) # a/b\n"));
        assert!(!bend1(
            "def e(x: U32) -> IO(U32):\n  import \"./e/f.c\"\n  import \"./e.js\"\n"
        ));
    }

    #[test]
    fn a_test_ends_in_its_expected_output() {
        let test = "import Base\n\ndef main() -> U32:\n  7\n\n#|7\n#|exit 0\n\n";
        let start = expected_output(test).unwrap();
        assert_eq!(&test[start..], "#|7\n#|exit 0\n\n");
        assert_eq!(expected_output("def main() -> U32:\n  7\n# 7\n"), None);
        assert_eq!(expected_output("#|only\n"), Some(0));
    }

    #[test]
    fn files_of_proofs_are_told_by_their_name_or_directory() {
        for path in [
            "PROOF.bend",
            "demos/sort/PROOF.bend",
            "padding_proof.bend",
            "proofs/BloomSafeProof.bend",
            "proofs/lib/lemmas/map.bend",
            "src/proof/nat.bend",
        ] {
            assert!(proof_file(Path::new(path)), "{path}");
        }
        for path in [
            "LAWS.bend",
            "src/proofreader.bend",
            "spec/containers/lru.bend",
            "proofs/notes.md",
        ] {
            assert!(!proof_file(Path::new(path)), "{path}");
        }
    }

    #[test]
    fn imports_resolve_relative_to_the_importer() {
        let importer = Path::new("demos/sort/PROOF.bend");
        assert_eq!(
            imported_file(importer, "./LAWS.bend").unwrap(),
            Path::new("demos/sort/LAWS.bend")
        );
        assert_eq!(
            imported_file(importer, "../lib/list.bend").unwrap(),
            Path::new("demos/lib/list.bend")
        );
        assert_eq!(imported_file(importer, "Base"), None);
        assert_eq!(imported_file(importer, "0xabc/main.bend"), None);
        let paths: Vec<&str> =
            import_paths("import Base\nimport ./main.bend as M\n  import \"./e.c\"\n").collect();
        assert_eq!(paths, ["Base", "./main.bend"]);
    }
}
