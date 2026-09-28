//! Asking custom questions in the built-in first-pass requests that already
//! send their units' evidence, so a unit's source goes up once. A request
//! takes a custom unit when one entry of its list holds every field of the
//! unit's item: the function stage's `{name, source}`, whatever else later
//! versions add to it. Only the questions are added; the built-in ones and
//! the state stay as they are.
use super::{Ask, body, items::Item, key, list};
use crate::{
    custom::Kind,
    schema::Pass,
    units::{FileContext, FilePlan, Planned, answers::AskedQuestion, evidence},
};
use serde_json::Value;

/// The built-in first-pass stage that sends units of `kind`.
fn stage(kind: Kind) -> Option<&'static str> {
    match kind {
        Kind::Function => Some("functions"),
        Kind::Test => Some("tests"),
        Kind::Comment => Some("comments"),
        Kind::Section => Some("instructions"),
        Kind::File | Kind::Hunk => None,
    }
}

/// Ask each of `asks` in the first of `requests`, the file's, whose state
/// lists its item; return the asks no request takes.
pub(super) fn ride(
    file: &FileContext<'_>,
    kind: Kind,
    items: &[Item],
    asks: Vec<Ask>,
    out: &FilePlan,
    requests: &mut [Planned],
) -> Vec<Ask> {
    let Some(stage) = stage(kind) else {
        return asks;
    };
    let mut pending = asks;
    for planned in requests.iter_mut() {
        if pending.is_empty() {
            break;
        }
        let first = !planned.asked.questions.is_empty()
            && planned
                .asked
                .questions
                .iter()
                .all(|q| q.pass == Pass::First);
        if first && crate::requests::stage(&planned.request) == stage {
            pending = join(file, (kind, items), pending, out, planned);
        }
    }
    pending
}

/// Add to `planned` the asks whose item its state lists, and return the
/// others. When the questions would take it past the provider limit, it
/// stays as it was and they are all returned: a built-in request is never
/// split for a custom question.
fn join(
    file: &FileContext<'_>,
    (kind, items): (Kind, &[Item]),
    pending: Vec<Ask>,
    out: &FilePlan,
    planned: &mut Planned,
) -> Vec<Ask> {
    let listed = planned.request["state"][list(kind)]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let facts: Vec<&str> = evidence::facts(&planned.request["state"]).collect();
    let mut request = planned.request.clone();
    let mut asked = planned.asked.clone();
    let (mut joined, mut rest) = (Vec::new(), Vec::new());
    for ask in pending {
        let item = &items[ask.item].state;
        let Some(index) = listed.iter().position(|entry| holds(entry, item)) else {
            rest.push(ask);
            continue;
        };
        let key = key(index, ask.question);
        let mut question = body(ask.question, kind, index);
        for fact in &facts {
            evidence::point_to(&mut question, fact);
        }
        request["questions"][key.as_str()] = question;
        let unit = &out.units[ask.unit].id;
        asked
            .questions
            .push(AskedQuestion::custom(key, unit, ask.question));
        joined.push(ask);
    }
    if !joined.is_empty() && file.budget.fits(&request) {
        planned.request = request;
        planned.asked = asked;
    } else {
        rest.extend(joined);
    }
    rest
}

/// Whether a built-in request's entry holds every field of an item.
fn holds(entry: &Value, item: &Value) -> bool {
    item.as_object().is_some_and(|fields| {
        fields
            .iter()
            .all(|(name, value)| entry.get(name) == Some(value))
    })
}
