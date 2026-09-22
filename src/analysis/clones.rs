//! Type-2 clone candidates across the selected files and explicit context.
//! Identifiers and literals are normalized; windows start and end on whole
//! statements inside function bodies; identifiers must be renamed consistently.
use super::{is_comment, line_of, text, units::Unit};
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range,
    path::{Path, PathBuf},
};
use tree_sitter::Node;

pub const MIN_BYTES: usize = 120;
pub const MIN_STATEMENTS: usize = 2;
pub const RUN_CAP: usize = 64;
pub const FILE_CAP: usize = 8;
const DIFFERENCES: usize = 12;
/// A statement pair repeated more often than this is an idiom; its extra pairs are not compared.
const SEED_OCCURRENCES: usize = 48;

pub struct SourceFile<'a> {
    pub path: &'a Path,
    pub source: &'a str,
    /// False for explicit context: a pair needs at least one selected site.
    pub selected: bool,
    pub units: &'a [Unit],
    /// Lines excluded from comparison, such as test code when tests are not judged.
    pub excluded: &'a [Range<usize>],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Site {
    pub file: usize,
    pub path: PathBuf,
    pub span: Range<usize>,
    pub start_line: usize,
    pub end_line: usize,
    /// Enclosing function or method, when there is one.
    pub function: Option<String>,
    pub function_source: Option<String>,
    pub quote: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Difference {
    pub a: String,
    pub b: String,
}

#[derive(Clone, Debug)]
pub struct Pair {
    pub a: Site,
    pub b: Site,
    pub differences: Vec<Difference>,
    /// Non-whitespace bytes of the shorter site.
    pub size: usize,
    /// Distinct sites that repeat the same normalized statements.
    pub occurrences: usize,
    /// Hash of the normalized statements; stable across renames and moves.
    pub normalized: String,
}

impl Pair {
    pub fn rank(&self) -> usize {
        self.size * self.occurrences
    }
}

#[derive(Default)]
pub struct Candidates {
    pub pairs: Vec<Pair>,
    /// Pairs dropped by the per-run or per-file caps, by owning path.
    pub omitted: BTreeMap<PathBuf, usize>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TokenKind {
    Identifier,
    Literal,
    Other,
}

struct Token<'a> {
    kind: TokenKind,
    text: &'a str,
    start: usize,
}

impl Token<'_> {
    fn normal(&self) -> &str {
        match self.kind {
            TokenKind::Identifier => "\u{1}id",
            TokenKind::Literal => "\u{1}lit",
            TokenKind::Other => self.text,
        }
    }
}

struct Statement {
    span: Range<usize>,
    tokens: Range<usize>,
    hash: u64,
}

struct Block {
    file: usize,
    statements: Vec<Statement>,
}

struct Parsed<'a> {
    tokens: Vec<Token<'a>>,
}

pub fn find(files: &[SourceFile<'_>]) -> Candidates {
    let mut parsed = Vec::new();
    let mut blocks = Vec::new();
    for (index, file) in files.iter().enumerate() {
        let Ok(Some(tree)) = crate::locations::parse(file.path, file.source) else {
            parsed.push(Parsed { tokens: Vec::new() });
            continue;
        };
        let mut tokens = Vec::new();
        leaves(tree.root_node(), file.source, &mut tokens);
        let bodies: Vec<Range<usize>> = file.units.iter().filter_map(|u| u.body.clone()).collect();
        collect_blocks(tree.root_node(), file, index, &bodies, &tokens, &mut blocks);
        parsed.push(Parsed { tokens });
    }
    let mut seeds = BTreeMap::<(u64, u64), Vec<(usize, usize)>>::new();
    for (b, block) in blocks.iter().enumerate() {
        for k in 0..block.statements.len().saturating_sub(1) {
            let key = (block.statements[k].hash, block.statements[k + 1].hash);
            seeds.entry(key).or_default().push((b, k));
        }
    }
    let mut covered = BTreeSet::new();
    let mut found = Vec::new();
    for occurrences in seeds.values() {
        let occurrences = &occurrences[..occurrences.len().min(SEED_OCCURRENCES)];
        for (x, &(bx, kx)) in occurrences.iter().enumerate() {
            for &(by, ky) in &occurrences[x + 1..] {
                if bx == by && ky < kx + MIN_STATEMENTS {
                    continue;
                }
                let diagonal = (bx, by, kx as isize - ky as isize);
                if covered.contains(&(diagonal, kx)) {
                    continue;
                }
                let (sx, sy) = (&blocks[bx].statements, &blocks[by].statements);
                let mut n = MIN_STATEMENTS;
                while kx + n < sx.len()
                    && ky + n < sy.len()
                    && sx[kx + n].hash == sy[ky + n].hash
                    && (bx != by || kx + n < ky)
                {
                    n += 1;
                }
                for t in 0..n {
                    covered.insert((diagonal, kx + t));
                }
                found.push(((bx, kx), (by, ky), n));
            }
        }
    }
    let mut pairs = Vec::new();
    for ((bx, kx), (by, ky), n) in found {
        let (fx, fy) = (blocks[bx].file, blocks[by].file);
        if !files[fx].selected && !files[fy].selected {
            continue;
        }
        let x = &blocks[bx].statements[kx..kx + n];
        let y = &blocks[by].statements[ky..ky + n];
        let tx = &parsed[fx].tokens[x[0].tokens.start..x[n - 1].tokens.end];
        let ty = &parsed[fy].tokens[y[0].tokens.start..y[n - 1].tokens.end];
        let Some(differences) = align(tx, ty) else {
            continue;
        };
        let span_x = x[0].span.start..x[n - 1].span.end;
        let span_y = y[0].span.start..y[n - 1].span.end;
        let size = compact(&files[fx].source[span_x.clone()])
            .min(compact(&files[fy].source[span_y.clone()]));
        if size < MIN_BYTES {
            continue;
        }
        let normalized = crate::schema::hash(
            tx.iter()
                .map(Token::normal)
                .collect::<Vec<_>>()
                .join("\u{0}")
                .as_bytes(),
        );
        let a = site(files, fx, span_x);
        let b = site(files, fy, span_y);
        // The owner is a selected site; ties keep path and line order.
        let (a, b, differences) = if !files[fx].selected
            || (files[fy].selected && (&b.path, b.start_line) < (&a.path, a.start_line))
        {
            (
                b,
                a,
                differences
                    .into_iter()
                    .map(|d| Difference { a: d.b, b: d.a })
                    .collect(),
            )
        } else {
            (a, b, differences)
        };
        pairs.push(Pair {
            a,
            b,
            differences,
            size,
            occurrences: 0,
            normalized,
        });
    }
    // Drop pairs whose sites both lie inside a larger pair's sites.
    let snapshot = pairs.clone();
    pairs.retain(|p| {
        !snapshot.iter().any(|q| {
            let larger = q.a.span.len() + q.b.span.len() > p.a.span.len() + p.b.span.len();
            larger
                && ((contains(&q.a, &p.a) && contains(&q.b, &p.b))
                    || (contains(&q.a, &p.b) && contains(&q.b, &p.a)))
        })
    });
    let mut sites = BTreeMap::<&str, BTreeSet<(PathBuf, usize)>>::new();
    for pair in &pairs {
        let entry = sites.entry(pair.normalized.as_str()).or_default();
        entry.insert((pair.a.path.clone(), pair.a.span.start));
        entry.insert((pair.b.path.clone(), pair.b.span.start));
    }
    let occurrences: BTreeMap<String, usize> = sites
        .into_iter()
        .map(|(key, set)| (key.to_string(), set.len()))
        .collect();
    for pair in &mut pairs {
        pair.occurrences = occurrences[&pair.normalized];
    }
    // One representative per repeated site: pair each copy with the first one.
    let mut first = BTreeMap::<String, (PathBuf, usize)>::new();
    for pair in &pairs {
        let key = (pair.a.path.clone(), pair.a.span.start);
        first
            .entry(pair.normalized.clone())
            .and_modify(|current| {
                if key < *current {
                    *current = key.clone();
                }
            })
            .or_insert(key);
    }
    let mut omitted = BTreeMap::<PathBuf, usize>::new();
    pairs.retain(|p| {
        let anchor = &first[&p.normalized];
        (&p.a.path, p.a.span.start) == (&anchor.0, anchor.1)
            || (&p.b.path, p.b.span.start) == (&anchor.0, anchor.1)
    });
    pairs.sort_by(|p, q| {
        q.rank()
            .cmp(&p.rank())
            .then_with(|| (&p.a.path, p.a.start_line).cmp(&(&q.a.path, q.a.start_line)))
            .then_with(|| (&p.b.path, p.b.start_line).cmp(&(&q.b.path, q.b.start_line)))
    });
    let mut per_file = BTreeMap::<PathBuf, usize>::new();
    let mut kept = Vec::new();
    for pair in pairs {
        let count = per_file.entry(pair.a.path.clone()).or_default();
        if kept.len() < RUN_CAP && *count < FILE_CAP {
            *count += 1;
            kept.push(pair);
        } else {
            *omitted.entry(pair.a.path.clone()).or_default() += 1;
        }
    }
    Candidates {
        pairs: kept,
        omitted,
    }
}

fn contains(outer: &Site, inner: &Site) -> bool {
    outer.path == inner.path
        && outer.span.start <= inner.span.start
        && inner.span.end <= outer.span.end
}

fn compact(text: &str) -> usize {
    text.bytes().filter(|b| !b.is_ascii_whitespace()).count()
}

fn site(files: &[SourceFile<'_>], index: usize, span: Range<usize>) -> Site {
    let file = &files[index];
    let unit = file
        .units
        .iter()
        .filter(|u| u.callable() && u.span.start <= span.start && span.end <= u.span.end)
        .min_by_key(|u| u.span.len());
    Site {
        file: index,
        path: file.path.to_path_buf(),
        start_line: line_of(file.source, span.start),
        end_line: line_of(file.source, span.end.saturating_sub(1)),
        function: unit.map(|u| u.name.clone()),
        function_source: unit.map(|u| u.source(file.source).to_string()),
        quote: file.source[span.clone()].to_string(),
        span,
    }
}

/// Aligned tokens must match after normalization, and each identifier must map
/// to exactly one identifier on the other side. Returns renamed names and values.
fn align(x: &[Token<'_>], y: &[Token<'_>]) -> Option<Vec<Difference>> {
    if x.len() != y.len() {
        return None;
    }
    let mut forward = BTreeMap::new();
    let mut backward = BTreeMap::new();
    let mut differences = Vec::new();
    for (a, b) in x.iter().zip(y) {
        if a.normal() != b.normal() {
            return None;
        }
        if a.kind == TokenKind::Identifier
            && (*forward.entry(a.text).or_insert(b.text) != b.text
                || *backward.entry(b.text).or_insert(a.text) != a.text)
        {
            return None;
        }
        if a.kind != TokenKind::Other && a.text != b.text {
            let difference = Difference {
                a: a.text.to_string(),
                b: b.text.to_string(),
            };
            if !differences.contains(&difference) && differences.len() < DIFFERENCES {
                differences.push(difference);
            }
        }
    }
    Some(differences)
}

fn leaves<'a>(node: Node<'_>, source: &'a str, tokens: &mut Vec<Token<'a>>) {
    if is_comment(node) {
        return;
    }
    let kind = node.kind();
    let literal = matches!(
        kind,
        "string_content"
            | "string_fragment"
            | "integer_literal"
            | "float_literal"
            | "char_literal"
            | "number"
            | "integer"
            | "float"
    );
    if node.child_count() == 0 || literal {
        let text = text(node, source);
        if text.trim().is_empty() {
            return;
        }
        let kind = if literal {
            TokenKind::Literal
        } else if kind.ends_with("identifier") || kind == "identifier" {
            TokenKind::Identifier
        } else {
            TokenKind::Other
        };
        tokens.push(Token {
            kind,
            text,
            start: node.start_byte(),
        });
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        leaves(child, source, tokens);
    }
}

fn collect_blocks(
    node: Node<'_>,
    file: &SourceFile<'_>,
    index: usize,
    bodies: &[Range<usize>],
    tokens: &[Token<'_>],
    blocks: &mut Vec<Block>,
) {
    if matches!(node.kind(), "block" | "statement_block")
        && bodies
            .iter()
            .any(|b| b.start <= node.start_byte() && node.end_byte() <= b.end)
    {
        let mut statements = Vec::new();
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            if is_comment(child) {
                continue;
            }
            let line = line_of(file.source, child.start_byte());
            if file.excluded.iter().any(|r| r.contains(&line)) {
                statements.push(None);
                continue;
            }
            let start = tokens.partition_point(|t| t.start < child.start_byte());
            let end = tokens.partition_point(|t| t.start < child.end_byte());
            let key = tokens[start..end]
                .iter()
                .map(Token::normal)
                .collect::<Vec<_>>()
                .join("\u{0}");
            statements.push(Some(Statement {
                span: child.byte_range(),
                tokens: start..end,
                hash: fast_hash(&key),
            }));
        }
        // Excluded statements break a window, so split the block there.
        let mut current = Vec::new();
        for statement in statements {
            match statement {
                Some(statement) => current.push(statement),
                None if !current.is_empty() => blocks.push(Block {
                    file: index,
                    statements: std::mem::take(&mut current),
                }),
                None => {}
            }
        }
        if !current.is_empty() {
            blocks.push(Block {
                file: index,
                statements: current,
            });
        }
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_blocks(child, file, index, bodies, tokens, blocks);
    }
}

fn fast_hash(text: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(files: &[(&str, &str, bool)]) -> Candidates {
        let units: Vec<_> = files
            .iter()
            .map(|(path, source, _)| super::super::units::parse(Path::new(path), source).unwrap())
            .collect();
        let sources: Vec<_> = files
            .iter()
            .zip(&units)
            .map(|((path, source, selected), units)| SourceFile {
                path: Path::new(path),
                source,
                selected: *selected,
                units: &units.units,
                excluded: &[],
            })
            .collect();
        find(&sources)
    }

    const LOAD: &str = "fn load_user(path: &str) -> Result<User> {\n    let text = std::fs::read_to_string(path)?;\n    let value: Value = serde_json::from_str(&text)?;\n    let name = value[\"name\"].as_str().unwrap_or(\"anonymous\").trim().to_string();\n    Ok(User { name })\n}\n";

    #[test]
    fn renamed_copies_match_across_files_with_statement_aligned_quotes() {
        let renamed = LOAD
            .replace("load_user", "load_team")
            .replace("text", "body")
            .replace("value", "parsed")
            .replace("\"name\"", "\"title\"")
            .replace("User", "Team");
        let found = run(&[("a.rs", LOAD, true), ("b.rs", &renamed, true)]);
        assert_eq!(found.pairs.len(), 1, "{:?}", found.pairs.len());
        let pair = &found.pairs[0];
        assert_eq!(pair.a.path, Path::new("a.rs"));
        assert_eq!(pair.b.path, Path::new("b.rs"));
        assert_eq!(pair.a.function.as_deref(), Some("load_user"));
        assert_eq!(pair.b.function.as_deref(), Some("load_team"));
        assert!(
            pair.a
                .quote
                .starts_with("let text = std::fs::read_to_string")
        );
        assert!(pair.a.quote.ends_with("Ok(User { name })"));
        assert_eq!((pair.a.start_line, pair.a.end_line), (2, 5));
        assert!(pair.differences.contains(&Difference {
            a: "text".into(),
            b: "body".into()
        }));
        assert!(pair.differences.contains(&Difference {
            a: "name".into(),
            b: "title".into()
        }));
        assert_eq!(pair.occurrences, 2);
    }

    #[test]
    fn inconsistent_renaming_and_short_windows_are_rejected() {
        // `text` becomes two different names on the other side.
        let inconsistent = LOAD
            .replace("let text", "let body")
            .replace("from_str(&text)", "from_str(&other)");
        assert!(
            run(&[("a.rs", LOAD, true), ("b.rs", &inconsistent, true)])
                .pairs
                .is_empty()
        );
        let short = "fn a(x: i32) -> i32 {\n    let y = x + 1;\n    y * 2\n}\nfn b(x: i32) -> i32 {\n    let y = x + 1;\n    y * 2\n}\n";
        assert!(run(&[("s.rs", short, true)]).pairs.is_empty());
    }

    #[test]
    fn context_only_pairs_are_excluded_but_selected_to_context_pairs_are_kept() {
        let copy = LOAD.replace("load_user", "load_again");
        assert!(
            run(&[("a.rs", LOAD, false), ("b.rs", &copy, false)])
                .pairs
                .is_empty()
        );
        let found = run(&[("context.rs", LOAD, false), ("selected.rs", &copy, true)]);
        assert_eq!(found.pairs.len(), 1);
        assert_eq!(found.pairs[0].a.path, Path::new("selected.rs"));
        assert_eq!(found.pairs[0].b.path, Path::new("context.rs"));
    }

    #[test]
    fn repeated_copies_in_one_file_count_occurrences_and_keep_one_anchor() {
        let three = [
            LOAD,
            &LOAD.replace("load_user", "second"),
            &LOAD.replace("load_user", "third"),
        ]
        .concat();
        let found = run(&[("three.rs", &three, true)]);
        assert_eq!(found.pairs.len(), 2);
        assert!(
            found
                .pairs
                .iter()
                .all(|p| p.occurrences == 3 && p.a.start_line == 2)
        );
        let body = "    total = decimal.Decimal(\"0\")\n    for row in rows:\n        total += row.amount * row.exchange_rate - row.discount_amount\n    return total.quantize(decimal.Decimal(\"0.01\"), rounding=decimal.ROUND_HALF_UP)\n";
        let python = format!(
            "def a(rows):\n{body}\ndef b(items):\n{}",
            body.replace("row", "item")
        );
        let found = run(&[("totals.py", &python, true)]);
        assert_eq!(found.pairs.len(), 1);
        assert_eq!(found.pairs[0].a.function.as_deref(), Some("a"));
    }
}
