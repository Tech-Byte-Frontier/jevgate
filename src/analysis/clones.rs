//! Type-2 clone candidates across the selected files and explicit context.
//! Identifiers and literals are normalized; windows start and end on whole
//! statements inside function bodies; identifiers must be renamed consistently.
use super::{fast_hash, is_comment, line_of, text, units::Unit};
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range,
    path::{Path, PathBuf},
};
use tree_sitter::Node;

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
    /// Part of the frame of a walk rather than its work: an early exit that
    /// does nothing else, or a recursion into the function itself.
    frame: bool,
}

struct Block {
    file: usize,
    statements: Vec<Statement>,
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
        })
        .filter_map(|window| pair(files, &parsed, &blocks, window))
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
        let mut tokens = Vec::new();
        leaves(tree.root_node(), file.source, &mut tokens);
        mark_recursion(&mut tokens, file.units);
        let bodies: Vec<Range<usize>> = file
            .units
            .iter()
            .filter(|u| !u.equality)
            .filter_map(|u| u.body.clone())
            .collect();
        collect_blocks(tree.root_node(), file, index, &bodies, &tokens, &mut blocks);
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
    for occurrences in seeds(blocks).values() {
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
                let n = extend(blocks, (bx, kx), (by, ky));
                for t in 0..n {
                    covered.insert((diagonal, kx + t));
                }
                found.push(((bx, kx), (by, ky), n));
            }
        }
    }
    found
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
    ((bx, kx), (by, ky), n): Window,
) -> Option<Pair> {
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
    // its check. Early exits and recursion into the function itself frame
    // any tree walk, so a copy needs as many statements beyond them: two
    // walks that stop at a different kind and recurse into their children
    // share only that frame.
    let work = x
        .iter()
        .zip(y)
        .filter(|(a, b)| !(a.frame && b.frame))
        .count();
    if n < MIN_CLONE_STATEMENTS || work < MIN_STATEMENTS || size < MIN_BYTES {
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
fn capped(pairs: Vec<Pair>) -> Candidates {
    let mut omitted = BTreeMap::<PathBuf, usize>::new();
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
            | "decimal_integer_literal"
            | "hex_integer_literal"
            | "octal_integer_literal"
            | "binary_integer_literal"
            | "decimal_floating_point_literal"
            | "hex_floating_point_literal"
            | "character_literal"
            | "number"
            | "integer"
            | "float"
            | "string_literal_content"
            | "raw_string_content"
            | "verbatim_string_literal"
            | "real_literal"
    );
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
    if holds_statements(node)
        && bodies
            .iter()
            .any(|b| b.start <= node.start_byte() && node.end_byte() <= b.end)
    {
        // Excluded statements break a window, so split the block there.
        for statements in block_statements(node, file, tokens).split(Option::is_none) {
            let statements: Vec<Statement> = statements.iter().flatten().cloned().collect();
            if !statements.is_empty() {
                blocks.push(Block {
                    file: index,
                    statements,
                });
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_blocks(child, file, index, bodies, tokens, blocks);
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

/// Mark each call of the function it sits in: its name followed by its
/// arguments inside the body of the innermost callable of that name, bare or
/// on the object itself (`self.walk(`, `this.walk(`, `Self::walk(`). A call
/// through another path, as `native::get()` inside `get`, names a
/// different function.
fn mark_recursion(tokens: &mut [Token<'_>], units: &[Unit]) {
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
        let qualifier = i
            .checked_sub(2)
            .filter(|&q| matches!(tokens[q].text, "." | "::" | "->" | "?."));
        let on_itself = qualifier.is_none_or(|q| {
            q.checked_sub(1).is_some_and(|o| {
                matches!(tokens[o].text, "self" | "Self" | "this" | "cls" | "static")
            })
        });
        if !on_itself {
            continue;
        }
        let own = callables
            .iter()
            .filter(|u| u.body.as_ref().is_some_and(|b| b.contains(&token.start)))
            .min_by_key(|u| u.span.len())
            .is_some_and(|u| u.short_name == token.text);
        tokens[i - 1].own = own;
    }
}

/// The tokens of one node.
fn tokens_of<'t, 'a>(tokens: &'t [Token<'a>], node: Node<'_>) -> &'t [Token<'a>] {
    let start = tokens.partition_point(|t| t.start < node.start_byte());
    let end = tokens.partition_point(|t| t.start < node.end_byte());
    &tokens[start..end]
}

/// Statements nested at any depth inside a statement's branches and loops.
fn nested_statements<'t>(node: Node<'t>, nested: &mut Vec<Node<'t>>) {
    let holds = holds_statements(node);
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if holds && !is_comment(child) {
            nested.push(child);
        }
        nested_statements(child, nested);
    }
}

/// Part of a walk's frame rather than its work: an early exit that does
/// nothing else, or a recursion whose nested statements only call the
/// function itself or leave early, as
/// `for child in node.named_children(&mut cursor) { bound(child, names); }`.
fn frame(statement: Node<'_>, tokens: &[Token<'_>]) -> bool {
    let calls_itself = |node: Node<'_>| tokens_of(tokens, node).iter().any(|t| t.own);
    if exit_guard(statement, tokens) {
        return true;
    }
    if !calls_itself(statement) {
        return false;
    }
    let mut nested = Vec::new();
    nested_statements(statement, &mut nested);
    nested
        .into_iter()
        .all(|n| calls_itself(n) || exits(n, tokens) || exit_guard(n, tokens))
}

/// A branch that only leaves, as `if node.kind() == "call" { return; }`,
/// `if (done) return;` or `return if done`.
fn exit_guard(statement: Node<'_>, tokens: &[Token<'_>]) -> bool {
    let words = tokens_of(tokens, statement);
    let guard = words
        .first()
        .is_some_and(|t| matches!(t.text, "if" | "unless"))
        || matches!(statement.kind(), "if_modifier" | "unless_modifier");
    if !guard
        || words
            .iter()
            .any(|t| matches!(t.text, "else" | "elif" | "elsif"))
    {
        return false;
    }
    let mut nested = Vec::new();
    nested_statements(statement, &mut nested);
    if nested.is_empty() {
        // A branch without braces holds its one statement in a field.
        nested.extend(
            ["consequence", "body"]
                .into_iter()
                .filter_map(|field| statement.child_by_field_name(field)),
        );
    }
    !nested.is_empty() && nested.into_iter().all(|n| exits(n, tokens))
}

/// `return`, `break`, `continue` or `next` with no value, or with nothing.
fn exits(statement: Node<'_>, tokens: &[Token<'_>]) -> bool {
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

/// A block's statements with normalized-token hashes; `None` for excluded lines.
fn block_statements(
    node: Node<'_>,
    file: &SourceFile<'_>,
    tokens: &[Token<'_>],
) -> Vec<Option<Statement>> {
    let mut statements = Vec::new();
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if is_comment(child) {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Candidate pairs between two selected files, each a (path, source).
    fn pairs_between(a: (&str, &str), b: (&str, &str)) -> usize {
        run(&[(a.0, a.1, true), (b.0, b.1, true)]).pairs.len()
    }

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
                excluded: Vec::new(),
                package: None,
            })
            .collect();
        find(&sources)
    }

    #[test]
    fn copies_in_unrelated_packages_are_not_candidates() {
        let package = |dir: &str, dependencies: &[&str]| crate::packages::Package {
            dir: dir.into(),
            name: Some(dir.into()),
            dependencies: dependencies.iter().map(|d| d.to_string()).collect(),
        };
        let (a, b, shared) = (
            package("a", &["shared"]),
            package("b", &["shared"]),
            package("shared", &[]),
        );
        let units = super::super::units::parse(Path::new("x.rs"), LOAD).unwrap();
        let file = |path: &'static str, package| SourceFile {
            path: Path::new(path),
            source: LOAD,
            selected: true,
            units: &units.units,
            excluded: Vec::new(),
            package,
        };
        let separate = package("c", &[]);
        assert!(
            find(&[file("a/x.rs", Some(&a)), file("c/x.rs", Some(&separate))])
                .pairs
                .is_empty()
        );
        let linked = find(&[
            file("a/x.rs", Some(&a)),
            file("b/x.rs", Some(&b)),
            file("shared/x.rs", Some(&shared)),
        ]);
        assert!(!linked.pairs.is_empty());
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
    fn java_copies_are_candidates_but_equality_boilerplate_is_not() {
        let position = "class Position {\n\tprivate final int line;\n\tprivate final int column;\n\n\t@Override\n\tpublic boolean equals(Object other) {\n\t\tif (this == other) return true;\n\t\tif (other == null || getClass() != other.getClass()) return false;\n\t\tPosition that = (Position) other;\n\t\tif (line != that.line) return false;\n\t\treturn column == that.column;\n\t}\n\n\tString describe(Map<String, String> fields) {\n\t\tString text = fields.get(\"text\");\n\t\tString trimmed = text.trim();\n\t\tString lower = trimmed.toLowerCase();\n\t\tfields.put(\"text\", lower);\n\t\treturn lower + line;\n\t}\n}\n";
        let range = position
            .replace("Position", "Range")
            .replace("line", "start")
            .replace("column", "end");
        let found = run(&[
            ("Position.java", position, true),
            ("Range.java", &range, true),
        ]);
        let functions: Vec<_> = found
            .pairs
            .iter()
            .map(|p| (p.a.function.as_deref(), p.b.function.as_deref()))
            .collect();
        assert_eq!(
            functions,
            [(Some("Position::describe"), Some("Range::describe"))]
        );
    }

    #[test]
    fn java_constructors_filling_their_fields_are_not_copies() {
        let position = "class Position {\n\tPosition(int sourceLineNumber, int sourceColumnNumber, int sourceByteOffset, int sourceCharacterOffset, int trackedPosition) {\n\t\tthis.sourceLineNumber = sourceLineNumber;\n\t\tthis.sourceColumnNumber = sourceColumnNumber;\n\t\tthis.sourceByteOffset = sourceByteOffset;\n\t\tthis.sourceCharacterOffset = sourceCharacterOffset;\n\t\tthis.trackedPosition = trackedPosition;\n\t\tthis.valid = true;\n\t}\n\n\tPosition(Position copy) {\n\t\tsourceLineNumber = copy.sourceLineNumber;\n\t\tsourceColumnNumber = copy.sourceColumnNumber;\n\t\tsourceByteOffset = copy.sourceByteOffset;\n\t\tsourceCharacterOffset = copy.sourceCharacterOffset;\n\t\ttrackedPosition = copy.trackedPosition;\n\t}\n}\n";
        let range = position
            .replace("Position", "Range")
            .replace("line", "start")
            .replace("column", "end");
        assert_eq!(
            pairs_between(("Position.java", position), ("Range.java", &range)),
            0
        );
        // Work beyond storing fields is still compared.
        let worker = |name: &str| {
            format!(
                "class {name} {{\n\t{name}(Map<String, String> fields) {{\n\t\tString text = fields.get(\"text\");\n\t\tString trimmed = text.trim();\n\t\tString lower = trimmed.toLowerCase();\n\t\tfields.put(\"text\", lower);\n\t\tfields.put(\"length\", String.valueOf(lower.length()));\n\t\tthis.fields = fields;\n\t}}\n}}\n"
            )
        };
        let (a, b) = (worker("Position"), worker("Range"));
        assert_eq!(pairs_between(("Position.java", &a), ("Range.java", &b)), 1);
    }

    #[test]
    fn java_setters_given_literals_are_data_not_copies() {
        let fixture = "class OwnerTests {\n\tprivate Owner george() {\n\t\tOwner george = new Owner();\n\t\tgeorge.setFirstName(\"George\");\n\t\tgeorge.setLastName(\"Franklin\");\n\t\tgeorge.setAddress(\"110 W. Liberty St.\");\n\t\tgeorge.setCity(\"Madison\");\n\t\tgeorge.setTelephone(\"6085551023\");\n\t\treturn george;\n\t}\n}\n";
        let inline = "class ServiceTests {\n\tvoid insertsOwner() {\n\t\tOwner owner = new Owner();\n\t\towner.setFirstName(\"Sam\");\n\t\towner.setLastName(\"Schultz\");\n\t\towner.setAddress(\"4, Evans Street\");\n\t\towner.setCity(\"Wollongong\");\n\t\towner.setTelephone(\"4444444444\");\n\t\towners.save(owner);\n\t}\n}\n";
        assert_eq!(
            pairs_between(("OwnerTests.java", fixture), ("ServiceTests.java", inline)),
            0
        );
        // Setters given computed values copy logic and are still compared.
        let mapping = |name: &str| {
            format!(
                "class {name} {{\n\tOwnerDto map(Owner owner) {{\n\t\tOwnerDto dto = new OwnerDto();\n\t\tdto.setFirstName(owner.getFirstName().trim());\n\t\tdto.setLastName(owner.getLastName().trim());\n\t\tdto.setAddress(owner.getAddress().trim());\n\t\tdto.setCity(owner.getCity().toUpperCase());\n\t\tdto.setTelephone(owner.getTelephone().replace(\" \", \"\"));\n\t\treturn dto;\n\t}}\n}}\n"
            )
        };
        let (a, b) = (mapping("OwnerMapper"), mapping("VetMapper"));
        assert_eq!(
            pairs_between(("OwnerMapper.java", &a), ("VetMapper.java", &b)),
            1
        );
    }

    /// A Ruby assignment's bound locals and a Rust `use` path's imported
    /// names: two walks that share only the frame of skipping one kind and
    /// recursing into their children.
    const BOUND: &str = "fn bound(node: Node<'_>, source: &str, names: &mut Vec<String>) {\n    if node.kind() == \"identifier\" {\n        names.push(text(node, source).to_string());\n        return;\n    }\n    if node.kind() == \"call\" {\n        return;\n    }\n    let mut cursor = node.walk();\n    for child in node.named_children(&mut cursor) {\n        bound(child, source, names);\n    }\n}\n";
    const IMPORTS: &str = "fn imports(node: Node<'_>, source: &str, names: &mut BTreeSet<String>) {\n    if node.kind() == \"identifier\" {\n        let name = text(node, source);\n        if !matches!(name, \"self\" | \"super\" | \"crate\") {\n            names.insert(name.to_string());\n        }\n        return;\n    }\n    if node.kind() == \"scoped_identifier\" {\n        if let Some(name) = node.child_by_field_name(\"name\") {\n            imports(name, source, names);\n        }\n        return;\n    }\n    if node.kind() == \"string\" {\n        return;\n    }\n    let mut cursor = node.walk();\n    for child in node.named_children(&mut cursor) {\n        imports(child, source, names);\n    }\n}\n";

    #[test]
    fn walks_sharing_only_an_early_exit_and_their_recursion_are_not_copies() {
        assert_eq!(
            pairs_between(("ruby.rs", BOUND), ("import_names.rs", IMPORTS)),
            0
        );
        // The same frame around a different exit, with the recursion written
        // as a method on the walker itself, is still only the frame.
        let method = |name: &str, stop: &str| {
            format!(
                "impl Walker {{\n    fn {name}(&mut self, node: Node<'_>) {{\n        if node.is_missing() {{\n            return;\n        }}\n        if node.kind() == \"{stop}\" {{\n            return;\n        }}\n        let mut cursor = node.walk();\n        for child in node.named_children(&mut cursor) {{\n            if child.is_extra() {{\n                continue;\n            }}\n            self.{name}(child);\n        }}\n    }}\n}}\n"
            )
        };
        let (a, b) = (method("locals", "call"), method("exports", "string"));
        assert_eq!(pairs_between(("a.rs", &a), ("b.rs", &b)), 0);
        // Exits without braces and `this.` recursion in JavaScript.
        let script = |name: &str, stop: &str| {
            format!(
                "class Scanner {{\n  {name}(node) {{\n    if (!node) return;\n    if (node.type === '{stop}') return;\n    const children = node.namedChildren;\n    for (const child of children) {{\n      this.{name}(child);\n    }}\n  }}\n}}\n"
            )
        };
        let (a, b) = (
            script("locals", "call_expression"),
            script("exports", "string"),
        );
        assert_eq!(pairs_between(("a.js", &a), ("b.js", &b)), 0);
    }

    #[test]
    fn walks_copying_the_work_they_do_at_each_node_are_candidates() {
        // Both walks collect the bound identifiers the same way; each calling
        // itself is the same step, so their own names are no difference.
        let copy = BOUND.replace("bound", "assigned");
        let found = run(&[("ruby.rs", BOUND, true), ("python.rs", &copy, true)]);
        assert_eq!(found.pairs.len(), 1);
        assert_eq!(found.pairs[0].differences, []);
        // A walk that does its work in the loop, around its recursion.
        let visit = |name: &str| {
            format!(
                "fn {name}(node: Node<'_>, source: &str, names: &mut Vec<String>) {{\n    if node.kind() == \"call\" {{\n        return;\n    }}\n    let mut cursor = node.walk();\n    for child in node.named_children(&mut cursor) {{\n        if child.kind() == \"identifier\" {{\n            names.push(text(child, source).to_string());\n        }}\n        {name}(child, source, names);\n    }}\n}}\n"
            )
        };
        let (a, b) = (visit("locals"), visit("parameters"));
        assert_eq!(pairs_between(("a.rs", &a), ("b.rs", &b)), 1);
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
    fn a_short_idiom_inside_a_larger_copy_forms_its_own_group() {
        let head = "    let text = std::fs::read_to_string(path).expect(\"reading the configured user file failed\");\n    let value: Value = serde_json::from_str(&text).expect(\"parsing the configured user file failed\");\n    let root = value.as_object().expect(\"the configured user file holds an object\");\n";
        let tail = "    let name = root[\"name\"].as_str().unwrap_or(\"anonymous\").trim().to_string();\n    let age = root[\"age\"].as_u64().unwrap_or(0).min(150) as u32;\n    let city = root[\"city\"].as_str().unwrap_or(\"unknown\").trim().to_string();\n    let email = root[\"email\"].as_str().unwrap_or(\"\").trim().to_lowercase();\n    Ok(User { name, age, city, email })\n";
        let full = format!("fn load(path: &str) -> Result<User> {{\n{head}{tail}}}\n");
        let other = full.replace("fn load", "fn again");
        let short = format!("fn count(path: &str) -> usize {{\n{head}    root.len()\n}}\n");
        let found = run(&[
            ("a.rs", &full, true),
            ("b.rs", &other, true),
            ("c.rs", &short, true),
        ]);
        let largest = found.pairs.iter().max_by_key(|p| p.size).unwrap();
        assert_eq!(
            (largest.a.path.as_path(), largest.b.path.as_path()),
            (Path::new("a.rs"), Path::new("b.rs"))
        );
        assert!(largest.copies.is_empty(), "{:?}", largest.copies);
        assert_eq!(found.pairs.len(), 2);
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
    fn repeated_copies_form_one_group_judged_through_one_pair() {
        let three = [
            LOAD,
            &LOAD.replace("load_user", "second"),
            &LOAD.replace("load_user", "third"),
        ]
        .concat();
        let found = run(&[("three.rs", &three, true)]);
        assert_eq!(found.pairs.len(), 1);
        let pair = &found.pairs[0];
        assert_eq!((pair.occurrences, pair.a.start_line), (3, 2));
        assert_eq!(pair.copies.len(), 1);
        let lines = [pair.b.start_line, pair.copies[0].start_line];
        assert!(lines.contains(&8) && lines.contains(&14), "{lines:?}");

        // Copies of different lengths still share a group through overlapping sites.
        let longer = LOAD.replace(
            "    Ok(User { name })",
            "    let checked = name.trim().to_string();\n    Ok(User { name: checked })",
        );
        let found = run(&[
            ("a.rs", LOAD, true),
            ("b.rs", &LOAD.replace("load_user", "other"), true),
            ("c.rs", &longer.replace("load_user", "third"), true),
        ]);
        assert_eq!(
            found.pairs.len(),
            1,
            "{:?}",
            found
                .pairs
                .iter()
                .map(|p| (&p.a.path, &p.b.path))
                .collect::<Vec<_>>()
        );
        assert_eq!(found.pairs[0].occurrences, 3);
        let body = "    total = decimal.Decimal(\"0\")\n    for row in rows:\n        total += row.amount * row.exchange_rate - row.discount_amount\n    return total.quantize(decimal.Decimal(\"0.01\"), rounding=decimal.ROUND_HALF_UP)\n";
        let python = format!(
            "def a(rows):\n{body}\ndef b(items):\n{}",
            body.replace("row", "item")
        );
        let found = run(&[("totals.py", &python, true)]);
        assert_eq!(found.pairs.len(), 1);
        assert_eq!(found.pairs[0].a.function.as_deref(), Some("a"));
    }

    #[test]
    fn windows_of_the_same_two_functions_split_by_one_statement_are_one_pair() {
        let head = "    parser = argparse.ArgumentParser(description=__doc__)\n    parser.add_argument('--owner', default='Tech')\n    parser.add_argument('--project', type=int, default=2)\n    parser.add_argument('--apply', action='store_true')\n";
        let tail = "    args = parser.parse_args()\n    if args.apply and not args.backup:\n        parser.error('--apply requires --backup')\n    run(args.owner, args.project, args.apply, args.backup)\n";
        let first = format!(
            "def main():\n{head}    parser.add_argument('--completed', action='store_true')\n{tail}"
        );
        let second = format!("def main():\n{head}{tail}");
        let found = run(&[("migrate.py", &first, true), ("retire.py", &second, true)]);
        assert_eq!(
            found
                .pairs
                .iter()
                .map(|p| (p.a.start_line, p.b.start_line))
                .collect::<Vec<_>>()
                .len(),
            1
        );
    }
}
