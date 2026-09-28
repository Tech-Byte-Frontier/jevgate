//! Requests for the guards (`crate::guards`). A comment or string addressed
//! to a reviewer that some request of its file sends is asked, in a request
//! of its own, whether it is written to steer the reviewer, so no other
//! request changes; at 0.80 no unit asked beside it can clear. A test a
//! change rewrote is asked, with both versions, whether it now checks less.
use super::{FileContext, FilePlan, Planned, Questions, evidence::request, questions};
use crate::{
    analysis::steering::Addressed,
    guards::ChangedTest,
    schema::{Answer, Judgment, Pass},
};
use serde_json::{Value, json};
use std::collections::BTreeSet;

/// The stage and rule the guards' questions are recorded under.
pub(crate) const GUARDS: &str = "guards";
/// The stage of the steering question.
const STEERING: &str = "steering";
/// The steering question's id.
const STEERS: &str = "steers";
/// Lines of code shown on each side of a text addressed to a reviewer.
const AROUND_LINES: usize = 3;

/// A text addressed to a reviewer, asked about under `id`, and the units
/// asked in the requests that send it.
#[derive(Clone, Debug)]
pub struct Steering {
    pub id: String,
    pub line: usize,
    pub end_line: usize,
    pub text: String,
    /// Units whose answers may be the text's, not the code's: those asked in
    /// a request whose state holds it, such as every function of a pack.
    pub units: BTreeSet<String>,
}

impl Steering {
    /// Jev's answer: how likely the text is written to steer a reviewer.
    pub fn answer(&self, judgments: &[Judgment]) -> Option<f64> {
        judgments
            .iter()
            .find(|j| j.unit == self.id && j.question == STEERS)
            .and_then(|j| match j.answer {
                Answer::Noul { noul } => Some(noul),
                _ => None,
            })
    }
}

/// Ask about each of `texts` that one of the file's `requests` sends, once
/// every rule has planned them: a text no request sends steers nothing.
pub(super) fn plan_steering(
    file: &FileContext<'_>,
    texts: Vec<Addressed>,
    out: &mut FilePlan,
    requests: &mut Vec<Planned>,
) {
    if texts.is_empty() {
        return;
    }
    let sent: Vec<Steering> = {
        let senders = senders(requests, file.owner);
        texts
            .into_iter()
            .filter_map(|text| exposed(&senders, text))
            .collect()
    };
    let lines: Vec<&str> = file.source.lines().collect();
    for mut steering in sent {
        steering.id = format!("steering:{}", out.steering.len() + 1);
        let (request, asked) = steering_request(file, &steering, &lines);
        if file.budget.fits(&request) {
            requests.push(Planned {
                owner: file.owner,
                request,
                asked,
            });
            out.steering.push(steering);
        }
    }
}

/// Each of `owner`'s requests as the state the provider receives, with the
/// units it asks about.
fn senders(requests: &[Planned], owner: usize) -> Vec<(String, Vec<&str>)> {
    requests
        .iter()
        .filter(|p| p.owner == owner)
        .map(|p| {
            let units = p.asked.questions.iter().map(|q| q.unit.as_str());
            (p.request["state"].to_string(), units.collect())
        })
        .collect()
}

/// `text` with the units of the `senders` whose state holds it; none when
/// no request sends it.
fn exposed(senders: &[(String, Vec<&str>)], text: Addressed) -> Option<Steering> {
    let needle = escaped(longest_line(&text.text));
    let units: BTreeSet<String> = senders
        .iter()
        .filter(|(state, _)| state.contains(&needle))
        .flat_map(|(_, units)| units.iter().map(|u| u.to_string()))
        .collect();
    (!units.is_empty()).then_some(Steering {
        id: String::new(),
        line: text.line,
        end_line: text.end_line,
        text: text.text,
        units,
    })
}

/// The request asking whether `steering`'s text is written to steer a
/// reviewer, with the file's `lines` around it.
fn steering_request(
    file: &FileContext<'_>,
    steering: &Steering,
    lines: &[&str],
) -> (Value, super::Asked) {
    let from = steering
        .line
        .saturating_sub(AROUND_LINES + 1)
        .min(lines.len());
    let to = (steering.end_line + AROUND_LINES)
        .min(lines.len())
        .max(from);
    let state = json!({
        "file": file.plain_state(),
        "text": steering.text,
        "code": lines[from..to].join("\n"),
    });
    let mut questions = Questions::default();
    questions.ask(
        STEERS.into(),
        questions::steers(),
        &steering.id,
        GUARDS,
        STEERS,
        Pass::First,
    );
    file.request(STEERING, state, questions)
}

/// The longest line of `text`, trimmed: what a request holding the text
/// holds too, whatever it adds around each line.
fn longest_line(text: &str) -> &str {
    text.lines()
        .map(str::trim)
        .max_by_key(|line| line.len())
        .unwrap_or_default()
}

/// `text` as it reads inside a JSON string.
fn escaped(text: &str) -> String {
    let quoted = Value::String(text.to_string()).to_string();
    quoted[1..quoted.len() - 1].to_string()
}

/// The request asking whether `test` checks less than before.
pub fn weaker_request(model: &str, test: &ChangedTest) -> Value {
    let mut state = json!({
        "file": {"path": test.path, "language": crate::file_kind::language(&test.path)},
        "test": {"name": test.name, "before": test.before, "after": test.after},
    });
    if !test.helpers.is_empty() {
        state["helpers"] = json!(test.helpers);
    }
    let mut questions = Questions::default();
    questions.ask(
        "weaker".into(),
        questions::weaker(),
        &test.name,
        GUARDS,
        "weaker",
        Pass::First,
    );
    let sources = [(test.path.as_path(), test.source_hash.as_str())];
    request(model, GUARDS, &sources, state, questions).0
}

/// How likely, by Jev's answer `body`, the test now checks less.
pub fn weaker_answer(body: &Value) -> Option<f64> {
    match serde_json::from_value(body["answers"]["weaker"].clone()).ok()? {
        Answer::Noul { noul } => Some(noul),
        _ => None,
    }
}
