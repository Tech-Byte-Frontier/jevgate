//! Hardcoded values: packed function sources with the literal values each one
//! uses, and one unit per file for its module-level constants. Per function,
//! Scores on whether a value belongs in configuration or deserves a name, and
//! a Noul on whether the function special-cases one identity. A unit left
//! undecided is asked, alone, whether every value is of an acceptable kind.
use super::{
    Asked, Detail, FileContext, FilePlan, FollowUp, Planned, Presence, Questions, UnitPlan,
    compact, identity, pack_runs, questions, unique_ids,
};
use crate::{
    analysis::{literals::Constant, sites::clip, units::Unit},
    catalog::HARDCODED_VALUES,
    schema::Pass,
};
use serde_json::{Value, json};
use std::path::Path;

pub(super) fn plan(
    file: &FileContext<'_>,
    units: &[&Unit],
    constants: &[Constant],
    out: &mut FilePlan,
    requests: &mut Vec<Planned>,
) {
    let units: Vec<&Unit> = units
        .iter()
        .copied()
        .filter(|u| !u.literals.is_empty())
        .collect();
    let ids = unique_ids("values", units.iter().map(|u| u.name.as_str()));
    let mut items = Vec::new();
    for (unit, id) in units.iter().zip(ids) {
        let source = unit.source(file.source);
        let values: Vec<&str> = unit.literals.iter().map(|l| l.text.as_str()).collect();
        let state = json!({"name": unit.name, "source": source, "values": values});
        out.units.push(UnitPlan {
            rule: HARDCODED_VALUES,
            id: id.clone(),
            name: unit.name.clone(),
            presence: Presence::Judged,
            locations: vec![file.location(unit.line, unit.end_line, Some(&unit.name))],
            quote: None,
            lines: unit.lines(),
            identity: identity(&[&unit.name, &compact(source)]),
            detail: {
                let mut choices: Vec<String> = Vec::new();
                for literal in &unit.literals {
                    if !choices.contains(&literal.text) {
                        choices.push(literal.text.clone());
                    }
                }
                let locate = (choices.len() <= LOCATE_CHOICES)
                    .then(|| locate(file, (unit, source), &id, &choices));
                Detail::Values {
                    values: unit.literals.iter().map(|l| l.text.clone()).collect(),
                    repeated: choices
                        .iter()
                        .map(|c| occurrences(file.source, c) != 1)
                        .collect(),
                    choices,
                    locate: locate.map(Into::into),
                }
            },
            recheck: benign_request(file, &id, json!({"functions": [state.clone()]}), true)
                .map(Into::into),
        });
        items.push((out.units.len() - 1, id, state));
    }
    // Runs end after the names of the functions holding the values, so a
    // value added or removed re-asks only its function's run.
    let packs = pack_runs(
        items,
        |(_, _, state)| state["name"].as_str().unwrap_or_default(),
        |(_, _, state)| state,
        |(index, _, _)| file.judges_unit(&out.units[*index]),
    );
    for group in packs {
        send_or_split(file, group, out, requests);
    }
    if !constants.is_empty() {
        plan_constants(file, constants, out, requests);
    }
}

/// Most distinct values a locate Choice offers; a unit with more is not located.
const LOCATE_CHOICES: usize = 24;

/// How often a literal is written in `source`.
fn occurrences(source: &str, literal: &str) -> usize {
    written_at(source, literal).count()
}

/// Where a literal is written in `source`: a number as a whole token (not
/// part of `100` or `10.5` for `10`), other text wherever it appears without
/// its quotes.
fn written_at<'a>(source: &'a str, literal: &'a str) -> impl Iterator<Item = usize> + 'a {
    let text = literal.trim_matches(['"', '\'', '`']);
    let number = text.starts_with(|c: char| c.is_ascii_digit() || c == '-' || c == '.');
    let word = |c: char| c.is_alphanumeric() || c == '_' || c == '.';
    (!text.is_empty())
        .then_some(text)
        .into_iter()
        .flat_map(move |text| source.match_indices(text))
        .filter(move |(at, _)| {
            !number
                || !(source[..*at].chars().next_back().is_some_and(word)
                    || source[at + text.len()..].chars().next().is_some_and(word))
        })
        .map(|(at, _)| at)
}

/// Lines of the file outside a unit shown with each of its values or
/// constants, at most.
const ELSEWHERE_LINES: usize = 3;

/// The lines of `source` outside `lines` that write `literal`, numbered.
fn elsewhere(source: &str, literal: &str, lines: (usize, usize)) -> Vec<String> {
    numbered_lines(source, written_at(source, literal), lines)
}

/// The lines of `source` outside a constant's own `lines` that name it.
fn used_at(source: &str, name: &str, lines: (usize, usize)) -> Vec<String> {
    let word = |c: char| c.is_alphanumeric() || c == '_' || c == '$';
    let named = source.match_indices(name).map(|(at, _)| at).filter(|&at| {
        !(source[..at].chars().next_back().is_some_and(word)
            || source[at + name.len()..].chars().next().is_some_and(word))
    });
    numbered_lines(source, named, lines)
}

/// The lines holding the ascending byte offsets `at`, outside `lines`,
/// numbered and at most `ELSEWHERE_LINES`.
fn numbered_lines(
    source: &str,
    at: impl Iterator<Item = usize>,
    lines: (usize, usize),
) -> Vec<String> {
    let mut found: Vec<(usize, String)> = Vec::new();
    let (mut line, mut counted) = (1, 0);
    for at in at {
        line += source.as_bytes()[counted..at]
            .iter()
            .filter(|b| **b == b'\n')
            .count();
        counted = at;
        if (lines.0..=lines.1).contains(&line) || found.last().is_some_and(|(l, _)| *l == line) {
            continue;
        }
        let start = source[..at].rfind('\n').map_or(0, |i| i + 1);
        let end = source[at..].find('\n').map_or(source.len(), |i| at + i);
        found.push((line, format!("{line}: {}", clip(source[start..end].trim()))));
        if found.len() == ELSEWHERE_LINES {
            break;
        }
    }
    found.into_iter().map(|(_, text)| text).collect()
}

/// Which value a finding is about: the function's source and its distinct
/// values, each with the other lines of the file that write it. Without
/// them, a value that must stay equal to a copy in another function read as
/// clear where it was used, and a value that merely recurs, such as the 4
/// of quarters in a year, read as a value to share.
fn locate(
    file: &FileContext<'_>,
    (unit, source): (&Unit, &str),
    id: &str,
    choices: &[String],
) -> (Value, Asked) {
    let ids = option_ids('v', choices.len());
    let values: Vec<Value> = ids
        .iter()
        .zip(choices)
        .map(|(id, value)| {
            let mut entry = json!({"id": id, "value": value});
            let lines = elsewhere(file.source, value, (unit.line, unit.end_line));
            if !lines.is_empty() {
                entry["elsewhere"] = json!(lines);
            }
            entry
        })
        .collect();
    let state = json!({
        "file": file.file_state(),
        "function": {
            "name": unit.name,
            "source": source,
            "values": values,
        },
    });
    locate_request(file, id, ("value", questions::hardcoded_value(&ids)), state)
}

/// What a consider's value is, asked about the value its locate named: the
/// function, the value and the other lines of its file that write it, taken
/// from the locate request.
pub(super) fn value_kind(locate: &FollowUp, option: usize, id: &str) -> Option<(Value, Asked)> {
    let located = locate.request();
    let function = &located["state"]["function"];
    let entry = function["values"].get(option)?;
    let mut state = json!({
        "file": located["state"]["file"],
        "function": {"name": function["name"], "source": function["source"]},
        "value": entry["value"],
    });
    if let Some(lines) = entry.get("elsewhere") {
        state["elsewhere"] = lines.clone();
    }
    let bend = located["state"]["file"]["language"] == crate::analysis::bend::LANGUAGE;
    let mut questions = Questions::default();
    questions.ask(
        "value_kind".into(),
        questions::hardcoded_value_kind(bend),
        id,
        HARDCODED_VALUES,
        "value_kind",
        Pass::Locate,
    );
    Some(located_request(&located, state, questions))
}

/// Where an environment finding's value or constant would differ, asked
/// about the one its locate named: the function and the value with the
/// other lines that write it, or the constant with the lines that use it,
/// taken from the locate request.
pub(super) fn environment_kind(
    locate: &FollowUp,
    option: usize,
    id: &str,
) -> Option<(Value, Asked)> {
    let located = locate.request();
    let file = &located["state"]["file"];
    let (state, subject, note) = if let Some(constants) = located["state"]["constants"].as_array() {
        let mut constant = constants.get(option)?.clone();
        let used = constant.as_object_mut()?.remove("used_at");
        constant.as_object_mut()?.remove("id");
        let mut state = json!({"file": file, "constant": constant});
        if let Some(lines) = used {
            state["used_at"] = lines;
        }
        (
            state,
            "constant",
            "`used_at` lists lines of the file that use the constant.",
        )
    } else {
        let function = &located["state"]["function"];
        let entry = function["values"].get(option)?;
        let mut state = json!({
            "file": file,
            "function": {"name": function["name"], "source": function["source"]},
            "value": entry["value"],
        });
        if let Some(lines) = entry.get("elsewhere") {
            state["elsewhere"] = lines.clone();
        }
        (
            state,
            "value",
            "`elsewhere` lists other lines of the file that write the same value.",
        )
    };
    let mut questions = Questions::default();
    questions.ask(
        "environment_kind".into(),
        questions::hardcoded_environment_kind(subject, note),
        id,
        HARDCODED_VALUES,
        "environment_kind",
        Pass::Locate,
    );
    Some(located_request(&located, state, questions))
}

/// A follow-up of the file a locate request was made for: its model, its
/// sources and `state`, asking `questions`.
fn located_request(located: &Value, state: Value, questions: Questions) -> (Value, Asked) {
    let language = located["state"]["file"]["language"]
        .as_str()
        .unwrap_or_default();
    let sources: Vec<(&Path, &str)> = located["jevgate"]["sources"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| Some((Path::new(s["path"].as_str()?), s["source_hash"].as_str()?)))
        .collect();
    super::evidence::request(
        located["model"].as_str().unwrap_or_default(),
        "locate",
        &sources,
        state,
        questions.reworded(language),
    )
}

/// Option ids `{prefix}0`, `{prefix}1`, … for a locate Choice over `count` entries.
fn option_ids(prefix: char, count: usize) -> Vec<String> {
    (0..count).map(|i| format!("{prefix}{i}")).collect()
}

/// A locate request asking one Choice, `question` with its body, about `state`.
fn locate_request(
    file: &FileContext<'_>,
    id: &str,
    (question, body): (&'static str, Value),
    state: Value,
) -> (Value, Asked) {
    let mut questions = Questions::default();
    questions.ask(
        question.into(),
        body,
        id,
        HARDCODED_VALUES,
        question,
        Pass::Locate,
    );
    file.request("locate", state, questions)
}

/// A pack that is too large is sent one function at a time.
fn send_or_split(
    file: &FileContext<'_>,
    group: Vec<(usize, String, Value)>,
    out: &mut FilePlan,
    requests: &mut Vec<Planned>,
) {
    let (request, asked) = functions_request(file, &group);
    if file.budget.fits(&request) {
        requests.push(Planned {
            owner: file.owner,
            request,
            asked,
        });
        return;
    }
    for item in group {
        let (request, asked) = functions_request(file, std::slice::from_ref(&item));
        if file.budget.fits(&request) {
            requests.push(Planned {
                owner: file.owner,
                request,
                asked,
            });
        } else {
            out.units[item.0].presence = Presence::NeedsContext;
            out.units[item.0].recheck = None;
        }
    }
}

fn functions_request(file: &FileContext<'_>, items: &[(usize, String, Value)]) -> (Value, Asked) {
    let mut questions = Questions::default();
    for (index, (_, id, _)) in items.iter().enumerate() {
        let values = format!("functions[{index}].values");
        let code = format!("functions[{index}].source");
        for (question, body) in [
            (
                "environment",
                questions::hardcoded_environment(&values, &format!("`{code}`")),
            ),
            ("magic", questions::hardcoded_magic(&values, &code)),
            ("special", questions::hardcoded_special(&code)),
        ] {
            questions.ask(
                format!("f{index}_{question}"),
                body,
                id,
                HARDCODED_VALUES,
                question,
                Pass::First,
            );
        }
    }
    let state = json!({
        "file": file.file_state(),
        "functions": items.iter().map(|(_, _, state)| state.clone()).collect::<Vec<_>>(),
    });
    file.request("values", state, questions)
}

const CONSTANTS_ID: &str = "constants";

/// One unit for the file's module-level constants: only the environment
/// question, since a name already explains each value.
fn plan_constants(
    file: &FileContext<'_>,
    constants: &[Constant],
    out: &mut FilePlan,
    requests: &mut Vec<Planned>,
) {
    let mut questions = Questions::default();
    questions.ask(
        "environment".into(),
        questions::hardcoded_environment("constants", "the program"),
        CONSTANTS_ID,
        HARDCODED_VALUES,
        "environment",
        Pass::First,
    );
    let listed: Vec<Value> = constants
        .iter()
        .map(|c| match &c.value {
            Some(value) => json!({"name": c.name, "value": value}),
            None => json!({"name": c.name, "values": c.values}),
        })
        .collect();
    let recheck = benign_request(
        file,
        CONSTANTS_ID,
        json!({"constants": listed.clone()}),
        false,
    );
    let locate = (constants.len() <= LOCATE_CHOICES).then(|| {
        let ids = option_ids('c', constants.len());
        let with_ids: Vec<Value> = ids
            .iter()
            .zip(&listed)
            .zip(constants)
            .map(|((id, listed), constant)| {
                let mut listed = listed.clone();
                listed["id"] = json!(id);
                let lines = used_at(
                    file.source,
                    &constant.name,
                    (constant.line, constant.end_line),
                );
                if !lines.is_empty() {
                    listed["used_at"] = json!(lines);
                }
                listed
            })
            .collect();
        locate_request(
            file,
            CONSTANTS_ID,
            ("constant", questions::hardcoded_constant(&ids)),
            json!({"file": file.file_state(), "constants": with_ids}),
        )
    });
    let state = json!({"file": file.file_state(), "constants": listed});
    let (request, asked) = file.request("constants", state, questions);
    let fits = file.budget.fits(&request);
    let names: Vec<&str> = constants.iter().map(|c| c.name.as_str()).collect();
    out.units.push(UnitPlan {
        rule: HARDCODED_VALUES,
        id: CONSTANTS_ID.into(),
        name: "module constants".into(),
        presence: if fits {
            Presence::Judged
        } else {
            Presence::NeedsContext
        },
        locations: constants
            .iter()
            .map(|c| file.location(c.line, c.end_line, Some(&c.name)))
            .collect(),
        quote: None,
        lines: constants.iter().map(|c| c.end_line + 1 - c.line).sum(),
        identity: identity(&names),
        detail: Detail::Constants {
            values: constants.iter().flat_map(|c| c.values.clone()).collect(),
            locate: locate.map(Into::into),
        },
        recheck: recheck.filter(|_| fits).map(Into::into),
    });
    if fits {
        requests.push(Planned {
            owner: file.owner,
            request,
            asked,
        });
    }
}

/// The benign-kind checks for one unit, sent only when a question stayed
/// undecided: every question for a function, the environment for constants.
fn benign_request(
    file: &FileContext<'_>,
    id: &str,
    evidence: Value,
    function: bool,
) -> Option<(Value, Asked)> {
    let (values, code, asked): (&str, &str, &[&'static str]) = if function {
        (
            "functions[0].values",
            "functions[0].source",
            &["environment", "magic", "special"],
        )
    } else {
        ("constants", "the program", &["environment"])
    };
    let mut questions = Questions::default();
    for question in asked {
        questions.ask(
            question.to_string(),
            questions::hardcoded_benign(question, values, code),
            id,
            HARDCODED_VALUES,
            super::outcome::benign_key(question),
            Pass::Recheck,
        );
    }
    let mut state = evidence;
    state["file"] = file.file_state();
    let (request, asked) = file.request("recheck", state, questions);
    file.budget.fits(&request).then_some((request, asked))
}
