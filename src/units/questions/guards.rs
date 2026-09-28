//! The guards' question, asked only of evidence code selected: whether a
//! test a change rewrote checks less than before.
use super::EVIDENCE;
use serde_json::{Value, json};

/// Whether the rewritten test in `test.after` checks less than it did in
/// `test.before`. A rewrite that moves assertions into a helper checks as
/// much, so the helpers it newly calls are sent with it.
pub fn weaker() -> Value {
    json!({
        "type": "noul",
        "instructions": {
            "question": "Does the test in `test.after` check less of the code's behavior than the same test in `test.before` does?",
            "note": format!("`helpers`, when present, holds the functions of the same file that `test.after` calls and `test.before` does not. {EVIDENCE}"),
        },
        "criteria": {
            "true": "Yes. `test.after` drops an assertion, compares more loosely (a range, a type or a substring where there was a value), only checks that no error was raised, or expects whatever the code returns now instead of what it should.",
            "false": "No. `test.after` checks the code at least as closely: its assertions are renamed, reordered, restyled or moved into a helper that still makes them, more were added, or an expected value changed along with the behavior it checks.",
        },
    })
}
