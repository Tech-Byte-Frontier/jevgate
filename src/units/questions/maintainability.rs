//! Questions of function simplification whose reviews the default gate
//! measures: splitting and flattening a function, and locating a split. The
//! maintainability rules' look-here questions are in `look`.
use super::{EVIDENCE, choose_id, score};
use serde_json::{Value, json};

/// Asks whether splitting would help a reader, not how many tasks the function
/// performs: counting tasks read multi-step functions literally, and most
/// functions it called multi-task were fine as written.
pub fn function_split(path: &str, callees: bool) -> Value {
    let note = if callees {
        format!("`callees` lists the signatures of functions it calls. {EVIDENCE}")
    } else {
        EVIDENCE.to_string()
    };
    json!({
        "type": "score",
        "instructions": {
            "question": format!("Would splitting the function in `{path}` into smaller named functions make it easier to understand?"),
            "note": note,
        },
        "criteria": [
            "No. It reads as one job: its steps are short, already call named functions, or belong together, such as checks before a write, one transaction, or one component and its markup.",
            "Slightly. One block could be named as a helper, but the function is readable as it is.",
            "Yes. It mixes separate jobs in long blocks, so a reader must keep unrelated details in mind at once.",
        ],
    })
}

/// Asked only when the parser finds deep nesting or a long branch chain.
pub fn function_flatten(path: &str) -> Value {
    score(
        format!(
            "Would guard clauses, early returns or a lookup table make the branching in `{path}` easier to follow?"
        ),
        "",
        [
            "No. The branching reads clearly as written.",
            "Slightly. One condition could return early, but the flow is easy to follow.",
            "Yes. Nested or repeated branches hide the main path, and flattening them would make it clear.",
        ],
    )
}

/// Asked only after the split question raised a finding, to locate it.
pub fn function_block(blocks: &[String]) -> Value {
    choose_id(
        "Which block in `function.blocks` would be most useful as its own named function?",
        format!(
            "`function.blocks` holds the body in order. Options are the `id` values in `function.blocks`. {EVIDENCE}"
        ),
        blocks,
        "No single block would be clearer as its own function.",
    )
}
