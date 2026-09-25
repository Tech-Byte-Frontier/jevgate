//! The message and recommended action of each kind of finding, one module
//! per rule family; helpers they share are here.
use super::{
    Block, Detail, GroupInfo,
    outcome::{
        Answers, Outcome, benefit, comment_concern_kind, comment_signals, disagreement,
        document_split, noul, origin_outcome, repeated, section_signals, settled_checks,
        value_signals,
    },
};
use crate::catalog;
use crate::schema::{Answer, Strength};

mod documentation;
mod maintainability;
mod security;
mod test_rules;

pub(super) use documentation::{
    comment_reason, comment_wording, doc_pair_wording, document_wording, plan_wording,
    section_wording, stale_wording,
};
pub(super) use maintainability::{function_wording, outline_wording, pair_wording, values_wording};
pub(super) use security::{handler_wording, module_wording, privilege_wording, security_wording};
pub(super) use test_rules::{test_pair_wording, test_wording};

/// A finding's message and the action it recommends.
pub(super) type Wording = (String, &'static str);

/// The probability a message shows: the one that set its review or consider.
/// A note shows none, since its concern stayed below the level that acts on
/// it; the report keeps the raw value.
pub(super) fn shown(strength: Strength, p: f64) -> String {
    match strength {
        Strength::Note => String::new(),
        _ => format!(" ({p:.2})"),
    }
}

/// A question in plain words, for listing what a unit left undecided.
pub(super) fn question_label(question: &str) -> &str {
    match question {
        "split" => "splitting",
        "flatten" => "flattening",
        "same" => "same steps",
        "environment" => "environment value",
        "magic" => "unnamed value",
        "special" => "special case",
        "restates" => "repeats its code",
        "verbose" => "sentences that add nothing",
        "disabled" => "code turned off",
        "interpreted" => "variable in interpreted text",
        "resource" => "variable in a path or URL",
        "origin" => "origin of values",
        "handled" => "values bound or checked",
        "logs_secret" => "secret in logs",
        "handler_leaks" => "error handler sends details",
        "data" | "exposed" => "public table of users' data",
        "rows" | "returns_others" => "other users' rows",
        "reach" => "whose state it changes",
        "argument_rows" => "rows its arguments choose",
        "operator_only" => "admin-only change",
        "error_details" => "error details to clients",
        "weakened" => "weak setting",
        "own_logic" => "recomputed expected value",
        "mock_only" => "checks only its mocks",
        "overlap" => "overlapping tests",
        "inferable" => "restates the repository",
        "describes" => "description only",
        "commands" => "commands the manifests show",
        "generic" => "generic advice",
        "history" => "past work",
        "plan" => "finished plan",
        "relies" => "relies on a missing path",
        "a_covers" | "b_covers" => "repeats the other section",
        "conflict" => "disagrees with the other section",
        "enforced" => "rule linters check",
        "others" => "other users' rows",
        "editable" => "value users can change",
        "search_path" => "open search_path",
        "unchecked" => "caller not checked",
        "broad" => "broad grant",
        "outside" => "outside text in a run script",
        "untrusted" => "pull request code with secrets",
        other => other,
    }
}

/// A phrase whose first word is a present-tense verb, after "may":
/// "sends details" becomes "send details", "hashes passwords" "hash passwords".
fn base_form(phrase: &str) -> String {
    let (verb, rest) = phrase.split_once(' ').unwrap_or((phrase, ""));
    let base = ["shes", "ches", "sses", "xes"]
        .iter()
        .find(|ending| verb.ends_with(*ending))
        .map_or_else(
            || verb.strip_suffix('s').unwrap_or(verb),
            |_| &verb[..verb.len() - 2],
        );
    if rest.is_empty() {
        base.to_string()
    } else {
        format!("{base} {rest}")
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn verbs_after_may_take_their_base_form() {
        for (phrase, expected) in [
            (
                "sends internal error details",
                "send internal error details",
            ),
            ("hashes passwords", "hash passwords"),
            (
                "chooses a weak security setting",
                "choose a weak security setting",
            ),
            ("fixes a value", "fix a value"),
            ("uses a value", "use a value"),
            (
                "special-cases one specific identity",
                "special-case one specific identity",
            ),
        ] {
            assert_eq!(super::base_form(phrase), expected);
        }
    }
}
