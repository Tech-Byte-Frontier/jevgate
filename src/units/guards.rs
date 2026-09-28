//! Requests for the guards (`crate::guards`): a test a change rewrote is
//! asked, with both versions, whether it now checks less.
use super::{Questions, evidence::request, questions};
use crate::{
    guards::ChangedTest,
    schema::{Answer, Pass},
};
use serde_json::{Value, json};

/// The stage and rule the guards' questions are recorded under.
pub(crate) const GUARDS: &str = "guards";

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
