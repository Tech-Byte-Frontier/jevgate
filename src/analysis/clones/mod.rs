//! Type-2 clone candidates across the selected files and explicit context.
//! Identifiers and literals are normalized; windows start and end on whole
//! statements inside function bodies; identifiers must be renamed consistently.
//! `apart` holds the copies that are never compared and `frame` the statements
//! every tree walk repeats, which do not make a copy on their own.
use super::{
    fast_hash, generic, is_comment, line_of, text,
    units::{Kind, Unit},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range,
    path::{Path, PathBuf},
};
use tree_sitter::Node;

mod apart;
mod frame;
#[cfg(test)]
mod tests;
use apart::*;
pub(crate) use apart::{benchmark_code, example_code};
use frame::*;

pub const MIN_BYTES: usize = 120;
/// Consecutive matching statements that seed a candidate window.
pub const MIN_STATEMENTS: usize = 2;
/// Statements a reported copy needs.
pub const MIN_CLONE_STATEMENTS: usize = 3;
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
    pub excluded: Vec<Range<usize>>,
    /// The package the file belongs to; explicit context has none.
    pub package: Option<&'a crate::packages::Package>,
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
    /// Distinct sites in this pair's clone group, including `a` and `b`.
    pub occurrences: usize,
    /// The group's other copies, reported with the judged pair.
    pub copies: Vec<Site>,
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
    /// A call of the function the token sits in: `bound(child, …)` inside
    /// `bound`. Recursion names the function itself, so two walks calling
    /// themselves differ by no renamed name.
    own: bool,
}

/// Placeholders for renamed identifiers and literals. The control character
/// keeps them apart from any real token text.
const IDENTIFIER_TOKEN: &str = "\u{1}id";
const LITERAL_TOKEN: &str = "\u{1}lit";

impl Token<'_> {
    fn normal(&self) -> &str {
        match self.kind {
            TokenKind::Identifier => IDENTIFIER_TOKEN,
            TokenKind::Literal => LITERAL_TOKEN,
            TokenKind::Other => self.text,
        }
    }
}

#[derive(Clone)]
struct Statement {
    span: Range<usize>,
    tokens: Range<usize>,
    hash: u64,
    /// Part of the frame of a walk rather than its work.
    frame: Option<Frame>,
    /// Go's error check or deferred cleanup, which every call site repeats.
    idiom: bool,
}

struct Block {
    file: usize,
    statements: Vec<Statement>,
    /// The statements are the whole body of a function.
    whole: bool,
}

struct Parsed<'a> {
    tokens: Vec<Token<'a>>,
}

pub fn find(files: &[SourceFile<'_>]) -> Candidates {
    let (parsed, blocks) = statement_blocks(files);
    let local: BTreeSet<String> = files
        .iter()
        .filter_map(|f| f.package?.name.clone())
        .collect();
    let mut pairs: Vec<Pair> = matching_windows(&blocks)
        .into_iter()
        .filter(|&((bx, _), (by, _), _)| {
            let (a, b) = (&files[blocks[bx].file], &files[blocks[by].file]);
            crate::packages::linked(a.package, b.package, &local)
                && generic::family(a.path) == generic::family(b.path)
                && !separate_examples(a.path, b.path)
                && !separate_tests(a, b)
        })
        .filter_map(|window| pair(files, &parsed, &blocks, window))
        .filter(|p| !deprecated(files, &p.a) && !deprecated(files, &p.b))
        .filter(|p| !retired(&p.a.path) && !retired(&p.b.path))
        .collect();
    drop_nested(&mut pairs);
    pairs.sort_by(by_rank);
    let mut pairs = representatives(pairs);
    // Groups rank by size times their number of copies.
    pairs.sort_by(by_rank);
    capped(one_per_function_pair(pairs))
}

/// Two copied windows of the same two functions, split by one differing
/// statement, are one repetition: keep the higher-ranked pair only.
fn one_per_function_pair(pairs: Vec<Pair>) -> Vec<Pair> {
    let mut seen = BTreeSet::new();
    pairs
        .into_iter()
        .filter(|pair| {
            let (Some(a), Some(b)) = (&pair.a.function, &pair.b.function) else {
                return true;
            };
            let mut key = [(&pair.a.path, a), (&pair.b.path, b)];
            key.sort();
            seen.insert(key.map(|(path, name)| (path.clone(), name.clone())))
        })
        .collect()
}

/// Tokens of every file and the statement blocks inside unit bodies.
fn statement_blocks<'a>(files: &[SourceFile<'a>]) -> (Vec<Parsed<'a>>, Vec<Block>) {
    let mut parsed = Vec::new();
    let mut blocks = Vec::new();
    for (index, file) in files.iter().enumerate() {
        let Ok(Some(tree)) = crate::syntax::parse(file.path, file.source) else {
            parsed.push(Parsed { tokens: Vec::new() });
            continue;
        };
        let generic = generic::read(file.path, file.source);
        let mut tokens = Vec::new();
        let literals = generic.map_or(LITERALS, |language| language.literals);
        leaves(tree.root_node(), file.source, literals, &mut tokens);
        mark_recursion(&mut tokens, file.units, file.path);
        let bodies: Vec<Range<usize>> = file
            .units
            .iter()
            .filter(|u| !u.equality)
            .filter_map(|u| u.body.clone())
            .collect();
        let found = Found {
            index,
            bodies: &bodies,
            tokens: &tokens,
            statements: generic.map(|language| language.blocks),
        };
        collect_blocks(tree.root_node(), file, &found, &mut blocks);
        parsed.push(Parsed { tokens });
    }
    (parsed, blocks)
}

/// A maximal run of matching statement hashes: (block, start) twice and its length.
type Window = ((usize, usize), (usize, usize), usize);

/// Seed on consecutive statement pairs, then extend each diagonal as far as the
/// hashes keep matching; a diagonal already covered is not reported again.
fn matching_windows(blocks: &[Block]) -> Vec<Window> {
    let mut covered = BTreeSet::new();
    let mut found = Vec::new();
    for ((bx, kx), (by, ky)) in seed_pairs(blocks) {
        let diagonal = (bx, by, kx as isize - ky as isize);
        if covered.contains(&(diagonal, kx)) {
            continue;
        }
        let n = extend(blocks, (bx, kx), (by, ky));
        covered.extend((0..n).map(|t| (diagonal, kx + t)));
        if bx == by && one_run(&blocks[bx].statements[kx.min(ky)..kx.max(ky) + n]) {
            continue;
        }
        found.push(((bx, kx), (by, ky), n));
    }
    found
}

/// Every two places one seed occurs, among its first `SEED_OCCURRENCES`;
/// two places in one block must be far enough apart not to overlap.
fn seed_pairs(blocks: &[Block]) -> impl Iterator<Item = ((usize, usize), (usize, usize))> {
    seeds(blocks).into_values().flat_map(|mut places| {
        places.truncate(SEED_OCCURRENCES);
        let pairs: Vec<_> = places
            .iter()
            .enumerate()
            .flat_map(|(x, &a)| places[x + 1..].iter().map(move |&b| (a, b)))
            .filter(|&((bx, kx), (by, ky))| bx != by || ky >= kx + MIN_STATEMENTS)
            .collect();
        pairs
    })
}

/// Statements that all read alike, such as sqlite-utils' nine
/// `x = self.value_or_default("x", x)` lines or a list of lazy imports: a
/// list of one kind of statement, which matches itself shifted by one.
fn one_run(statements: &[Statement]) -> bool {
    statements
        .windows(2)
        .all(|pair| pair[0].hash == pair[1].hash)
}

/// Every place a pair of consecutive statement hashes occurs.
fn seeds(blocks: &[Block]) -> BTreeMap<(u64, u64), Vec<(usize, usize)>> {
    let mut seeds = BTreeMap::<(u64, u64), Vec<(usize, usize)>>::new();
    for (b, block) in blocks.iter().enumerate() {
        for k in 0..block.statements.len().saturating_sub(1) {
            let key = (block.statements[k].hash, block.statements[k + 1].hash);
            seeds.entry(key).or_default().push((b, k));
        }
    }
    seeds
}

/// How many statements match from two seeds; a window never overlaps itself.
fn extend(blocks: &[Block], (bx, kx): (usize, usize), (by, ky): (usize, usize)) -> usize {
    let (sx, sy) = (&blocks[bx].statements, &blocks[by].statements);
    let mut n = MIN_STATEMENTS;
    while kx + n < sx.len()
        && ky + n < sy.len()
        && sx[kx + n].hash == sy[ky + n].hash
        && (bx != by || kx + n < ky)
    {
        n += 1;
    }
    n
}

/// A candidate pair from one window, when its tokens align with consistent
/// renaming and it is large enough to report. The owner is a selected site.
fn pair(
    files: &[SourceFile<'_>],
    parsed: &[Parsed<'_>],
    blocks: &[Block],
    window: Window,
) -> Option<Pair> {
    let ((bx, kx), (by, ky), n) = window;
    let (fx, fy) = (blocks[bx].file, blocks[by].file);
    if !files[fx].selected && !files[fy].selected {
        return None;
    }
    let x = &blocks[bx].statements[kx..kx + n];
    let y = &blocks[by].statements[ky..ky + n];
    let tx = &parsed[fx].tokens[x[0].tokens.start..x[n - 1].tokens.end];
    let ty = &parsed[fy].tokens[y[0].tokens.start..y[n - 1].tokens.end];
    let differences = align(tx, ty)?;
    let span_x = x[0].span.start..x[n - 1].span.end;
    let span_y = y[0].span.start..y[n - 1].span.end;
    let size =
        compact(&files[fx].source[span_x.clone()]).min(compact(&files[fy].source[span_y.clone()]));
    // A repeated pair of statements is usually an idiom, such as a call and
    // its check.
    if n < MIN_CLONE_STATEMENTS
        || size < MIN_BYTES
        || only_frame(files, blocks, window)
        || mostly_guards(files, blocks, window)
    {
        return None;
    }
    let normalized = crate::schema::hash(
        tx.iter()
            .map(Token::normal)
            .collect::<Vec<_>>()
            .join(crate::schema::HASH_SEPARATOR)
            .as_bytes(),
    );
    let a = site(files, fx, span_x);
    let b = site(files, fy, span_y);
    // Ties keep path and line order.
    let swap = !files[fx].selected
        || (files[fy].selected && (&b.path, b.start_line) < (&a.path, a.start_line));
    let (a, b, differences) = if swap {
        let flipped = differences
            .into_iter()
            .map(|d| Difference { a: d.b, b: d.a })
            .collect();
        (b, a, flipped)
    } else {
        (a, b, differences)
    };
    Some(Pair {
        a,
        b,
        differences,
        size,
        occurrences: 2,
        copies: Vec::new(),
        normalized,
    })
}

/// Two parts of recursive functions that share little beyond the frame of a
/// walk: early exits and recursion into the function itself. Two walks that
/// stop at a different kind and recurse into their children share that
/// frame whatever they do at each node, so a copy of part of them needs two
/// statements beyond it, or one of half the size a copy needs. A copy of
/// the whole of both functions is a copy, however small its work.
fn only_frame(files: &[SourceFile<'_>], blocks: &[Block], window: Window) -> bool {
    let ((bx, kx), (by, ky), n) = window;
    let x = &blocks[bx].statements[kx..kx + n];
    let y = &blocks[by].statements[ky..ky + n];
    let recurses = |s: &[Statement]| s.iter().any(|s| s.frame == Some(Frame::Recursion));
    let whole = |b: usize, k: usize| blocks[b].whole && k == 0 && n == blocks[b].statements.len();
    if !recurses(x) || !recurses(y) || whole(bx, kx) && whole(by, ky) {
        return false;
    }
    let work: Vec<(&Statement, &Statement)> = x
        .iter()
        .zip(y)
        .filter(|(a, b)| a.frame.is_none() || b.frame.is_none())
        .collect();
    let bytes = |file: usize, s: &Statement| compact(&files[file].source[s.span.clone()]);
    let size = work
        .iter()
        .map(|(a, _)| bytes(blocks[bx].file, a))
        .sum::<usize>()
        .min(work.iter().map(|(_, b)| bytes(blocks[by].file, b)).sum());
    work.len() < MIN_STATEMENTS && size < MIN_BYTES / 2
}

/// Part of two Go functions that is mostly error checks and deferred
/// cleanups: `if err != nil { return err }` after each call and
/// `defer tx.Rollback()`. wtf's per-entity store functions shared a
/// transaction's begin, rollback and error checks around calls to their own
/// type's functions, which read as copies. A copy of part of them needs as
/// much other work as a copy of a walk's frame does; a copy of the whole of
/// both functions is still one.
fn mostly_guards(files: &[SourceFile<'_>], blocks: &[Block], window: Window) -> bool {
    let ((bx, kx), (by, ky), n) = window;
    let x = &blocks[bx].statements[kx..kx + n];
    let y = &blocks[by].statements[ky..ky + n];
    let whole = |b: usize, k: usize| blocks[b].whole && k == 0 && n == blocks[b].statements.len();
    if !x.iter().any(|s| s.idiom) || whole(bx, kx) && whole(by, ky) {
        return false;
    }
    let work: Vec<(&Statement, &Statement)> = x
        .iter()
        .zip(y)
        .filter(|(a, b)| !a.idiom || !b.idiom)
        .collect();
    let bytes = |file: usize, s: &Statement| compact(&files[file].source[s.span.clone()]);
    let size = work
        .iter()
        .map(|(a, _)| bytes(blocks[bx].file, a))
        .sum::<usize>()
        .min(work.iter().map(|(_, b)| bytes(blocks[by].file, b)).sum());
    work.len() < MIN_CLONE_STATEMENTS && size < MIN_BYTES
}

/// Go's `if err != nil { return …, err }` or a `defer` statement.
fn go_idiom(statement: Node<'_>, source: &str) -> bool {
    match statement.kind() {
        "defer_statement" => true,
        "if_statement" => {
            let checks_err = statement
                .child_by_field_name("condition")
                .is_some_and(|c| compact_text(&source[c.byte_range()]) == "err!=nil");
            let returns = statement
                .child_by_field_name("consequence")
                .is_some_and(|block| {
                    // Newer Go grammars wrap a block's statements in a list.
                    let list = block
                        .named_child(0)
                        .filter(|c| c.kind() == "statement_list")
                        .unwrap_or(block);
                    let mut cursor = list.walk();
                    let body: Vec<Node<'_>> = list.named_children(&mut cursor).collect();
                    !body.is_empty() && body.iter().all(|s| s.kind() == "return_statement")
                });
            checks_err && returns && statement.child_by_field_name("alternative").is_none()
        }
        _ => false,
    }
}

fn compact_text(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

/// Drop pairs whose sites both lie inside a larger pair's sites.
fn drop_nested(pairs: &mut Vec<Pair>) {
    let snapshot = pairs.clone();
    pairs.retain(|p| {
        !snapshot.iter().any(|q| {
            let larger = q.a.span.len() + q.b.span.len() > p.a.span.len() + p.b.span.len();
            larger
                && ((contains(&q.a, &p.a) && contains(&q.b, &p.b))
                    || (contains(&q.a, &p.b) && contains(&q.b, &p.a)))
        })
    });
}

/// Keep ranked groups within the per-run and per-file caps; count the rest.
/// Copies in a language of the generic tier (`analysis::generic`) take only
/// the places the languages with analyzers of their own leave: ranked
/// together, C copies of b2-bend-collections' native benchmarks took a
/// place from a Bend copy, changing findings that labels measured.
fn capped(pairs: Vec<Pair>) -> Candidates {
    let mut omitted = BTreeMap::<PathBuf, usize>::new();
    let mut per_file = BTreeMap::<PathBuf, usize>::new();
    let mut kept = Vec::new();
    let (specific, generic): (Vec<Pair>, Vec<Pair>) = pairs
        .into_iter()
        .partition(|pair| generic::family(&pair.a.path).is_none());
    for pair in specific.into_iter().chain(generic) {
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

fn by_rank(p: &Pair, q: &Pair) -> std::cmp::Ordering {
    q.rank()
        .cmp(&p.rank())
        .then_with(|| (&p.a.path, p.a.start_line).cmp(&(&q.a.path, q.a.start_line)))
        .then_with(|| (&p.b.path, p.b.start_line).cmp(&(&q.b.path, q.b.start_line)))
}

fn overlaps(x: &Site, y: &Site) -> bool {
    x.path == y.path && x.span.start < y.span.end && y.span.start < x.span.end
}

/// Sites that cover at least half of each other describe the same code.
fn same_code(x: &Site, y: &Site) -> bool {
    if x.path != y.path {
        return false;
    }
    let shared = x
        .span
        .end
        .min(y.span.end)
        .saturating_sub(x.span.start.max(y.span.start));
    2 * shared >= x.span.len() && 2 * shared >= y.span.len()
}

/// One judged pair per clone group. Pairs whose sites repeat the same code
/// (mutual half overlap) are linked; the first pair of each group in rank
/// order represents it and carries the other copies. Linking on plain overlap
/// let short idioms inside a larger copy chain unrelated code together.
fn representatives(pairs: Vec<Pair>) -> Vec<Pair> {
    let groups = same_code_groups(&pairs);
    let mut sites = BTreeMap::<usize, Vec<Site>>::new();
    for (i, pair) in pairs.iter().enumerate() {
        let group = sites.entry(groups[i]).or_default();
        for site in [&pair.a, &pair.b] {
            if !group.iter().any(|known| overlaps(known, site)) {
                group.push(site.clone());
            }
        }
    }
    let mut kept = Vec::new();
    for (i, mut pair) in pairs.into_iter().enumerate() {
        if groups[i] != i {
            continue;
        }
        pair.copies = sites[&i]
            .iter()
            .filter(|s| !overlaps(s, &pair.a) && !overlaps(s, &pair.b))
            .cloned()
            .collect();
        pair.copies
            .sort_by(|x, y| (&x.path, x.start_line).cmp(&(&y.path, y.start_line)));
        pair.occurrences = 2 + pair.copies.len();
        kept.push(pair);
    }
    kept
}

/// For each pair, the first pair of its group: pairs whose sites repeat the
/// same code are linked, transitively (union-find).
fn same_code_groups(pairs: &[Pair]) -> Vec<usize> {
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    let mut parent: Vec<usize> = (0..pairs.len()).collect();
    for i in 0..pairs.len() {
        for j in i + 1..pairs.len() {
            let (p, q) = (&pairs[i], &pairs[j]);
            let linked = [&p.a, &p.b]
                .iter()
                .any(|x| same_code(x, &q.a) || same_code(x, &q.b));
            if linked {
                let (ri, rj) = (root(&mut parent, i), root(&mut parent, j));
                parent[ri.max(rj)] = ri.min(rj);
            }
        }
    }
    (0..pairs.len()).map(|i| root(&mut parent, i)).collect()
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
        // Each side calling itself is the same step, not a rename.
        if a.own && b.own {
            continue;
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

/// Leaves holding literal values in the languages with their own analyzers;
/// a language of the generic tier names its own (`analysis::generic`).
const LITERALS: &[&str] = &[
    "string_content",
    "string_fragment",
    "integer_literal",
    "float_literal",
    "char_literal",
    "decimal_integer_literal",
    "hex_integer_literal",
    "octal_integer_literal",
    "binary_integer_literal",
    "decimal_floating_point_literal",
    "hex_floating_point_literal",
    "character_literal",
    "number",
    "integer",
    "float",
    "string_literal_content",
    "raw_string_content",
    "verbatim_string_literal",
    "real_literal",
];

fn leaves<'a>(node: Node<'_>, source: &'a str, literals: &[&str], tokens: &mut Vec<Token<'a>>) {
    if is_comment(node) {
        return;
    }
    let kind = node.kind();
    let literal = literals.contains(&kind);
    if node.child_count() == 0 || literal {
        let text = text(node, source);
        if text.trim().is_empty() {
            return;
        }
        let kind = if literal {
            TokenKind::Literal
        } else if kind.ends_with("identifier")
            || matches!(
                kind,
                "identifier" | "constant" | "instance_variable" | "name"
            )
        {
            TokenKind::Identifier
        } else {
            TokenKind::Other
        };
        tokens.push(Token {
            kind,
            text,
            start: node.start_byte(),
            own: false,
        });
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        leaves(child, source, literals, tokens);
    }
}

/// What finding a file's statement blocks reads: the file's index, the
/// bodies of its units, its tokens, and for a language of the generic tier
/// the kinds that hold its statements.
struct Found<'f, 'a> {
    index: usize,
    bodies: &'f [Range<usize>],
    tokens: &'f [Token<'a>],
    statements: Option<&'static [&'static str]>,
}

fn collect_blocks(
    node: Node<'_>,
    file: &SourceFile<'_>,
    found: &Found<'_, '_>,
    blocks: &mut Vec<Block>,
) {
    let holds = match found.statements {
        Some(kinds) => kinds.contains(&node.kind()),
        None => holds_statements(node),
    };
    if holds
        && found
            .bodies
            .iter()
            .any(|b| b.start <= node.start_byte() && node.end_byte() <= b.end)
    {
        let all = block_statements(node, file, found.tokens);
        // A Go body holds its statements in a `statement_list` inside the block.
        let body = node
            .parent()
            .filter(|p| node.kind() == "statement_list" && p.kind() == "block")
            .unwrap_or(node);
        let whole = all.iter().all(Option::is_some)
            && file
                .units
                .iter()
                .any(|u| u.callable() && u.body == Some(body.byte_range()));
        // Excluded statements break a window, so split the block there.
        for statements in all.split(Option::is_none) {
            let statements: Vec<Statement> = statements.iter().flatten().cloned().collect();
            if !statements.is_empty() {
                blocks.push(Block {
                    file: found.index,
                    statements,
                    whole,
                });
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_blocks(child, file, found, blocks);
    }
}

/// A node whose named children are statements. Ruby holds statements in a
/// `body_statement` or `block_body`, and in the `then`, `else` and `do` of a
/// branch or loop; its `block` is a `{ … }` argument around a `block_body`.
/// PHP holds them in a `compound_statement`, and a Java constructor's
/// statements are in a `constructor_body`.
fn holds_statements(node: Node<'_>) -> bool {
    let ruby_block = node.kind() == "block" && node.parent().is_some_and(|p| p.kind() == "call");
    matches!(
        node.kind(),
        "block"
            | "statement_block"
            | "statement_list"
            | "compound_statement"
            | "body_statement"
            | "block_body"
            | "then"
            | "else"
            | "do"
            | "constructor_body"
    ) && !ruby_block
}

/// A block's statements with normalized-token hashes; `None` for excluded lines.
fn block_statements(
    node: Node<'_>,
    file: &SourceFile<'_>,
    tokens: &[Token<'_>],
) -> Vec<Option<Statement>> {
    let mut statements = Vec::new();
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        // A docstring documents; counted as a statement, it made one-line
        // wrappers such as flask's `render_template` read as copies.
        if is_comment(child) || super::comments::docstring(child).is_some() {
            continue;
        }
        let line = line_of(file.source, child.start_byte());
        if file.excluded.iter().any(|r| r.contains(&line))
            || node.kind() == "constructor_body" && field_initializer(child)
            || literal_setter(child, file.source)
        {
            statements.push(None);
            continue;
        }
        let start = tokens.partition_point(|t| t.start < child.start_byte());
        let end = tokens.partition_point(|t| t.start < child.end_byte());
        let key = tokens[start..end]
            .iter()
            .map(Token::normal)
            .collect::<Vec<_>>()
            .join(crate::schema::HASH_SEPARATOR);
        statements.push(Some(Statement {
            span: child.byte_range(),
            tokens: start..end,
            hash: fast_hash(&key),
            frame: frame(child, tokens),
            idiom: go_idiom(child, file.source),
        }));
    }
    statements
}

/// A Java constructor statement that stores a parameter, another object's
/// field or a literal in a field, as in `this.name = name;` or
/// `timeout = copy.timeout;`. A run of them is how a constructor fills its
/// fields: two constructors assigning different fields matched as copies.
fn field_initializer(statement: Node<'_>) -> bool {
    let Some(assignment) = statement.named_child(0).filter(|a| {
        statement.kind() == "expression_statement" && a.kind() == "assignment_expression"
    }) else {
        return false;
    };
    let simple = |side: Option<Node<'_>>, value: bool| {
        side.is_some_and(|n| match n.kind() {
            "identifier" => true,
            "field_access" => n
                .child_by_field_name("object")
                .is_some_and(|o| matches!(o.kind(), "this" | "identifier")),
            kind => value && (kind.ends_with("_literal") || matches!(kind, "true" | "false")),
        })
    };
    assignment
        .child_by_field_name("operator")
        .is_some_and(|o| o.kind() == "=")
        && simple(assignment.child_by_field_name("left"), false)
        && simple(assignment.child_by_field_name("right"), true)
}

/// A Java setter given one literal, as in `owner.setCity("Madison");`. A run
/// of them fills an object with data: a test fixture built in one test and a
/// helper building another owner matched as copies whose only differences
/// were the values.
fn literal_setter(statement: Node<'_>, source: &str) -> bool {
    let Some(call) = statement
        .named_child(0)
        .filter(|c| statement.kind() == "expression_statement" && c.kind() == "method_invocation")
    else {
        return false;
    };
    let setter = call.child_by_field_name("name").is_some_and(|name| {
        text(name, source)
            .strip_prefix("set")
            .is_some_and(|rest| rest.starts_with(|c: char| c.is_ascii_uppercase()))
    });
    let on_object = call
        .child_by_field_name("object")
        .is_some_and(|o| matches!(o.kind(), "identifier" | "this" | "field_access"));
    let literal = call
        .child_by_field_name("arguments")
        .is_some_and(|arguments| {
            arguments.named_child_count() == 1
                && arguments.named_child(0).is_some_and(|argument| {
                    argument.kind().ends_with("_literal")
                        || matches!(argument.kind(), "true" | "false")
                })
        });
    setter && on_object && literal
}
