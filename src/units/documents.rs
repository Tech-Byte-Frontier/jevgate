//! Large project documents, judged by their outline: the headings in order,
//! never the text, so a long document costs a few hundred tokens. A Score on
//! whether splitting it would make it easier to find and maintain, and a Noul
//! on whether it mainly records past work; a split finding is then located
//! with one Choice among its top-level parts.
use super::{
    Block, Detail, FileContext, FilePlan, Planned, Presence, Questions, UnitPlan, identity,
};
use crate::{
    catalog::LARGE_DOCS,
    docs::markdown::{self, Heading},
    schema::Pass,
};
use serde_json::{Value, json};

/// Documents shorter than this are too small for this rule; the floor only
/// scopes the rule and never decides a finding.
pub(crate) const LARGE_DOC_LINES: usize = 300;
/// Deeper headings are left out of an outline longer than this.
const OUTLINE_HEADINGS: usize = 400;
const UNIT: &str = "document";

pub(super) fn plan(file: &FileContext<'_>, out: &mut FilePlan, requests: &mut Vec<Planned>) {
    out.rules.insert(LARGE_DOCS, 0);
    let lines = file.source.lines().count().max(1);
    let headings = markdown::headings(file.source);
    let name = file.path.display().to_string();
    let texts: Vec<&str> = headings.iter().map(|h| h.text.as_str()).collect();
    let mut unit = UnitPlan {
        rule: LARGE_DOCS,
        id: UNIT.into(),
        name: name.clone(),
        presence: Presence::Judged,
        locations: vec![file.location(1, lines, None)],
        quote: None,
        lines,
        identity: identity(&texts),
        detail: Detail::Document {
            parts: Vec::new(),
            locate: None,
        },
        recheck: None,
    };
    if lines < LARGE_DOC_LINES || headings.len() < 2 {
        unit.presence = Presence::TooSmall;
        out.units.push(unit);
        return;
    }
    let shown = shallow(&headings);
    let parts = parts(file, &shown, lines);
    let (request, asked) = first_request(file, &shown);
    if !file.budget.fits(&request) {
        unit.presence = Presence::NeedsContext;
        out.units.push(unit);
        return;
    }
    let locate = (parts.len() >= 2)
        .then(|| locate_request(file, &shown, &parts))
        .filter(|(request, _)| file.budget.fits(request));
    unit.detail = Detail::Document { parts, locate };
    out.units.push(unit);
    requests.push(Planned {
        owner: file.owner,
        request,
        asked,
    });
}

/// The headings shown: every level while the outline stays short enough,
/// else only the shallowest levels.
fn shallow(headings: &[Heading]) -> Vec<&Heading> {
    let mut depth = 6;
    while depth > 1 && headings.iter().filter(|h| h.level <= depth).count() > OUTLINE_HEADINGS {
        depth -= 1;
    }
    headings.iter().filter(|h| h.level <= depth).collect()
}

/// The shallowest level below the title, each heading of it opening a part.
fn top_level(shown: &[&Heading]) -> usize {
    shown.iter().skip(1).map(|h| h.level).min().unwrap_or(1)
}

fn parts(file: &FileContext<'_>, shown: &[&Heading], lines: usize) -> Vec<Block> {
    let top = top_level(shown);
    let starts: Vec<&&Heading> = shown.iter().filter(|h| h.level == top).collect();
    starts
        .iter()
        .enumerate()
        .map(|(i, heading)| {
            let end = starts.get(i + 1).map_or(lines, |next| next.line - 1);
            Block {
                id: format!("P{}", i + 1),
                location: file.location(heading.line, end.max(heading.line), Some(&heading.text)),
            }
        })
        .collect()
}

fn outline(shown: &[&Heading], ids: bool) -> Vec<Value> {
    let top = top_level(shown);
    let mut part = 0;
    shown
        .iter()
        .map(|h| {
            let mut item = json!({"heading": format!("{} {}", "#".repeat(h.level), h.text)});
            if ids && h.level == top {
                part += 1;
                item["id"] = json!(format!("P{part}"));
            }
            item
        })
        .collect()
}

fn first_request(file: &FileContext<'_>, shown: &[&Heading]) -> (Value, super::Asked) {
    let mut questions = Questions::default();
    for (question, body) in [
        ("split", super::questions::document_split()),
        ("history", super::questions::document_history()),
    ] {
        questions.ask(
            question.into(),
            body,
            UNIT,
            LARGE_DOCS,
            question,
            Pass::First,
        );
    }
    let state = json!({"file": {"path": file.path}, "outline": outline(shown, false)});
    file.request("docs", state, questions)
}

fn locate_request(
    file: &FileContext<'_>,
    shown: &[&Heading],
    parts: &[Block],
) -> (Value, super::Asked) {
    let ids: Vec<String> = parts.iter().map(|p| p.id.clone()).collect();
    let mut questions = Questions::default();
    questions.ask(
        "part".into(),
        super::questions::document_part(&ids),
        UNIT,
        LARGE_DOCS,
        "part",
        Pass::Locate,
    );
    let state = json!({"file": {"path": file.path}, "outline": outline(shown, true)});
    file.request("locate", state, questions)
}
