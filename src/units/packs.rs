//! Function packs: the first-pass questions of every rule about a file's
//! functions, asked in one request per pack, so each function's source is
//! uploaded once. Function simplification, hardcoded values and the security
//! rules each packed the functions they judge, so a function all three
//! judged was sent three times.
use super::{
    Asked, FileContext, FilePlan, Planned, Questions, functions, hardcoded, pack_runs, security,
};
use crate::schema::Pass;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

/// The stage of every function pack, whatever rules it asks for: one
/// request serves several rules, so it is not counted under one of them.
const STAGE: &str = "functions";

/// One rule's first-pass questions about one function of a file.
pub(super) struct FunctionAsk {
    /// Its position among the file's parsed units, the order each rule
    /// lists its functions in; a PHP page script comes after them all.
    pub position: usize,
    pub name: String,
    pub source: String,
    pub ask: Ask,
}

/// What one rule asks about a function, with the unit its answers are
/// recorded on and the evidence its questions read beside the function's
/// name and source.
pub(super) enum Ask {
    /// Function simplification: whether splitting it would help and, where
    /// its nesting is deep, whether flattening it would.
    Split { unit: usize, nested: bool },
    /// Hardcoded values: whether the literal values in `evidence` change
    /// between deployments, need a name, or special-case one identity.
    Values {
        unit: usize,
        evidence: Map<String, Value>,
    },
    /// The presence questions of each enabled security rule, one unit per
    /// rule, with the framework facts its code is judged with.
    Presence {
        units: Vec<usize>,
        evidence: Map<String, Value>,
        django: bool,
    },
}

impl Ask {
    fn units(&self) -> &[usize] {
        match self {
            Self::Split { unit, .. } | Self::Values { unit, .. } => std::slice::from_ref(unit),
            Self::Presence { units, .. } => units,
        }
    }

    fn evidence(&self) -> Option<&Map<String, Value>> {
        match self {
            Self::Split { .. } => None,
            Self::Values { evidence, .. } | Self::Presence { evidence, .. } => Some(evidence),
        }
    }

    /// Questions about how code reads get no framework role: sent beside
    /// split questions, a role moved their answers without informing them.
    fn reads_role(&self) -> bool {
        !matches!(self, Self::Split { .. })
    }

    /// Add its questions about `functions[index]` of a pack.
    fn ask(
        &self,
        questions: &mut Questions,
        (index, state): (usize, &Value),
        file: &FileContext<'_>,
        out: &FilePlan,
    ) {
        match self {
            Self::Split { unit, nested } => {
                functions::ask(questions, index, &out.units[*unit].id, *nested, Pass::First);
            }
            Self::Values { unit, .. } => hardcoded::ask(questions, index, &out.units[*unit].id),
            Self::Presence { units, django, .. } => {
                let units: Vec<(&'static str, &str)> = units
                    .iter()
                    .map(|unit| (out.units[*unit].rule, out.units[*unit].id.as_str()))
                    .collect();
                let key = format!("f{index}");
                let code = format!("functions[{index}].source");
                security::ask(questions, file, (&key, &code, state), &units, *django);
            }
        }
    }
}

/// A function in a pack: its name, source and the evidence its asks add,
/// sent once, and what each rule asks about it.
struct Entry {
    name: String,
    source: String,
    asks: Vec<Ask>,
    state: Value,
}

impl Entry {
    fn new(name: String, source: String, asks: Vec<Ask>) -> Self {
        let mut state = Map::new();
        state.insert("name".into(), json!(name));
        state.insert("source".into(), json!(source));
        for evidence in asks.iter().filter_map(Ask::evidence) {
            state.extend(evidence.clone());
        }
        Self {
            name,
            source,
            asks,
            state: Value::Object(state),
        }
    }
}

/// Plan the requests of a file's function asks: each function once per
/// pack with every rule's questions about it. In a file whose framework
/// role is set, split questions are packed apart with the plain file state.
/// With a change judged, only the functions it touched are packed, within
/// the runs of the whole file.
pub(super) fn send(
    file: &FileContext<'_>,
    asks: Vec<FunctionAsk>,
    out: &mut FilePlan,
    requests: &mut Vec<Planned>,
) {
    let (plain, with_role): (Vec<_>, Vec<_>) = asks
        .into_iter()
        .partition(|a| file.framework.is_some() && !a.ask.reads_role());
    for (lane, asks) in [(file.plain_state(), plain), (file.file_state(), with_role)] {
        // Runs end after names, so a function added, removed or resized
        // re-asks only its own run.
        let packs = pack_runs(
            entries(asks),
            |e| e.name.as_str(),
            |e| &e.state,
            |e| {
                e.asks
                    .iter()
                    .flat_map(Ask::units)
                    .any(|&unit| file.judges_unit(&out.units[unit]))
            },
        );
        for pack in packs {
            send_pack(file, &lane, pack, out, requests);
        }
    }
}

/// The functions asked about, in their file's parsed order, each with every
/// rule's asks.
fn entries(asks: Vec<FunctionAsk>) -> Vec<Entry> {
    let mut functions = BTreeMap::<usize, (String, String, Vec<Ask>)>::new();
    for FunctionAsk {
        position,
        name,
        source,
        ask,
    } in asks
    {
        functions
            .entry(position)
            .or_insert_with(|| (name, source, Vec::new()))
            .2
            .push(ask);
    }
    functions
        .into_values()
        .map(|(name, source, asks)| Entry::new(name, source, asks))
        .collect()
}

/// Plan one pack's request, or when it does not fit, its functions one at a
/// time, and a function whose questions do not fit together, one rule at a
/// time: the request that rule sends for it alone. A rule whose request
/// still does not fit leaves its units needing context.
fn send_pack(
    file: &FileContext<'_>,
    lane: &Value,
    mut pack: Vec<Entry>,
    out: &mut FilePlan,
    requests: &mut Vec<Planned>,
) {
    let (request, asked) = request(file, lane, &pack, out);
    if file.budget.fits(&request) {
        requests.push(Planned {
            owner: file.owner,
            request,
            asked,
        });
        return;
    }
    if pack.len() > 1 {
        for entry in pack {
            send_pack(file, lane, vec![entry], out, requests);
        }
        return;
    }
    let Some(Entry {
        name, source, asks, ..
    }) = pack.pop()
    else {
        return;
    };
    if let [ask] = &asks[..] {
        for &unit in ask.units() {
            out.units[unit].unsent();
        }
        return;
    }
    for ask in asks {
        let alone = Entry::new(name.clone(), source.clone(), vec![ask]);
        send_pack(file, lane, vec![alone], out, requests);
    }
}

/// One pack's request: the file's `lane` state and each function's state,
/// with every ask's questions about it.
fn request(file: &FileContext<'_>, lane: &Value, pack: &[Entry], out: &FilePlan) -> (Value, Asked) {
    let mut questions = Questions::default();
    for (index, entry) in pack.iter().enumerate() {
        for ask in &entry.asks {
            ask.ask(&mut questions, (index, &entry.state), file, out);
        }
    }
    let state = json!({
        "file": lane,
        "functions": pack.iter().map(|e| e.state.clone()).collect::<Vec<_>>(),
    });
    file.request(STAGE, state, questions)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        catalog::{FUNCTION_SIMPLIFICATION, HARDCODED_VALUES},
        token_budget::{Limits, TokenBudget},
        units::{Detail, FollowUp, Presence, UnitPlan},
    };

    /// A follow-up that is never sent: only whether a unit keeps it matters.
    fn follow_up() -> FollowUp {
        (json!({}), Asked::default()).into()
    }

    /// A judged unit with a recheck.
    fn unit(rule: &'static str, id: &str, detail: Detail) -> UnitPlan {
        UnitPlan {
            rule,
            id: id.into(),
            name: id.into(),
            presence: Presence::Judged,
            locations: Vec::new(),
            quote: None,
            lines: 3,
            identity: String::new(),
            detail,
            recheck: Some(follow_up()),
        }
    }

    /// The function simplification and hardcoded-value units of a function
    /// `name` at `position`, with what each asks. Neither `find` nor `scan`
    /// ends a run, so they start in one pack.
    fn judged(name: &str, position: usize, out: &mut FilePlan) -> [FunctionAsk; 2] {
        let split = Detail::Function {
            blocks: Vec::new(),
            locate: Some(follow_up()),
        };
        out.units.push(unit(FUNCTION_SIMPLIFICATION, name, split));
        let values = Detail::Values {
            values: vec!["40".into()],
            choices: vec!["40".into()],
            repeated: vec![false],
            locate: Some(follow_up()),
        };
        out.units.push(unit(HARDCODED_VALUES, name, values));
        let ask = |ask| FunctionAsk {
            position,
            name: name.into(),
            source: format!("fn {name}() -> u32 {{\n    40\n}}\n"),
            ask,
        };
        [
            ask(Ask::Split {
                unit: out.units.len() - 2,
                nested: false,
            }),
            ask(Ask::Values {
                unit: out.units.len() - 1,
                evidence: Map::from_iter([("values".to_string(), json!(["40"]))]),
            }),
        ]
    }

    /// The question names of each request planned for `asks` when only the
    /// requests `fits` accepts fit, as if the answer cache answered them
    /// whole: at a thousand tokens a byte, none fits by its size.
    fn planned(
        asks: Vec<FunctionAsk>,
        out: &mut FilePlan,
        fits: &dyn Fn(&Value) -> bool,
    ) -> Vec<Vec<String>> {
        let budget = TokenBudget {
            bytes_per_token: 1e-3,
        };
        let unanswered = |request: &Value| (!fits(request)).then(|| request.clone());
        let file = FileContext {
            owner: 0,
            path: std::path::Path::new("lib.rs"),
            language: "Rust",
            source: "",
            source_hash: "",
            model: "jev-1.13.0",
            budget: Limits::new(&budget, &unanswered),
            framework: None,
            project: None,
            changed: None,
        };
        let mut requests = Vec::new();
        send(&file, asks, out, &mut requests);
        requests
            .iter()
            .map(|p| {
                p.request["questions"]
                    .as_object()
                    .unwrap()
                    .keys()
                    .cloned()
                    .collect()
            })
            .collect()
    }

    #[test]
    fn a_pack_that_does_not_fit_is_sent_a_function_at_a_time_with_every_rule() {
        let mut out = FilePlan::default();
        let mut asks = Vec::from(judged("find", 0, &mut out));
        asks.extend(judged("scan", 1, &mut out));
        let one_function = |request: &Value| {
            request["state"]["functions"]
                .as_array()
                .is_some_and(|f| f.len() == 1)
        };
        let asked = planned(asks, &mut out, &one_function);
        let every_rule = ["f0_environment", "f0_magic", "f0_special", "f0_split"];
        assert_eq!(asked, [every_rule, every_rule]);
        assert!(out.units.iter().all(|u| u.presence == Presence::Judged));
    }

    #[test]
    fn a_function_whose_questions_do_not_fit_together_is_asked_rule_by_rule() {
        let mut out = FilePlan::default();
        let asks = Vec::from(judged("find", 0, &mut out));
        let split_alone = |request: &Value| {
            request["questions"]
                .as_object()
                .is_some_and(|q| q.keys().all(|key| key.ends_with("_split")))
        };
        assert_eq!(planned(asks, &mut out, &split_alone), [["f0_split"]]);
        let [split, values] = &out.units[..] else {
            panic!("two units");
        };
        assert_eq!(split.presence, Presence::Judged);
        assert!(split.recheck.is_some(), "the split is still rechecked");
        // Its values could not be sent even alone: they need context, and
        // neither their recheck nor their locate is asked.
        assert_eq!(values.presence, Presence::NeedsContext);
        assert!(values.recheck.is_none());
        assert!(matches!(values.detail, Detail::Values { locate: None, .. }));
    }
}
