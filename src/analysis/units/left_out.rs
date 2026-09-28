//! What a file's syntax errors leave out of the judgment: each unit whose
//! syntax holds an error, by name, and each error outside every unit, by its
//! lines. The rest of the file is judged, and every rule asks
//! `FileUnits::intact` whether its own candidate lies clear of what was left
//! out. Most errors are grammar gaps in valid code (`syntax::error_regions`).
use super::{Definition, FileUnits, Kind, Unit, line_of};
use crate::analysis::test_map::TestCase;
use std::ops::Range;
use tree_sitter::Node;

/// A unit a syntax error left out, or code outside every unit that holds one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeftOut {
    /// The unit's name, a definition's or a test's; empty for code outside
    /// every unit.
    pub name: String,
    /// Its bytes, from the documentation above a definition.
    pub span: Range<usize>,
    pub line: usize,
    pub end_line: usize,
    /// The line of its first syntax error.
    pub error_line: usize,
}

impl LeftOut {
    /// A definition whose `node` holds a syntax error, named and placed as
    /// its unit would have been (`Unit::placed`), at its first error.
    fn definition(placed: Unit, node: Node<'_>, source: &str) -> Self {
        let error = crate::syntax::error_regions(node)
            .first()
            .map_or(node.start_byte(), |region| region.start);
        Self {
            name: placed.name,
            span: placed.span,
            line: placed.line,
            end_line: placed.end_line,
            error_line: line_of(source, error),
        }
    }

    /// Code outside every unit holding the syntax error at `region`.
    fn outside(region: &Range<usize>, lines: &Lines) -> Self {
        let line = lines.line(region.start);
        Self {
            name: String::new(),
            span: region.clone(),
            line,
            end_line: lines.last_line(region),
            error_line: line,
        }
    }
}

/// Where each line of a file starts, to place many bytes in one pass over
/// it: a file of 9,000 error regions took 0.7 s counting the lines before
/// each one.
struct Lines(Vec<usize>);

impl Lines {
    fn of(source: &str) -> Self {
        let starts = source.match_indices('\n').map(|(at, _)| at + 1);
        Self(std::iter::once(0).chain(starts).collect())
    }

    /// The line holding `byte`, as `line_of` counts it.
    fn line(&self, byte: usize) -> usize {
        self.0.partition_point(|&start| start <= byte)
    }

    /// The line of a span's last byte; an empty span's own line.
    fn last_line(&self, span: &Range<usize>) -> usize {
        self.line(span.end.saturating_sub(1).max(span.start))
    }
}

/// Whether two byte ranges share a byte, counting an empty range (a token
/// the parser assumed missing) as the byte at its position.
fn overlaps(left_out: &Range<usize>, span: &Range<usize>) -> bool {
    left_out.start < span.end && span.start < left_out.end.max(left_out.start + 1)
}

fn contains(outer: &Range<usize>, inner: &Range<usize>) -> bool {
    outer.start <= inner.start && inner.end <= outer.end
}

impl FileUnits {
    /// A definition placed as its unit (`Unit::placed`), or none when its
    /// syntax holds an error: it is then left out and named, and the rest
    /// of its file is judged. Every walk, the generic tier's included,
    /// places its definitions here.
    pub(super) fn place(
        &mut self,
        definition: Definition<'_>,
        names: (&str, &str),
        kind: Kind,
        source: &str,
    ) -> Option<Unit> {
        let placed = Unit::placed(definition, names, kind, source);
        if !definition.outer.has_error() {
            return Some(placed);
        }
        let left_out = LeftOut::definition(placed, definition.outer, source);
        self.left_out.push(left_out);
        None
    }

    /// Record the syntax errors under `root` outside every unit the walk
    /// left out: code outside every unit.
    pub(super) fn record_errors(&mut self, root: Node<'_>) {
        self.errors = crate::syntax::error_regions(root)
            .into_iter()
            .filter(|r| !self.left_out.iter().any(|l| contains(&l.span, r)))
            .collect();
    }

    /// Leave out the module constants on the lines syntax errors left out.
    pub(super) fn leave_out_constants(&mut self, source: &str) {
        if !self.partial() {
            return;
        }
        let left_out = self.left_out_code(source);
        self.constants.retain(|c| {
            !left_out
                .iter()
                .any(|l| l.line <= c.end_line && c.line <= l.end_line)
        });
    }

    /// Whether the parse held syntax errors.
    pub fn partial(&self) -> bool {
        !self.left_out.is_empty() || !self.errors.is_empty()
    }

    /// Whether `span` lies clear of everything a syntax error left out, so a
    /// rule may judge what it holds.
    pub fn intact(&self, span: &Range<usize>) -> bool {
        !self.left_out.iter().any(|l| overlaps(&l.span, span))
            && !self.errors.iter().any(|e| overlaps(e, span))
    }

    /// The share of the file's non-blank lines outside what syntax errors
    /// left out, documentation above a left-out unit included: 1.0 for a
    /// clean parse.
    pub fn coverage(&self, source: &str) -> f64 {
        let lines: Vec<&str> = source.split('\n').collect();
        let index = Lines::of(source);
        // Whether each line, from 1, is left out: each entry marks its own
        // lines, so a file with thousands of errors stays linear.
        let mut out = vec![false; lines.len() + 1];
        for l in self.left_out_code(source) {
            let first = index.line(l.span.start);
            for mark in &mut out[first.min(l.end_line)..=l.end_line.min(lines.len())] {
                *mark = true;
            }
        }
        let code: Vec<bool> = lines
            .iter()
            .enumerate()
            .filter(|(_, line)| !line.trim().is_empty())
            .map(|(index, _)| out[index + 1])
            .collect();
        if code.is_empty() {
            return 1.0;
        }
        let left = code.iter().filter(|&&left| left).count();
        1.0 - left as f64 / code.len() as f64
    }

    /// Leave out the test cases whose syntax holds an error, with the units
    /// inside them: a Bend 2 test is its whole program, defs included. Each
    /// is named in place of the errors it holds.
    pub fn leave_out_tests(&mut self, cases: Vec<TestCase>, source: &str) {
        let lines = Lines::of(source);
        for case in cases {
            if self.left_out.iter().any(|l| contains(&l.span, &case.span)) {
                continue;
            }
            // Its first error: outside every unit, or in a unit it holds.
            let loose = self.errors.iter().filter(|e| contains(&case.span, e));
            let held = self
                .left_out
                .iter()
                .filter(|l| contains(&case.span, &l.span));
            let error_line = loose
                .map(|e| lines.line(e.start))
                .chain(held.map(|l| l.error_line))
                .min()
                .unwrap_or(case.line);
            self.units.retain(|u| !contains(&case.span, &u.span));
            self.left_out.retain(|l| !contains(&case.span, &l.span));
            self.errors.retain(|e| !contains(&case.span, e));
            self.left_out.push(LeftOut {
                name: case.name,
                span: case.span,
                line: case.line,
                end_line: case.end_line,
                error_line,
            });
        }
        self.left_out.sort_by_key(|l| (l.span.start, l.span.end));
    }

    /// Everything syntax errors left out, in source order: the units, then
    /// the errors outside them joined into runs of lines, since one broken
    /// passage can hold a hundred regions.
    pub fn left_out_code(&self, source: &str) -> Vec<LeftOut> {
        let lines = Lines::of(source);
        let mut runs: Vec<LeftOut> = Vec::new();
        for error in &self.errors {
            match runs.last_mut() {
                Some(run) if lines.line(error.start) <= run.end_line + 1 => {
                    run.span.end = run.span.end.max(error.end);
                    run.end_line = run.end_line.max(lines.last_line(error));
                }
                _ => runs.push(LeftOut::outside(error, &lines)),
            }
        }
        let mut found = self.left_out.clone();
        found.extend(runs);
        found.sort_by_key(|l| (l.span.start, l.span.end));
        found
    }
}
