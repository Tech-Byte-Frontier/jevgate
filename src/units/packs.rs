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
