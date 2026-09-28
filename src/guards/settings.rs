//! Edits to `jevgate.toml`, custom question files and the baseline, compared
//! by what they say: comments, layout and the baseline's time are not an edit.
use super::{Guard, Kind, join};
use crate::{baseline::BASELINE_FILE, init::CONFIG_FILE, output};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

/// Configuration keys named in a message, at most.
const SHOWN_KEYS: usize = 6;

/// What an edit changed in jevgate.toml, a question file or the baseline.
enum Edit {
    /// Only comments, layout or the baseline's time: nothing it says.
    Same,
    /// What changed, as a phrase: "fail_on, rules", "level, threshold",
    /// "accepts 2 more findings".
    Named(String),
    /// The version before does not parse.
    Unreadable,
}

/// An edit to jevgate.toml, a custom question file or the baseline, from the
/// text `before` and `after` it (none when the file did not exist). One that
/// no longer parses, or a question that no longer loads, says so: the agent
/// hook's gate reads the file as the turn began, so the agent cannot switch
/// the gate off by breaking it, and the person hears why the next turn
/// cannot be checked. A question file's edit names the keys that changed, as
/// jevgate.toml's does: its level, threshold, question or guidance.
pub(super) fn settings_guard(
    path: &Path,
    before: Option<&str>,
    after: Option<&str>,
) -> Option<Guard> {
    let baseline = path == Path::new(BASELINE_FILE);
    let kind = if baseline {
        Kind::Baseline
    } else if path == Path::new(CONFIG_FILE) {
        Kind::Configuration
    } else {
        Kind::Question
    };
    let parses = |text: &str| match kind {
        Kind::Baseline => crate::baseline::parses(text),
        Kind::Configuration => toml::from_str::<crate::config::Config>(text).is_ok(),
        _ => crate::custom::loads(path, text),
    };
    let broken = if kind == Kind::Question {
        "load"
    } else {
        "parse"
    };
    let (text, message) = match (before, after) {
        (None, None) => return None,
        (_, Some(after)) if !parses(after) => (
            String::new(),
            format!(
                "is {} and does not {broken}",
                if before.is_some() { "edited" } else { "added" }
            ),
        ),
        (None, Some(_)) => (String::new(), "is added".to_string()),
        (Some(_), None) => (String::new(), "is deleted".to_string()),
        (Some(before), Some(after)) => {
            let edit = if baseline {
                baseline_edit(before, after)
            } else {
                configuration_edit(before, after)
            };
            match edit {
                Edit::Same => return None,
                Edit::Named(changed) => (changed.clone(), format!("is edited: {changed}")),
                Edit::Unreadable => (String::new(), "is edited".to_string()),
            }
        }
    };
    Some(Guard::new(kind, path, None, &text, message))
}

/// The top-level settings whose values differ, as "fail_on, rules".
fn configuration_edit(before: &str, after: &str) -> Edit {
    let (Ok(before), Ok(after)) = (
        toml::from_str::<toml::Table>(before),
        toml::from_str::<toml::Table>(after),
    ) else {
        return Edit::Unreadable;
    };
    let keys: BTreeSet<&String> = before.keys().chain(after.keys()).collect();
    let changed: Vec<&str> = keys
        .into_iter()
        .filter(|key| before.get(*key) != after.get(*key))
        .map(String::as_str)
        .collect();
    if changed.is_empty() {
        return Edit::Same;
    }
    let shown = changed.len().min(SHOWN_KEYS);
    let mut named = changed[..shown].join(", ");
    if changed.len() > shown {
        named.push_str(&format!(" and {} more", changed.len() - shown));
    }
    Edit::Named(named)
}

/// Findings the baseline accepts that it did not, those it no longer
/// accepts, and those whose reason changed.
fn baseline_edit(before: &str, after: &str) -> Edit {
    let entries = |text: &str| -> Option<BTreeMap<String, serde_json::Value>> {
        let value: serde_json::Value = serde_json::from_str(text).ok()?;
        let entry = |f: &serde_json::Value| {
            Some((f["fingerprint"].as_str()?.to_string(), f["reason"].clone()))
        };
        Some(
            value["findings"]
                .as_array()?
                .iter()
                .filter_map(entry)
                .collect(),
        )
    };
    let (Some(before), Some(after)) = (entries(before), entries(after)) else {
        return Edit::Unreadable;
    };
    let accepted = after.keys().filter(|f| !before.contains_key(*f)).count();
    let dropped = before.keys().filter(|f| !after.contains_key(*f)).count();
    let marked = after
        .iter()
        .filter(|(f, reason)| before.get(*f).is_some_and(|was| was != *reason))
        .count();
    let parts: Vec<String> = [
        (accepted, "accepts", "more finding"),
        (dropped, "drops", "finding"),
        (marked, "changes the reason of", "finding"),
    ]
    .into_iter()
    .filter(|(n, ..)| *n > 0)
    .map(|(n, verb, noun)| format!("{verb} {}", output::count(n, noun)))
    .collect();
    if parts.is_empty() {
        Edit::Same
    } else {
        Edit::Named(join(&parts))
    }
}
