//! The questions an undecided unit's answers left open, quoted as they were
//! asked: the text naming the evidence, what each answer means and the
//! answer itself, so a person or a coding agent can weigh what Jev left open.
//! Only undecided units are looked up: 1,718 of 206,622 judged units on the
//! corpus (pin-0241).
use super::{Answers, UnitPlan};
use crate::{
    schema::{Answer, Judgment, OpenQuestion, Pass},
    units::{Asked, Planned},
};
use serde_json::Value;
use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
};

/// Where an undecided unit's open questions are quoted from: the file's
/// judgments, which say in which pass each answer was given, and the
/// requests its units were first asked in. Later passes were asked in
/// follow-ups, which each unit keeps.
#[derive(Clone, Copy)]
pub struct Quotes<'p> {
    pub judgments: &'p [Judgment],
    pub first: &'p [&'p Planned],
}

impl Quotes<'_> {
    /// Each of `questions` whose answer `answers` holds, as it was asked. A
    /// question whose request is not kept, such as a value's kind built from
    /// earlier answers, is left out.
    pub(super) fn open(
        &self,
        unit: &UnitPlan,
        questions: &[&str],
        answers: &Answers<'_>,
    ) -> Vec<OpenQuestion> {
        questions
            .iter()
            .filter_map(|&question| {
                let answer = *answers.get(question)?;
                // The merged answers point at the judgments they were read from.
                let pass = self
                    .judgments
                    .iter()
                    .find(|j| std::ptr::eq(&j.answer, answer))?
                    .pass;
                let (request, key) = self.request(unit, question, pass)?;
                Some(quote(question, pass, &request, &key, answer))
            })
            .collect()
    }

    /// The request that asked `question` of `unit` in `pass`, and the key the
    /// question has there. A follow-up's JSON text is read back only for the
    /// one that asked it.
    fn request(
        &self,
        unit: &UnitPlan,
        question: &str,
        pass: Pass,
    ) -> Option<(Cow<'_, Value>, String)> {
        let key = |asked: &Asked| {
            asked
                .questions
                .iter()
                .find(|q| q.unit == unit.id && q.question == question && q.pass == pass)
                .map(|q| q.key.clone())
        };
        if pass == Pass::First {
            self.first
                .iter()
                .find_map(|planned| Some((Cow::Borrowed(&planned.request), key(&planned.asked)?)))
        } else {
            unit.follow_ups().find_map(|follow_up| {
                let key = key(&follow_up.asked)?;
                Some((Cow::Owned(follow_up.request()), key))
            })
        }
    }
}

fn quote(id: &str, pass: Pass, request: &Value, key: &str, answer: &Answer) -> OpenQuestion {
    let body = &request["questions"][key];
    let text = body["instructions"]["question"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    OpenQuestion {
        id: id.into(),
        pass,
        evidence: state_paths(&text, &request["state"]),
        options: options(&body["criteria"]),
        text,
        answer: answer.clone(),
    }
}

/// What each answer means, by the names the answer's probabilities use: a
/// Score's levels by position, a Noul's `true` and `false`, a Choice's
/// options. A structured criterion is read by what it describes, without its
/// examples; an option the state defines, such as a block id, means nothing
/// by itself.
fn options(criteria: &Value) -> BTreeMap<String, String> {
    let meaning = |criterion: &Value| {
        criterion
            .as_str()
            .or_else(|| criterion["what"].as_str())
            .unwrap_or_default()
            .to_string()
    };
    match criteria {
        Value::Array(levels) => levels
            .iter()
            .enumerate()
            .map(|(level, criterion)| (level.to_string(), meaning(criterion)))
            .collect(),
        Value::Object(options) => options
            .iter()
            .map(|(option, criterion)| (option.clone(), meaning(criterion)))
            .collect(),
        _ => BTreeMap::new(),
    }
}

/// The backticked spans of a question that name a value of its request's
/// state, such as `functions[0].source`, each once; code a question quotes,
/// such as `eval`, names none.
fn state_paths(text: &str, state: &Value) -> Vec<String> {
    let mut seen = BTreeSet::new();
    text.split('`')
        .skip(1)
        .step_by(2)
        .filter(|span| resolves(state, span) && seen.insert(*span))
        .map(str::to_string)
        .collect()
}

/// Whether `path`, names and `[index]`es joined by dots, names a value in
/// `state`. An empty index (`subjects[]`) reads the first element.
fn resolves(state: &Value, path: &str) -> bool {
    let mut value = state;
    for part in path.split('.') {
        let mut pieces = part.split('[');
        let name = pieces.next().unwrap_or_default();
        if !name.is_empty() {
            match value.get(name) {
                Some(field) => value = field,
                None => return false,
            }
        }
        for index in pieces {
            let index = index.strip_suffix(']').unwrap_or(index);
            let position = if index.is_empty() {
                Some(0)
            } else {
                index.parse::<usize>().ok()
            };
            match position.and_then(|at| value.get(at)) {
                Some(element) => value = element,
                None => return false,
            }
        }
    }
    !path.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn evidence_is_the_backticked_paths_the_state_holds() {
        let state = json!({
            "functions": [{"source": "fn a() {}"}, {"source": "fn b() {}"}],
            "subjects": [{"source": "fn c() {}"}],
            "file": {"path": "a.rs"},
        });
        let text = "Does `functions[1].source` call `eval` on `file.path` or `subjects[].source`, as `functions[1].source` says, or `functions[2].source`?";
        assert_eq!(
            state_paths(text, &state),
            ["functions[1].source", "file.path", "subjects[].source"]
        );
        assert!(!resolves(&state, ""));
        assert!(!resolves(&state, "functions[x]"));
        assert!(!resolves(&state, "file.path.name"));
    }

    #[test]
    fn options_name_levels_nouls_and_choices() {
        let score = options(&json!(["No.", "Slightly.", "Yes."]));
        assert_eq!(score["0"], "No.");
        assert_eq!(score["2"], "Yes.");
        let noul = options(
            &json!({"true": {"what": "It does.", "examples": ["x"]}, "false": "It does not."}),
        );
        assert_eq!(noul["true"], "It does.");
        assert_eq!(noul["false"], "It does not.");
        let choice = options(&json!({"B1": null, "none": "No block."}));
        assert_eq!(choice["B1"], "");
        assert_eq!(choice["none"], "No block.");
    }
}
