//! Comments: one unit per comment of application code, with the code it is
//! about. Per comment, a Score on whether it only repeats its code, and for
//! a long one a Score on whether sentences add nothing; Nouls on whether it
//! narrates an edit and, for one that reads like code, whether it is code
//! turned off. An undecided comment is asked again with the whole definition
//! it sits in, then, alone, what kind of comment it is.
use super::{
    Asked, Detail, FileContext, FilePlan, PACK_ITEMS, Planned, Presence, Questions, UnitPlan,
    compact, identity, pack, questions,
};
use crate::{
    analysis::{
        comments::{Comment, Placement},
        units::Unit,
    },
    catalog::COMMENTS,
    schema::Pass,
};
use serde_json::{Value, json};

/// Comments judged per file, at most; the rest are counted as omitted.
pub(super) const MAX_COMMENTS: usize = 80;

/// The name of the unit a comment outside every definition belongs to.
pub(super) const TOP_LEVEL: &str = "top-level code";

/// The first-pass questions of a comment; `verbose` only of a long one and
/// `disabled` only of one that reads like code.
pub(super) const QUESTIONS: [&str; 4] = ["restates", "verbose", "history", "disabled"];

pub(super) fn plan(
    file: &FileContext<'_>,
    units: &[Unit],
    comments: &[Comment],
    out: &mut FilePlan,
    requests: &mut Vec<Planned>,
) {
    let omitted = comments.len().saturating_sub(MAX_COMMENTS);
    *out.rules.entry(COMMENTS).or_default() += omitted;
    let mut seen = std::collections::BTreeMap::<String, usize>::new();
    let mut items = Vec::new();
    for comment in comments.iter().take(MAX_COMMENTS) {
        let unit = comment.unit.map(|i| &units[i]);
        let owner = unit.map_or_else(
            || comment.definition.as_deref().unwrap_or(TOP_LEVEL),
            |u| u.name.as_str(),
        );
        let count = seen.entry(owner.to_string()).or_default();
        *count += 1;
        let id = format!("comment:{owner}#{count}");
        let state = comment_state(comment, unit);
        let recheck = unit.and_then(|u| {
            let source = u.source(file.source);
            let mut state = state.clone();
            state["function_source"] = json!(source);
            let entry = Entry {
                id: id.clone(),
                state,
                code_like: comment.code_like,
                words: comment.words,
            };
            let (request, asked) = build(file, &[entry], Pass::Recheck);
            file.budget.fits(&request).then_some((request, asked))
        });
        let kind = kind(file, &id, &state, unit);
        out.units.push(UnitPlan {
            rule: COMMENTS,
            id: id.clone(),
            name: owner.to_string(),
            presence: Presence::Judged,
            locations: vec![file.location(comment.line, comment.end_line, Some(owner))],
            quote: Some(comment.text.clone()),
            lines: comment.end_line + 1 - comment.line,
            identity: identity(&[owner, &compact(&comment.text)]),
            detail: Detail::Comment {
                owner: owner.to_string(),
                documentation: matches!(
                    comment.placement,
                    Placement::Declaration | Placement::File
                ),
                kind,
            },
            recheck,
        });
        items.push((
            out.units.len() - 1,
            Entry {
                id,
                state,
                code_like: comment.code_like,
                words: comment.words,
            },
        ));
    }
    for group in pack(items, PACK_ITEMS, |(_, entry)| &entry.state) {
        let entries: Vec<Entry> = group.iter().map(|(_, e)| e.clone()).collect();
        let (request, asked) = build(file, &entries, Pass::First);
        if file.budget.fits(&request) {
            requests.push(Planned {
                owner: file.owner,
                request,
                asked,
            });
            continue;
        }
        // A pack that is too large is sent one comment at a time.
        for (index, entry) in group {
            let (request, asked) = build(file, &[entry], Pass::First);
            if file.budget.fits(&request) {
                requests.push(Planned {
                    owner: file.owner,
                    request,
                    asked,
                });
            } else {
                let unit = &mut out.units[index];
                unit.presence = Presence::NeedsContext;
                unit.recheck = None;
            }
        }
    }
}

/// What kind of comment it is, alone, with the definition it sits in when
/// that fits.
fn kind(
    file: &FileContext<'_>,
    id: &str,
    comment: &Value,
    unit: Option<&Unit>,
) -> Option<(Value, Asked)> {
    let ask = |source: Option<&str>| {
        let mut state = json!({"file": file.plain_state(), "comment": comment});
        if let Some(source) = source {
            state["function_source"] = json!(source);
        }
        let mut questions = Questions::default();
        questions.ask(
            "kind".into(),
            questions::comment_kind(source.is_some()),
            id,
            COMMENTS,
            "kind",
            Pass::Settle,
        );
        file.request("settle", state, questions)
    };
    unit.map(|u| ask(Some(u.source(file.source))))
        .filter(|(request, _)| file.budget.fits(request))
        .or_else(|| Some(ask(None)).filter(|(request, _)| file.budget.fits(request)))
}

/// Where a comment sits, in words.
fn placement_text(placement: Placement) -> &'static str {
    match placement {
        Placement::Declaration => "documentation directly above the declaration in `code`",
        Placement::File => {
            "documentation at the top of the file; `code` lists the file's definitions"
        }
        Placement::Above => "on its own lines next to the lines in `code`",
        Placement::Trailing => "at the end of the line in `code`",
    }
}

fn comment_state(comment: &Comment, unit: Option<&Unit>) -> Value {
    let mut state = json!({
        "text": comment.text,
        "placement": placement_text(comment.placement),
        "code": comment.code,
    });
    // The definition a comment inside a body sits in, by its signature.
    if let Some(unit) = unit.filter(|_| comment.placement != Placement::Declaration) {
        state["in"] = json!(unit.signature);
    }
    state
}

/// One comment as its requests send it.
#[derive(Clone)]
struct Entry {
    id: String,
    state: Value,
    /// Asked whether it is code turned off: only a comment whose lines read
    /// like statements. Asked of every comment, the question stayed near
    /// 0.5 on docstrings holding usage examples, and left their comments
    /// undecided.
    code_like: bool,
    /// Asked whether sentences add nothing: only a comment of
    /// `VERBOSE_WORDS` or more. On a two-word trailing comment the question
    /// has no sentences to weigh and stayed spread over its levels.
    words: usize,
}

/// Words a comment needs before it is asked whether sentences add nothing.
const VERBOSE_WORDS: usize = 20;

/// One request asking every question about each comment, or, as a recheck,
/// about one comment with the whole definition it sits in.
fn build(file: &FileContext<'_>, entries: &[Entry], pass: Pass) -> (Value, Asked) {
    let recheck = pass == Pass::Recheck;
    let mut questions = Questions::default();
    for (index, entry) in entries.iter().enumerate() {
        let (id, path) = (&entry.id, format!("comments[{index}]"));
        for question in QUESTIONS {
            if (question == "disabled" && !entry.code_like)
                || (question == "verbose" && entry.words < VERBOSE_WORDS)
            {
                continue;
            }
            let body = match question {
                "restates" => questions::comment_restates(&path, recheck),
                "verbose" => questions::comment_verbose(&path, recheck),
                "history" => questions::comment_history(&path),
                _ => questions::comment_disabled(&path),
            };
            questions.ask(
                format!("c{index}_{question}"),
                body,
                id,
                COMMENTS,
                question,
                pass,
            );
        }
    }
    let mut state = json!({
        "file": file.plain_state(),
        "comments": entries.iter().map(|entry| {
            let mut state = entry.state.clone();
            if let Some(object) = state.as_object_mut() {
                object.remove("function_source");
            }
            state
        }).collect::<Vec<_>>(),
    });
    if recheck && let Some(source) = entries[0].state.get("function_source") {
        state["function_source"] = source.clone();
    }
    let stage = if recheck { "recheck" } else { "comments" };
    file.request(stage, state, questions)
}
