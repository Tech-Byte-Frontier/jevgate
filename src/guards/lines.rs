//! The lines a change added, and the markers on them that work: a
//! suppression a tool reads, a skipped or focused test.
use super::{
    Guard, Kind,
    markers::{self, TestMarker},
};
use crate::analysis::regions::{Region, Regions};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

/// The guards of the `added` lines (0-based) of the file `text` at `path`:
/// on each, the first marker that sits where it works, since a quoted
/// `# noqa` turns nothing off and one after it on the line may. Test
/// markers count in a `test_file` only. The file is parsed once a line
/// holds a marker; a language without a grammar keeps the line's reading.
pub(super) fn marked(
    path: &Path,
    text: &str,
    added: &BTreeSet<usize>,
    test_file: bool,
) -> Vec<Guard> {
    let extension = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let ruby = extension == "rb";
    let mut regions = None;
    let mut guards = Vec::new();
    for (at, (start, line)) in lines(text).enumerate() {
        if !added.contains(&at) {
            continue;
        }
        let suppressions = markers::suppressions(line, &extension)
            .into_iter()
            .map(|m| (m.at, m.region, suppression(m.what)));
        let tests = markers::test_markers(line, ruby)
            .into_iter()
            .filter(|_| test_file)
            .map(|m| (m.at, m.region, test_marker(m.what)));
        let mut candidates = suppressions.chain(tests).peekable();
        if candidates.peek().is_none() {
            continue;
        }
        let regions = regions.get_or_insert_with(|| Regions::of(path, text).ok().flatten());
        // An allow comment counts only when it accepts findings.
        let works = |(offset, region, (kind, _)): &(usize, Region, (Kind, String))| {
            (*kind != Kind::Allow || crate::suppress::accepts(line))
                && regions
                    .as_ref()
                    .is_none_or(|r| r.at(start + offset) == *region)
        };
        if let Some((_, _, (kind, message))) = candidates.find(works) {
            guards.push(Guard::new(kind, path, Some(at + 1), line.trim(), message));
        }
    }
    guards
}

fn suppression(tool: &str) -> (Kind, String) {
    if tool == markers::JEVGATE {
        (Kind::Allow, "accepts a finding".into())
    } else {
        (Kind::Suppression, format!("turns off {tool} here"))
    }
}

fn test_marker(marker: TestMarker) -> (Kind, String) {
    match marker {
        TestMarker::Skip => (Kind::SkippedTest, "skips a test".into()),
        TestMarker::Focus => (
            Kind::FocusedTest,
            "runs only the focused tests, skipping every other".into(),
        ),
    }
}

/// Each line of `text`, as `str::lines` reads them, with the byte where it
/// starts.
fn lines(text: &str) -> impl Iterator<Item = (usize, &str)> {
    let mut start = 0;
    text.split_inclusive('\n').map(move |piece| {
        let at = start;
        start += piece.len();
        (
            at,
            piece
                .strip_suffix('\n')
                .unwrap_or(piece)
                .trim_end_matches('\r'),
        )
    })
}

/// The lines of `after` (0-based) whose trimmed text `before` does not hold
/// as often: added or rewritten lines, not ones a move or rename kept.
pub(super) fn added_lines(before: &str, after: &str) -> BTreeSet<usize> {
    let mut kept: BTreeMap<&str, usize> = BTreeMap::new();
    for line in before.lines() {
        *kept.entry(line.trim()).or_default() += 1;
    }
    after
        .lines()
        .enumerate()
        .filter(|(_, line)| match kept.get_mut(line.trim()) {
            Some(n) if *n > 0 => {
                *n -= 1;
                false
            }
            _ => true,
        })
        .map(|(at, _)| at)
        .collect()
}
