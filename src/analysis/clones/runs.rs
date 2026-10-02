//! Repeated token runs: loose shared-logic candidates beside the statement
//! windows. A run of `RUN_TOKENS` or more tokens that recurs elsewhere in a
//! project, or elsewhere in its own file, is a candidate; local names are
//! abstracted, while member names (`.kind`, `?.name`) and literals are kept,
//! so a renamed local still matches and an unrelated call with the same shape
//! does not. Runs read the statements the windows read, so they leave out the
//! same code (excluded lines, a constructor filling its fields, setters given
//! literals) and skip the statements of a walk's frame and Go's error checks.
//! The look-here question and the coding agent that verifies it sort the
//! candidates: statement windows of three renamed statements saw none of the
//! one-line predicates, lookups with a fallback and connection lifecycles a
//! reviewer wanted shared on a 32-file sample, and runs saw six of nine.
use super::{Block, Pair, Parsed, SourceFile, Token, TokenKind, compact, ordered_sites};
use std::{collections::BTreeSet, ops::Range};

/// Tokens a run needs to be a candidate.
pub const RUN_TOKENS: usize = 12;
/// Distinct words a run needs beside abstracted names and punctuation,
/// such as member names, keywords and literals: `x = y(z)` repeated is no
/// rule to share.
const RUN_WORDS: usize = 3;
/// A run that recurs in more places than this is an idiom of the language
/// or framework.
const RUN_OCCURRENCES: usize = 12;

/// The placeholder for an abstracted local name.
const LOCAL: &str = "\u{1}id";

/// One file's tokens that runs read, in order: `eligible` holds their
/// indices among the file's tokens, and `segment` where each one's stretch
/// of consecutive tokens begins, so no run crosses a statement left out.
struct Stream<'a> {
    eligible: Vec<usize>,
    segment: Vec<usize>,
    normal: Vec<&'a str>,
}

/// Candidate pairs of repeated runs among `files`, from the statement
/// `blocks` of their `parsed` tokens, before grouping and caps. `linked` says
/// whether two files may be compared.
pub(super) fn pairs(
    files: &[SourceFile<'_>],
    (parsed, blocks): (&[Parsed<'_>], &[Block]),
    linked: impl Fn(usize, usize) -> bool,
) -> Vec<Pair> {
    let streams = streams(parsed, blocks);
    let wrappers = wrappers(parsed.len(), blocks);
    let starts = matched_starts(&streams, linked);
    let mut found = Vec::new();
    for &(fx, px, fy, py) in &starts {
        if px > 0 && py > 0 && starts.contains(&(fx, px - 1, fy, py - 1)) {
            continue; // not the start of a run
        }
        let mut n = 1;
        while starts.contains(&(fx, px + n, fy, py + n)) {
            n += 1;
        }
        let length = n as usize + RUN_TOKENS - 1;
        let (fx, px, fy, py) = (fx as usize, px as usize, fy as usize, py as usize);
        let run = &streams[fx].normal[px..px + length];
        if fx == fy && px + length > py {
            continue; // a run overlapping itself, such as a list of one kind of line
        }
        if words(run) < RUN_WORDS || !files[fx].selected && !files[fy].selected {
            continue;
        }
        let within = |f: usize, p: usize| {
            wrapped(
                &wrappers[f],
                streams[f].eligible[p],
                streams[f].eligible[p + length - 1],
            )
        };
        if within(fx, px) && within(fy, py) {
            continue;
        }
        let x = (fx, span(&parsed[fx].tokens, &streams[fx], px, length));
        let y = (fy, span(&parsed[fy].tokens, &streams[fy], py, length));
        found.push(pair(files, x, y, run));
    }
    found
}

/// Statements a function body needs for a run inside it to be a candidate,
/// as statement windows need three: two one-line wrappers that differ in the
/// function they call, such as flask's `render_template` and
/// `stream_template`, are how a module offers variants.
const WRAPPER_STATEMENTS: usize = 2;

/// The token ranges of each file's function bodies of at most
/// `WRAPPER_STATEMENTS` statements.
fn wrappers(files: usize, blocks: &[Block]) -> Vec<Vec<Range<usize>>> {
    let mut found = vec![Vec::new(); files];
    for block in blocks.iter().filter(|b| b.whole) {
        if let (Some(first), Some(last)) = (block.statements.first(), block.statements.last())
            && block.statements.len() <= WRAPPER_STATEMENTS
        {
            found[block.file].push(first.tokens.start..last.tokens.end);
        }
    }
    found
}

/// Whether tokens `first..=last` lie inside one of `bodies`.
fn wrapped(bodies: &[Range<usize>], first: usize, last: usize) -> bool {
    bodies.iter().any(|b| b.start <= first && last < b.end)
}

/// The distinct words of a run: neither abstracted names nor punctuation.
fn words(run: &[&str]) -> usize {
    run.iter()
        .filter(|t| **t != LOCAL && t.chars().any(char::is_alphanumeric))
        .collect::<BTreeSet<_>>()
        .len()
}

fn pair(
    files: &[SourceFile<'_>],
    (fx, x): (usize, Range<usize>),
    (fy, y): (usize, Range<usize>),
    run: &[&str],
) -> Pair {
    let (a, b) = ordered_sites(files, (fx, x), (fy, y));
    let size = compact(&files[a.file].source[a.span.clone()])
        .min(compact(&files[b.file].source[b.span.clone()]));
    Pair {
        a,
        b,
        size,
        occurrences: 2,
        copies: Vec::new(),
        normalized: crate::schema::hash(run.join(crate::schema::HASH_SEPARATOR).as_bytes()),
    }
}

/// The source bytes of `length` eligible tokens from position `p`.
fn span(tokens: &[Token<'_>], stream: &Stream<'_>, p: usize, length: usize) -> Range<usize> {
    let first = &tokens[stream.eligible[p]];
    let last = &tokens[stream.eligible[p + length - 1]];
    first.start..last.start + last.text.len()
}

/// The pairs of positions, as (file, position, file, position), where the
/// same `RUN_TOKENS` tokens start in two places of linked files, within one
/// segment each, apart and among at most `RUN_OCCURRENCES` places.
fn matched_starts(
    streams: &[Stream<'_>],
    linked: impl Fn(usize, usize) -> bool,
) -> BTreeSet<(u32, u32, u32, u32)> {
    let mut shingles: Vec<(u64, u32, u32)> = Vec::new();
    for (f, stream) in streams.iter().enumerate() {
        for p in 0..stream.eligible.len().saturating_sub(RUN_TOKENS - 1) {
            let last = p + RUN_TOKENS - 1;
            if stream.segment[p] == stream.segment[last] {
                let hash = crate::analysis::fast_hash(&stream.normal[p..=last]);
                shingles.push((hash, f as u32, p as u32));
            }
        }
    }
    shingles.sort_unstable();
    let mut starts = BTreeSet::new();
    for group in shingles.chunk_by(|a, b| a.0 == b.0) {
        if group.len() < 2 || group.len() > RUN_OCCURRENCES {
            continue;
        }
        for (i, &(_, fx, px)) in group.iter().enumerate() {
            for &(_, fy, py) in &group[i + 1..] {
                let apart = fx != fy || px.abs_diff(py) as usize >= RUN_TOKENS;
                if apart && linked(fx as usize, fy as usize) {
                    starts.insert((fx, px, fy, py));
                }
            }
        }
    }
    starts
}

/// Each file's stream: the tokens of its blocks' statements, without those
/// of a walk's frame or a Go error check, at any depth.
fn streams<'a>(parsed: &[Parsed<'a>], blocks: &[Block]) -> Vec<Stream<'a>> {
    let mut kept = vec![BTreeSet::<usize>::new(); parsed.len()];
    let mut cut = vec![BTreeSet::<usize>::new(); parsed.len()];
    for block in blocks {
        for statement in &block.statements {
            let into = if statement.frame.is_some() || statement.idiom {
                &mut cut[block.file]
            } else {
                &mut kept[block.file]
            };
            into.extend(statement.tokens.clone());
        }
    }
    parsed
        .iter()
        .zip(kept.iter().zip(&cut))
        .map(|(file, (kept, cut))| {
            let mut stream = Stream {
                eligible: Vec::new(),
                segment: Vec::new(),
                normal: Vec::new(),
            };
            for &i in kept.difference(cut) {
                let begins = stream
                    .eligible
                    .last()
                    .is_none_or(|&previous| previous + 1 != i);
                let start = if begins {
                    stream.eligible.len()
                } else {
                    *stream.segment.last().unwrap_or(&0)
                };
                stream.segment.push(start);
                stream.eligible.push(i);
                stream.normal.push(normal_text(&file.tokens, i));
            }
            stream
        })
        .collect()
}

/// A token as runs compare it: a local name as `LOCAL`, a member name, a
/// literal or any other token as written.
fn normal_text<'a>(tokens: &[Token<'a>], i: usize) -> &'a str {
    let member = i > 0 && matches!(tokens[i - 1].text, "." | "?." | "::" | "->");
    match tokens[i].kind {
        TokenKind::Identifier if !member => LOCAL,
        _ => tokens[i].text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_run_needs_three_words_beside_names_and_punctuation() {
        assert_eq!(
            words(&["if", LOCAL, ".", "kind", "===", "settlement", "||"]),
            3
        );
        assert_eq!(words(&[LOCAL, "=", LOCAL, "(", LOCAL, ")"]), 0);
    }
}
