//! The bytes a request is sent as: compact JSON, with the look-here
//! questions' keys in the order they were validated in.
use serde_json::Value;
use std::collections::BTreeSet;

/// The order the keys of a question sent in its validated order are
/// written in, where it names them; other keys follow, sorted.
const VALIDATED_ORDER: [&str; 9] = [
    "type",
    "instructions",
    "question",
    "note",
    "criteria",
    "true",
    "false",
    "what",
    "examples",
];

/// The questions of `request` sent with their keys in the order their
/// threshold was chosen on, `what` before `examples`, rather than sorted:
/// the look-here questions, named under `jevgate.validated_order`. Sent
/// sorted, `examples` first, a look-here answer about each of 15 functions
/// fell by up to 0.10, and below 0.70 for four a reviewer flagged.
pub(super) fn validated_order(request: &Value) -> BTreeSet<&str> {
    request["jevgate"]["validated_order"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect()
}

/// Compact JSON of `question`, its keys in [`VALIDATED_ORDER`] when
/// `validated`, else sorted as every other value is written.
pub(super) fn write_question(
    out: &mut Vec<u8>,
    question: &Value,
    validated: bool,
) -> serde_json::Result<()> {
    let Some(fields) = question.as_object().filter(|_| validated) else {
        return serde_json::to_writer(out, question);
    };
    let rank = |key: &str| {
        VALIDATED_ORDER
            .iter()
            .position(|k| *k == key)
            .unwrap_or(VALIDATED_ORDER.len())
    };
    let mut keys: Vec<&String> = fields.keys().collect();
    keys.sort_by_key(|key| rank(key));
    let members = keys.into_iter().map(|key| (key.as_str(), &fields[key]));
    write_object(out, members, |out, _, value| {
        write_question(out, value, validated)
    })
}

/// The body sent for `request`: compact JSON of its provider copy, each
/// question named in `jevgate.validated_order` in its validated order.
pub(crate) fn body(request: &Value) -> serde_json::Result<Vec<u8>> {
    let validated = validated_order(request);
    let provider = super::provider_request(request);
    let Some(fields) = provider.as_object().filter(|_| !validated.is_empty()) else {
        return serde_json::to_vec(provider.as_ref());
    };
    let mut out = Vec::new();
    let members = fields.iter().map(|(key, value)| (key.as_str(), value));
    write_object(&mut out, members, |out, key, value| {
        match value.as_object().filter(|_| key == "questions") {
            Some(questions) => {
                let members = questions.iter().map(|(name, q)| (name.as_str(), q));
                write_object(out, members, |out, name, question| {
                    write_question(out, question, validated.contains(name))
                })
            }
            None => serde_json::to_writer(out, value),
        }
    })?;
    Ok(out)
}

/// Compact JSON of an object whose `members` keep their order, each value
/// written by `write`.
fn write_object<'v>(
    out: &mut Vec<u8>,
    members: impl IntoIterator<Item = (&'v str, &'v Value)>,
    mut write: impl FnMut(&mut Vec<u8>, &str, &Value) -> serde_json::Result<()>,
) -> serde_json::Result<()> {
    out.push(b'{');
    for (i, (key, value)) in members.into_iter().enumerate() {
        if i > 0 {
            out.push(b',');
        }
        serde_json::to_writer(&mut *out, key)?;
        out.push(b':');
        write(out, key, value)?;
    }
    out.push(b'}');
    Ok(())
}
