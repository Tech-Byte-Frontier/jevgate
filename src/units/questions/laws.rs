//! Questions of the laws rule: whether the comment above a Bend 2 law
//! claims more than the law states, and what it adds when the first answer
//! stays undecided.
use super::EVIDENCE;
use serde_json::{Value, json};

/// Whether a Bend 2 law's comment claims more than the law states. The law
/// is what the compiler checks and its comment what a person reads: a
/// behavior the comment promises and the law leaves out can break while
/// every proof still passes. The law is compared in words: asked of its
/// notation alone, answers did not tell a comment that restates its law
/// from one that claims more, either as "promises more than" or as whether
/// the definitions could change so the comment turns false.
pub fn law_states(index: usize) -> Value {
    let law = format!("laws[{index}]");
    super::score(
        format!(
            "Does the comment in `{law}.comment` claim anything about behavior that the law, read in `{law}.reading`, does not state?"
        ),
        &format!(
            "`{law}.source` is the law as written and `{law}.defs` the definitions it names; `file.comment`, when present, says what the file's laws pin. The compiler checks the law, not its comment."
        ),
        [
            "No. The comment says what the law states in other words: informally, with an example, or with its intuition.",
            "Barely. One loose word reads wider than the law, such as `any` where the law's clauses bound the input, and a reader would take the law's meaning.",
            "Yes. The comment claims a property the law does not state (such as `sound and complete` above a law that states only soundness), for more inputs than the law covers, or a consequence that needs a condition the law does not state.",
        ],
    )
}

/// The options of the law recheck that name something the comment claims
/// and the law does not state.
pub const LAW_GAPS: [&str; 3] = ["property", "inputs", "condition"];

/// What a Bend 2 law's comment says beyond its law, asked of a law whose
/// first answer stayed undecided: its options name what separates a
/// comment that restates its law from one that promises more.
pub fn law_relation() -> Value {
    json!({
        "type": "choice",
        "instructions": {
            "question": "What does the comment in `law.comment` say about behavior beyond what the law, read in `law.reading`, states?",
            "note": format!("`law.source` is the law as written and `law.defs` the definitions it names; `file.comment`, when present, says what the file's laws pin. {EVIDENCE}"),
        },
        "criteria": {
            "nothing": "Nothing: it states the law in other words, informally, with an example or with its intuition.",
            "context": "Only why the law matters, where it is used, how it is proven, or that it checks a sample of cases.",
            "property": "A property the law does not state, such as `complete` beside `sound`, or a result the law leaves free.",
            "inputs": "The property for more inputs or states than the law's clauses cover.",
            "condition": "A consequence that follows from the law only under a condition the law does not state.",
        },
    })
}

/// Whether a Bend 2 law checks particular inputs where its comment speaks
/// of inputs in general, asked beside `law_relation`. On thirteen Bend 2
/// projects, 11 of the 29 laws still undecided after `law_relation` claimed
/// in their comment what the law checks for one fixed key, an empty or
/// one-entry object or one byte, and its options did not tell them from
/// laws that restate their comment.
pub fn law_fixed() -> Value {
    json!({
        "type": "noul",
        "instructions": {
            "question": "Does the law in `law.source` check only particular inputs, such as one given key, an empty or one-entry structure or one number, where the comment in `law.comment` says the behavior holds for such inputs in general?",
            "note": format!("`law.reading` reads the law in words. {EVIDENCE}"),
        },
        "criteria": {
            "true": {
                "what": "The law passes a literal or a fixed structure where the comment speaks of any key, object, list, byte or channel, so a definition that fails on other inputs passes the law.",
                "examples": [
                    "A law that looks up a map holding one entry under a comment saying lookups return the value of any key present",
                    "A law that removes the only entry of a map under a comment saying removal deletes a key"
                ]
            },
            "false": {
                "what": "The law quantifies over every input the comment speaks of, or the comment itself names the particular case.",
                "examples": [
                    "A comment about the end-of-file marker above a law that checks that marker",
                    "A comment marking the law as an example or a sanity check"
                ]
            }
        },
    })
}
