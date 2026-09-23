//! Line ranges of a source file: masking, blanking, merging, excerpting
//! and naming them.
use crate::schema::SourceRange;

/// One flag per line, set for lines inside any of `ranges`.
fn line_mask(lines: usize, ranges: &[SourceRange]) -> Vec<bool> {
    let mut mask = vec![false; lines];
    for range in ranges {
        for line in range.start_line..=range.end_line {
            if let Some(slot) = mask.get_mut(line.saturating_sub(1)) {
                *slot = true;
            }
        }
    }
    mask
}

pub(crate) fn overlaps(range: &SourceRange, ranges: &[SourceRange]) -> bool {
    ranges
        .iter()
        .any(|other| range.start_line <= other.end_line && other.start_line <= range.end_line)
}

/// Characters of a range shown as evidence before it is cut short.
const EXCERPT_CHARS: usize = 1500;

pub(crate) fn excerpt(source: &str, range: &SourceRange) -> String {
    let text = source
        .lines()
        .skip(range.start_line.saturating_sub(1))
        .take(range.end_line.saturating_sub(range.start_line) + 1)
        .collect::<Vec<_>>()
        .join("\n");
    if text.len() <= EXCERPT_CHARS {
        text
    } else {
        format!("{}…", text.chars().take(EXCERPT_CHARS).collect::<String>())
    }
}

pub(crate) fn blank_lines(source: &str, ranges: &[SourceRange]) -> String {
    if ranges.is_empty() {
        return source.to_string();
    }
    let blank = line_mask(source.lines().count(), ranges);
    let mut out = String::with_capacity(source.len());
    for (index, line) in source.lines().enumerate() {
        if blank.get(index).copied().unwrap_or(false) {
            out.push_str(&" ".repeat(line.len()));
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    if !source.ends_with('\n') {
        out.pop();
    }
    out
}

pub(crate) fn merge_ranges(mut ranges: Vec<SourceRange>) -> Vec<SourceRange> {
    ranges.sort_by_key(|range| (range.start_line, range.end_line));
    let mut merged: Vec<SourceRange> = Vec::new();
    for range in ranges {
        if let Some(last) = merged.last_mut()
            && range.start_line <= last.end_line.saturating_add(1)
        {
            last.end_line = last.end_line.max(range.end_line);
            continue;
        }
        merged.push(range);
    }
    merged
}

pub(crate) fn ranges_phrase(ranges: &[SourceRange]) -> String {
    if ranges.is_empty() {
        return String::new();
    }
    let listed = ranges
        .iter()
        .take(8)
        .map(|range| {
            if range.start_line == range.end_line {
                format!("line {}", range.start_line)
            } else {
                format!("lines {}–{}", range.start_line, range.end_line)
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!(" Separated tests: {listed}.")
}
