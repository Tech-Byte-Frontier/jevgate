//! Bend 2's wording of the function questions: its code is shaped by rules
//! other languages do not have, and its flattening tools are patterns, not
//! early returns. Labeled by hand on thirteen Bend 2 projects, 13 of 22 wrong
//! function-simplification findings split a one-def state machine, a match
//! helper or a proof that follows its definition, or proposed guard clauses
//! and early returns, which Bend does not have.
use serde_json::{Value, json};

const SHAPES: &str = "In Bend 2 a loop with several states is one def with a state argument, since Bend has no mutual recursion; a `match` inspects only parameters and pattern variables, so a computed value is matched in a small helper def; and a proof follows the cases of the definition it proves.";

pub fn reword_bend(language: &str, id: &str, body: &mut Value) {
    if language != crate::analysis::bend::LANGUAGE {
        return;
    }
    let question = body["instructions"]["question"]
        .as_str()
        .unwrap_or_default();
    match id {
        // A file outline's split question shares the id.
        "split" if question.contains("splitting the function") => {
            let note = body["instructions"]["note"].as_str().unwrap_or_default();
            body["instructions"]["note"] = json!(format!("{SHAPES} {note}"));
        }
        "flatten" => {
            let question = question.replace(
                "guard clauses, early returns or a lookup table make the branching",
                "nested patterns, a `case _:` fallback or a helper def make the matches",
            );
            body["instructions"]["question"] = json!(question);
            body["criteria"] = json!([
                "No. The matches follow the shape of the data or a state machine's states, and each arm is short.",
                "Slightly. One nested pattern could merge two matches, but the flow is easy to follow.",
                "Yes. Nested matches repeat the same arms, such as the same failure for each element peeled off a list, or hide the main path, and nested patterns with a `case _:` fallback would show it.",
            ]);
        }
        _ => {}
    }
}
