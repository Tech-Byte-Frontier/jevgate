//! The requests of `jevgate rules propose`: whether each candidate line is
//! a rule and on what unit, then what would check each rule, packed by
//! heading within a file and asked through `check`'s session, so answers
//! are cached, budgeted and priced the same way.
use super::files::File;
use crate::{
    custom::Kind,
    evaluate::Session,
    options::CheckArgs,
    schema::Answer,
    units::questions::{
        PROPOSAL_UNITS, TOOL_CHECKERS, proposal_checker, proposal_convention, proposal_unit,
    },
};
use serde_json::{Map, Value, json};
use std::{collections::BTreeMap, path::Path};

/// The stage its requests record, in local metadata only.
const STAGE: &str = "propose";

/// The questions one pass asks of each candidate it asks about.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Pass {
    /// Whether it states a rule, and on what unit: every candidate.
    First,
    /// What would check the rule: only the candidates the first pass
    /// calls rules.
    Checker,
}

impl Pass {
    fn questions(self, candidate: &str) -> Vec<(&'static str, Value)> {
        match self {
            Self::First => vec![
                ("convention", proposal_convention(candidate)),
                ("unit", proposal_unit(candidate)),
            ],
            Self::Checker => vec![("checker", proposal_checker(candidate))],
        }
    }
}

/// What one candidate's answers say.
#[derive(Clone, Debug, PartialEq)]
pub struct Answered {
    /// The probability that it states a rule one piece of the code shows.
    pub convention: f64,
    /// The probability of each unit a reviewer would read to check it.
    pub units: BTreeMap<Kind, f64>,
    /// The probability of each thing that would check the rule, asked only
    /// of a line the first pass calls a rule.
    pub checkers: Option<BTreeMap<String, f64>>,
}

impl Answered {
    /// The likeliest unit and its probability.
    pub fn unit(&self) -> (Kind, f64) {
        self.units
            .iter()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .map_or((Kind::Function, 0.0), |(kind, p)| (*kind, *p))
    }

    /// The probability that a formatter, linter, compiler or measuring
    /// script already checks the rule, when it was asked.
    pub fn tool_checked(&self) -> Option<f64> {
        let checkers = self.checkers.as_ref()?;
        Some(TOOL_CHECKERS.iter().filter_map(|t| checkers.get(*t)).sum())
    }
}

/// The requests of one pass, and where each candidate's answers are.
pub struct Plan {
    pub requests: Vec<Value>,
    /// For each file, for each of its lines: its request and its place
    /// there, when this pass asks about it.
    pub places: Vec<Vec<Option<(usize, usize)>>>,
}

/// The lines of each file that `asked` selects, packed in runs that end
/// after a heading, as the agent-context stage packs sections: an edit
/// re-asks only its own run. The state holds no path and no line number,
/// so moved lines and a copy of a file (an AGENTS.md beside a CLAUDE.md
/// with the same text) ask nothing new, and a request identical to an
/// earlier one is asked once.
pub fn plan(
    files: &[File],
    (model, pass): (&str, Pass),
    asked: impl Fn(usize, usize) -> bool,
) -> Plan {
    let mut requests = Vec::new();
    let mut keys = BTreeMap::new();
    let mut places = Vec::new();
    for (at_file, file) in files.iter().enumerate() {
        let items: Vec<(usize, Value)> = file
            .lines
            .iter()
            .enumerate()
            .filter(|(line, _)| asked(at_file, *line))
            .map(|(line, candidate)| (line, state(candidate)))
            .collect();
        let mut place = vec![None; file.lines.len()];
        let packs = crate::units::pack_runs(
            items,
            |(_, state)| state["heading"].as_str().unwrap_or_default(),
            |(_, state)| state,
            |_| true,
        );
        for pack in packs {
            let request = request((model, pass), file, &pack);
            let key = crate::requests::provider_request(&request).to_string();
            let at = *keys.entry(key).or_insert_with(|| {
                requests.push(request);
                requests.len() - 1
            });
            for (position, (line, _)) in pack.iter().enumerate() {
                place[*line] = Some((at, position));
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

/// One pack's request: the pass's questions about each candidate.
fn request((model, pass): (&str, Pass), file: &File, pack: &[(usize, Value)]) -> Value {
    let mut questions = Map::new();
    for position in 0..pack.len() {
        for (name, body) in pass.questions(&format!("candidates[{position}]")) {
            questions.insert(format!("c{position}_{name}"), body);
        }
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

/// A dry run's count of the first pass: requests, those the cache answers,
/// and the new input tokens of the rest, each with only the questions the
/// cache lacks.
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

/// Ask every request of a pass, from the cache first.
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
    /// The answer to `question` about the candidate at `place`.
    fn answer(&self, place: Option<(usize, usize)>, question: &str) -> Option<Answer> {
        let (request, position) = place?;
        let body = self.0.get(request)?.as_ref().ok()?;
        let key = format!("c{position}_{question}");
        serde_json::from_value(body["answers"][key].clone()).ok()
    }

    fn noul(&self, place: Option<(usize, usize)>, question: &str) -> Option<f64> {
        match self.answer(place, question)? {
            Answer::Noul { noul } => Some(noul),
            _ => None,
        }
    }

    fn choice(
        &self,
        place: Option<(usize, usize)>,
        question: &str,
    ) -> Option<BTreeMap<String, f64>> {
        match self.answer(place, question)? {
            Answer::Choice { probabilities, .. } => Some(probabilities),
            _ => None,
        }
    }

    /// Why requests went unanswered, each reason once.
    fn errors(&self) -> impl Iterator<Item = &String> {
        self.0.iter().filter_map(|answer| answer.as_ref().err())
    }
}

/// Ask the first pass about every candidate, then what would check each
/// line it calls a rule at `threshold`.
pub fn ask_both(
    files: &[File],
    (first, model): (Plan, &str),
    threshold: f64,
    session: &mut Session<'_>,
) -> Asked {
    let answers = ask(&first, session);
    let rule = |file: usize, line: usize| {
        answers
            .noul(first.places[file][line], "convention")
            .is_some_and(|p| crate::policy::probability_at_least(p, threshold))
    };
    let checker = plan(files, (model, Pass::Checker), rule);
    let checked = ask(&checker, session);
    Asked {
        first: (first, answers),
        checker: (checker, checked),
    }
}

/// Both passes, planned and answered.
pub struct Asked {
    pub first: (Plan, Answers),
    pub checker: (Plan, Answers),
}

impl Asked {
    /// The answers about line `line` of file `file`, when the first pass
    /// has them.
    pub fn answered(&self, file: usize, line: usize) -> Option<Answered> {
        let (plan, answers) = &self.first;
        let place = plan.places[file][line];
        let convention = answers.noul(place, "convention")?;
        let units = answers.choice(place, "unit")?;
        let units = PROPOSAL_UNITS
            .iter()
            .filter_map(|(option, _)| Some((kind(option)?, *units.get(*option)?)))
            .collect();
        let (plan, answers) = &self.checker;
        Some(Answered {
            convention,
            units,
            checkers: answers.choice(plan.places[file][line], "checker"),
        })
    }

    /// Why requests of either pass went unanswered, each reason once.
    pub fn errors(&self) -> Vec<String> {
        let mut errors: Vec<String> = Vec::new();
        for error in self.first.1.errors().chain(self.checker.1.errors()) {
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
