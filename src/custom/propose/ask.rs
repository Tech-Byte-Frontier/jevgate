//! The requests of `jevgate rules propose`: two questions about each
//! candidate line, packed by heading within a file, asked through `check`'s
//! session, so answers are cached, budgeted and priced the same way.
use super::files::File;
use crate::{
    custom::Kind,
    evaluate::Session,
    options::CheckArgs,
    schema::Answer,
    units::questions::{PROPOSAL_UNITS, proposal_convention, proposal_unit},
};
use serde_json::{Map, Value, json};
use std::{collections::BTreeMap, path::Path};

/// The stage its requests record, in local metadata only.
const STAGE: &str = "propose";

/// What one candidate's answers say.
#[derive(Clone, Debug, PartialEq)]
pub struct Answered {
    /// The probability that it states a rule one piece of the code shows.
    pub convention: f64,
    /// The probability of each unit a reviewer would read to check it.
    pub units: BTreeMap<Kind, f64>,
}

impl Answered {
    /// The likeliest unit and its probability.
    pub fn unit(&self) -> (Kind, f64) {
        self.units
            .iter()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .map_or((Kind::Function, 0.0), |(kind, p)| (*kind, *p))
    }
}

/// The requests, and where each candidate's answers are.
pub struct Plan {
    pub requests: Vec<Value>,
    /// For each file, for each of its lines: its request and its place there.
    pub places: Vec<Vec<(usize, usize)>>,
}

/// Each file's lines packed in runs that end after a heading, as the
/// agent-context stage packs sections: an edit re-asks only its own run.
/// The state holds no path and no line number, so moved lines and a copy of
/// a file (an AGENTS.md beside a CLAUDE.md with the same text) ask nothing
/// new, and a request identical to an earlier one is asked once.
pub fn plan(files: &[File], model: &str) -> Plan {
    let mut requests = Vec::new();
    let mut keys = BTreeMap::new();
    let mut places = Vec::new();
    for file in files {
        let items: Vec<(usize, Value)> = file.lines.iter().map(state).enumerate().collect();
        let mut place = vec![(0, 0); items.len()];
        let packs = crate::units::pack_runs(
            items,
            |(_, state)| state["heading"].as_str().unwrap_or_default(),
            |(_, state)| state,
            |_| true,
        );
        for pack in packs {
            let request = request(model, file, &pack);
            let key = crate::requests::provider_request(&request).to_string();
            let at = *keys.entry(key).or_insert_with(|| {
                requests.push(request);
                requests.len() - 1
            });
            for (position, (line, _)) in pack.iter().enumerate() {
                place[*line] = (at, position);
            }
        }
        places.push(place);
    }
    Plan { requests, places }
}

/// What Jev is told about one candidate.
fn state(line: &super::lines::Line) -> Value {
    let mut state = json!({"heading": line.heading, "text": line.text});
    if let Some(lead_in) = &line.lead_in {
        state["lead_in"] = json!(lead_in);
    }
    state
}

/// One pack's request: whether each candidate is a rule, and on what unit.
fn request(model: &str, file: &File, pack: &[(usize, Value)]) -> Value {
    let mut questions = Map::new();
    for position in 0..pack.len() {
        let candidate = format!("candidates[{position}]");
        questions.insert(
            format!("c{position}_convention"),
            proposal_convention(&candidate),
        );
        questions.insert(format!("c{position}_unit"), proposal_unit(&candidate));
    }
    let candidates: Vec<&Value> = pack.iter().map(|(_, state)| state).collect();
    json!({
        "model": model,
        "state": {"candidates": candidates},
        "questions": questions,
        "jevgate": {
            "stage": STAGE,
            "sources": [{"path": file.path, "source_hash": file.source_hash}],
        },
    })
}

/// A dry run's count: requests, those the cache answers, and the new input
/// tokens of the rest, each with only the questions the cache lacks.
pub struct Price {
    pub requests: usize,
    pub cached: usize,
    pub tokens: u64,
}

pub fn price(plan: &Plan, root: &Path, args: &CheckArgs) -> Price {
    let budget = crate::token_budget::TokenBudget::load(root);
    let mut price = Price {
        requests: plan.requests.len(),
        cached: 0,
        tokens: 0,
    };
    for request in &plan.requests {
        match crate::requests::unanswered(root, args, request) {
            None => price.cached += 1,
            Some(sent) => price.tokens += budget.request_tokens(&sent) as u64,
        }
    }
    price
}

/// Each request's answer, or why it has none.
pub struct Answers(Vec<Result<Value, String>>);

/// Ask every request, from the cache first.
pub fn ask(plan: &Plan, session: &mut Session<'_>) -> Answers {
    let requests: Vec<&Value> = plan.requests.iter().collect();
    Answers(
        session
            .queries(&requests)
            .into_iter()
            .map(|receipt| {
                receipt
                    .result
                    .map(|(body, _, _)| body)
                    .map_err(|error| format!("{error:#}"))
            })
            .collect(),
    )
}

impl Answers {
    /// The answers about the candidate at `place`, when its request has them.
    pub fn of(&self, (request, position): (usize, usize)) -> Option<Answered> {
        let body = self.0.get(request)?.as_ref().ok()?;
        let answer = |question: &str| {
            let key = format!("c{position}_{question}");
            serde_json::from_value::<Answer>(body["answers"][key].clone()).ok()
        };
        let Some(Answer::Noul { noul }) = answer("convention") else {
            return None;
        };
        let Some(Answer::Choice { probabilities, .. }) = answer("unit") else {
            return None;
        };
        let units = PROPOSAL_UNITS
            .iter()
            .filter_map(|(option, _)| Some((kind(option)?, *probabilities.get(*option)?)))
            .collect();
        Some(Answered {
            convention: noul,
            units,
        })
    }

    /// Why requests went unanswered, each reason once.
    pub fn errors(&self) -> Vec<String> {
        let mut errors: Vec<String> = Vec::new();
        for error in self.0.iter().filter_map(|answer| answer.as_ref().err()) {
            if !errors.contains(error) {
                errors.push(error.clone());
            }
        }
        errors
    }
}

/// The custom-question unit an option of the unit Choice names.
pub fn kind(option: &str) -> Option<Kind> {
    Some(match option {
        "function" => Kind::Function,
        "test" => Kind::Test,
        "comment" => Kind::Comment,
        "section" => Kind::Section,
        "file" => Kind::File,
        "change" => Kind::Hunk,
        _ => return None,
    })
}
