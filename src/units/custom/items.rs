//! The units of each kind a custom question can ask about in one file, with
//! the evidence it reads. Each item's state is the one the built-in stage
//! for the same unit sends, so a question can ride in that stage's request.
use super::hunks::Hunk;
use crate::{
    analysis::{line_of, test_map::TestCase, units::Unit},
    schema::Location,
    units::{FileContext, comments, compact, identity, instructions, test_units, unique_ids},
};
use serde_json::{Value, json};
use std::ops::Range;

/// One unit's evidence as a custom question reads it.
pub(super) struct Item {
    /// Its entry in the state's list for its kind (`functions`, `tests`, …),
    /// or for a whole file, its source.
    pub state: Value,
    pub name: String,
    /// Unique among the file's items of its kind.
    pub id: String,
    pub location: Location,
    /// The lines its evidence covers, which a change must touch for a check
    /// with `--base` to ask about it: a definition's documentation,
    /// attributes and decorators with it.
    pub reach: (usize, usize),
    pub lines: usize,
    /// What its findings' fingerprints keep across unrelated edits.
    pub identity: String,
    pub quote: Option<String>,
    /// Where runs of packed items end, as the built-in stage keys them: a
    /// definition's name, a comment's owner, a heading, a hunk's content.
    pub run: String,
}

/// Functions and methods outside `tests`, any size.
pub(super) fn functions(
    file: &FileContext<'_>,
    units: &[Unit],
    tests: &[Range<usize>],
) -> Vec<Item> {
    let judged: Vec<&Unit> = units
        .iter()
        .filter(|u| u.callable() && !tests.iter().any(|l| u.overlaps(l)))
        .collect();
    let ids = unique_ids("function", judged.iter().map(|u| u.name.as_str()));
    judged
        .into_iter()
        .zip(ids)
        .map(|(unit, id)| {
            let source = unit.source(file.source);
            Item {
                state: json!({"name": unit.name, "source": source}),
                name: unit.name.clone(),
                id,
                location: file.location(unit.line, unit.end_line, Some(&unit.name)),
                reach: (line_of(file.source, unit.span.start), unit.end_line),
                lines: unit.lines(),
                identity: identity(&[&unit.name, &compact(source)]),
                quote: None,
                run: unit.name.clone(),
            }
        })
        .collect()
}

/// Test cases.
pub(super) fn tests(file: &FileContext<'_>, cases: &[TestCase]) -> Vec<Item> {
    let ruby = file.path.extension().is_some_and(|e| e == "rb");
    let ids = unique_ids("test", cases.iter().map(|c| c.name.as_str()));
    cases
        .iter()
        .zip(ids)
        .map(|(case, id)| {
            let source = case.source(file.source);
            Item {
                state: test_units::test_item(case, source, ruby),
                name: case.name.clone(),
                id,
                location: file.location(case.line, case.end_line, Some(&case.name)),
                reach: (line_of(file.source, case.span.start), case.end_line),
                lines: case.end_line + 1 - case.line,
                identity: identity(&[&case.name, &compact(source)]),
                quote: None,
                run: case.name.clone(),
            }
        })
        .collect()
}

/// Comments and docstrings outside `tests`, as the comments rule collects
/// them, each with the code it is about.
pub(super) fn comments(
    file: &FileContext<'_>,
    units: &[Unit],
    tests: &[Range<usize>],
) -> Vec<Item> {
    let found = crate::analysis::comments::comments(file.path, file.source, units)
        .unwrap_or_default()
        .into_iter()
        .filter(|c| !tests.iter().any(|l| l.contains(&c.line)))
        .collect::<Vec<_>>();
    let owners: Vec<&str> = found
        .iter()
        .map(|c| comments::owner(c, c.unit.map(|i| &units[i])))
        .collect();
    let ids = unique_ids("comment", owners.iter().copied());
    found
        .iter()
        .zip(owners)
        .zip(ids)
        .map(|((comment, owner), id)| Item {
            state: comments::comment_state(comment, comment.unit.map(|i| &units[i])),
            name: owner.to_string(),
            id,
            location: file.location(comment.line, comment.end_line, Some(owner)),
            reach: (comment.line, comment.end_line),
            lines: comment.end_line + 1 - comment.line,
            identity: identity(&[owner, &compact(&comment.text)]),
            quote: Some(comment.text.clone()),
            run: owner.to_string(),
        })
        .collect()
}

/// Heading sections of a document with text under their heading.
pub(super) fn sections(file: &FileContext<'_>) -> Vec<Item> {
    let sections: Vec<_> = crate::docs::markdown::parse(file.source)
        .sections
        .into_iter()
        .filter(|s| !s.text.trim().is_empty())
        .collect();
    let names: Vec<String> = sections
        .iter()
        .map(|s| match s.heading.as_str() {
            "" => instructions::PREAMBLE.to_string(),
            heading => heading.to_string(),
        })
        .collect();
    let ids = unique_ids("section", names.iter().map(String::as_str));
    sections
        .iter()
        .zip(names)
        .zip(ids)
        .map(|((section, name), id)| Item {
            state: json!({"heading": section.heading, "text": section.text}),
            location: file.location(section.start_line, section.end_line, Some(&name)),
            reach: (section.start_line, section.end_line),
            lines: section.end_line + 1 - section.start_line,
            identity: identity(&[&section.heading, &compact(&section.text)]),
            quote: None,
            run: section.heading.clone(),
            name,
            id,
        })
        .collect()
}

/// The whole file. Its findings are identified by its path alone: a
/// judgment of all of it has no smaller part to follow through edits.
pub(super) fn whole(file: &FileContext<'_>) -> Item {
    let lines = file.source.lines().count().max(1);
    Item {
        state: json!({"source": file.source}),
        name: file.path.display().to_string(),
        id: "file".into(),
        location: file.location(1, lines, None),
        reach: (1, lines),
        lines,
        identity: identity(&["file"]),
        quote: None,
        run: String::new(),
    }
}

/// Changed hunks. A hunk's findings are identified by the definition Git
/// names for it and what it changes, without line numbers, so they follow
/// the change when lines above it move.
pub(super) fn changed(file: &FileContext<'_>, hunks: &[Hunk]) -> Vec<Item> {
    let labels: Vec<String> = hunks.iter().map(Hunk::lines).collect();
    let ids = unique_ids("hunk", labels.iter().map(String::as_str));
    hunks
        .iter()
        .zip(labels)
        .zip(ids)
        .map(|((hunk, label), id)| {
            let mut state = json!({"lines": label, "diff": hunk.diff});
            if let Some(context) = &hunk.context {
                state["in"] = json!(context);
            }
            let changed = identity(&[
                hunk.context.as_deref().unwrap_or_default(),
                &compact(&hunk.changed),
            ]);
            Item {
                state,
                name: if hunk.start == hunk.end {
                    format!("line {label}")
                } else {
                    format!("lines {label}")
                },
                id,
                location: file.location(hunk.start, hunk.end, None),
                reach: (hunk.start, hunk.end),
                lines: hunk.end + 1 - hunk.start,
                identity: changed.clone(),
                quote: None,
                run: changed,
            }
        })
        .collect()
}
