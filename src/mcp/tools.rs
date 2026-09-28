//! The tools the server offers: what each does, the arguments it takes and
//! the shape of its structured result. The schemas use only `type`,
//! `properties`, `required`, `items`, `enum` and number bounds, which every
//! client's validator reads the same way.
use super::results::{DEFAULT_FINDINGS, DEFAULT_VERIFY, MAX_LISTED};
use serde_json::{Value, json};

pub(super) fn list() -> Value {
    json!([
        {
            "name": "jevgate_check",
            "title": "Review code with JevGate",
            "description": "Run `jevgate check` in the repository and return its findings, each with an id, a location, how often findings of its rule and level were right, and a next step, then the units Jev left undecided as verify items and what the change does to the checks around the code as guards. Uses jevgate.toml and the API key `jevgate auth status` shows; unchanged code is answered from the cache for free, and dry_run costs nothing. Can take minutes on a large change; with a progress token, it reports each stage.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "base": {"type": "string", "description": "Review only what changed since this Git revision, such as origin/main: the functions, tests and comments on changed lines, and copies where either copy changed"},
                    "whole_files": {"type": "boolean", "description": "With base, judge each changed file whole instead of only what the change touched"},
                    "paths": {"type": "array", "items": {"type": "string"}, "description": "Files or directories to review instead of the discovered source"},
                    "rules": {"type": "array", "items": {"type": "string"}, "description": "Rule IDs, names, keys or groups, such as security or file-organization; replaces the configured selection"},
                    "include_tests": {"type": "boolean", "description": "Also judge tests"},
                    "dry_run": {"type": "boolean", "description": "List the files and planned requests without sending anything"},
                    "verbose": {"type": "boolean", "description": "Also return optional notes, and per-file detail in the text"},
                    "max_findings": max_findings(),
                    "max_verify": max_verify(),
                },
                "additionalProperties": false,
            },
            "outputSchema": result_schema(),
        },
        {
            "name": "jevgate_findings",
            "title": "Read the last JevGate report",
            "description": "Return the findings and verify items of the last check in this repository (.jevgate/latest.json), ranked, without running anything.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "Only findings and verify items in files under this path"},
                    "include_notes": {"type": "boolean", "description": "Also return optional notes"},
                    "max_findings": max_findings(),
                    "max_verify": max_verify(),
                },
                "additionalProperties": false,
            },
            "outputSchema": result_schema(),
            "annotations": {"readOnlyHint": true},
        },
        {
            "name": "jevgate_rules",
            "title": "List JevGate's rules",
            "description": "Every rule with its ID, group, default, the question it asks and what it looks at, the repository's custom questions included.",
            "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false},
            "outputSchema": rules_schema(),
            "annotations": {"readOnlyHint": true},
        },
    ])
}

fn max_findings() -> Value {
    json!({
        "type": "integer", "minimum": 1, "maximum": MAX_LISTED,
        "description": format!("Return at most this many findings, those that fail the gate first, then new ones and reviews [default: {DEFAULT_FINDINGS}]; total_findings counts them all"),
    })
}

fn max_verify() -> Value {
    json!({
        "type": "integer", "minimum": 0, "maximum": MAX_LISTED,
        "description": format!("Return at most this many verify items, those leaning most toward the concern first [default: {DEFAULT_VERIFY}]; total_verify counts them all"),
    })
}

/// The structured result of `jevgate_check` and `jevgate_findings`.
fn result_schema() -> Value {
    let strings = json!({"type": "array", "items": {"type": "string"}});
    let count = json!({"type": "integer", "minimum": 0});
    json!({
        "type": "object",
        "properties": {
            "headline": {"type": "string", "description": "Status, gate, files, requests and cost on one line"},
            "status": {"type": "string", "description": "The most severe outcome: review, consider, needs-context, uncertain, note, clear, not-applicable, no-changed-source, or incomplete when the run could not finish"},
            "complete": {"type": "boolean", "description": "Whether every selected file was judged; an incomplete run is never a pass"},
            "dry_run": {"type": "boolean"},
            "exit_code": {"type": "integer", "description": "0: the gate passed (or a dry run); 1: the gate failed; 2: the run could not finish"},
            "gate": {
                "type": "object",
                "properties": {
                    "passed": {"type": "boolean"},
                    "reasons": strings,
                    "new_findings": count,
                    "baselined_findings": count,
                    "suppressed_findings": count,
                },
                "required": ["passed", "reasons", "new_findings", "baselined_findings"],
            },
            "errors": {"type": "array", "items": {"type": "string"}, "description": "The run's errors, then `Failed N: reason` for files that failed"},
            "skipped": {"type": "array", "items": {"type": "string"}, "description": "`Skipped N: reason` for files that were not judged, such as a syntax error"},
            "left_out": {"type": "array", "items": {"type": "string"}, "description": "`path:line unit: reason` for code the parser could not read in files judged otherwise, at most 20; that code was not reviewed"},
            "total_left_out": {"type": "integer", "description": "Units left out in all, when any was"},
            "usage": {
                "type": "object",
                "properties": {
                    "api_requests": count,
                    "input_tokens": count,
                    "usd": {"type": "number", "description": "Estimated cost of the paid input tokens"},
                },
                "required": ["api_requests", "input_tokens"],
            },
            "planned": {
                "type": "object",
                "description": "A dry run's first-pass requests and their questions, those the cache answers, and the new input tokens and dollars of what the rest send: a request sends only the questions the cache lacks",
                "properties": {"requests": count, "cached": count, "questions": count, "cached_questions": count, "tokens": count, "usd": {"type": "number"}},
                "required": ["requests", "cached", "questions", "cached_questions", "tokens"],
            },
            "findings": {"type": "array", "items": finding_schema()},
            "total_findings": count,
            "verify": {"type": "array", "items": verify_schema()},
            "total_verify": count,
            "guards": {"type": "array", "items": guard_schema()},
            "total_guards": count,
        },
        "required": ["headline", "status", "complete", "dry_run", "exit_code", "errors", "usage", "findings", "total_findings", "verify", "total_verify", "guards", "total_guards"],
    })
}

/// A guard, as the JSON report records it.
fn guard_schema() -> Value {
    json!({
        "type": "object",
        "description": "What the change does to the checks around the code, for a person to look at: never a finding, never failing the gate",
        "properties": {
            "kind": {"type": "string", "enum": ["allow", "suppression", "skipped-test", "focused-test", "deleted-test", "weaker-assertion", "configuration", "question", "baseline", "skipped-file", "steering", "cache"]},
            "path": {"type": "string"},
            "line": {"type": "integer"},
            "text": {"type": "string", "description": "What was found: the line, a test's name, the settings changed"},
            "message": {"type": "string", "description": "What it does, such as `skips a test`"},
            "probability": {"type": "number", "description": "Jev's answer, for a weaker test or text written to steer a reviewer"},
            "id": {"type": "string", "description": "A hash of the kind, path and text, which stays when lines move"},
        },
        "required": ["kind", "path", "text", "message", "id"],
    })
}

fn finding_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "id": {"type": "string", "description": "The finding's fingerprint (rule, path and unit identity), as the baseline and SARIF record it; stable across unrelated edits"},
            "path": {"type": "string"},
            "line": {"type": "integer"},
            "end_line": {"type": "integer"},
            "rule": {"type": "string"},
            "strength": {"type": "string", "enum": ["review", "consider", "note"], "description": "review: act on it when it is right; consider: fix it or say why the code should stay; note: optional. `gate` says whether it fails the gate"},
            "message": {"type": "string", "description": "Why it was found, then how often findings of its rule and level were right: `Right 87% of the time (23 labels).`, or `Not yet measured.` below 20 labels; a note's message alone"},
            "action": {"type": "string", "description": "The next step"},
            "probability": {"type": "number", "description": "The probability of the answer that set the level: how sure that answer was, not how often such findings are right"},
            "precision": {
                "type": "object",
                "description": "How often findings of its rule and level were right on projects JevGate was never tuned on, as `jevgate rules` counts them, in a preview language that language's own; absent for notes",
                "properties": {"right": {"type": "integer", "minimum": 0}, "labeled": {"type": "integer", "minimum": 0}},
                "required": ["right", "labeled"],
            },
            "preview": {"type": "string", "description": "The preview language of its file, for a finding of JevGate's own rules there: `precision` is that language's own, and the default gate never fails on it"},
            "symbol": {"type": "string"},
            "category": {"type": "string", "description": "The weakness a security finding names, such as CWE-89 SQL injection"},
            "gate": {"type": "string", "enum": ["fails", "measuring", "advisory"], "description": "How the gate counted a new finding: fails (it fails the gate), measuring (reported without failing: its rule and level are still being measured, or its file's language is in preview) or advisory (the level in force does not count it); absent for notes and accepted findings"},
            "baselined": {"type": "boolean", "description": "Accepted by the baseline; it never fails the gate"},
            "suppressed": {"type": "string", "description": "The reason an inline `jevgate: allow` comment gives; it never fails the gate"},
        },
        "required": ["id", "path", "line", "end_line", "rule", "strength", "message", "action", "probability"],
    })
}

fn verify_schema() -> Value {
    json!({
        "type": "object",
        "description": "A unit whose answers stayed undecided: never a finding, and failing the gate only where jevgate.toml puts `uncertain` among its rule's levels",
        "properties": {
            "id": {"type": "string", "description": "The unit's fingerprint (rule, path and unit identity), made as a finding's is; stable across unrelated edits"},
            "path": {"type": "string"},
            "line": {"type": "integer"},
            "end_line": {"type": "integer"},
            "rule": {"type": "string"},
            "unit": {"type": "string", "description": "The function, file outline, test or section judged"},
            "concern": {"type": "number", "description": "The highest probability its open questions give the answer that raises their concern"},
            "questions": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "question": {"type": "string", "description": "The question as it was asked; its backticked paths name the evidence"},
                        "evidence": {"type": "array", "items": {"type": "string"}, "description": "The state paths the question names, such as functions[0].source: the unit's code at its location"},
                        "answers": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "option": {"type": "string"},
                                    "meaning": {"type": "string"},
                                    "probability": {"type": "number"},
                                },
                                "required": ["option", "probability"],
                            },
                        },
                    },
                    "required": ["question"],
                },
            },
        },
        "required": ["path", "line", "end_line", "rule", "unit", "concern", "questions"],
    })
}

/// `jevgate_rules`' result: the objects `jevgate rules --format json` prints.
fn rules_schema() -> Value {
    let text = json!({"type": "string"});
    json!({
        "type": "object",
        "properties": {
            "rules": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "id": text, "key": text, "group": text,
                        "default_enabled": {"type": "boolean"},
                        "version": text, "scope": text, "unit": text,
                        "inspection": text, "acceptable_example": text,
                        "requires_tests": {"type": "boolean"},
                        "evaluation_dataset": text,
                        "thresholds_validated": {"type": "boolean"},
                        "decision_policy": {"type": "object"},
                        "maturity": {"type": "object", "description": "For each level, how many labeled findings were right on projects JevGate was never tuned on (unseen) and on the ones it was tuned on, and whether the level fails the default gate (mature)"},
                        "custom": {"type": "object", "description": "A custom question's definition, as jevgate.toml or its file in .jevgate/questions/ gives it, with the file that defines it (source); absent for built-in rules"},
                    },
                    "required": ["id", "key", "group", "default_enabled"],
                },
            },
        },
        "required": ["rules"],
    })
}

/// Panics unless `value` conforms to `schema` and declares every property
/// it holds. Clients validate a result against its tool's output schema, so
/// a field left out of the schema, or of the wrong type, breaks the call.
#[cfg(test)]
pub(super) fn assert_conforms(value: &Value, schema: &Value) {
    let problems = problems(value, schema, "$");
    assert!(problems.is_empty(), "{problems:#?}\n{value:#}");
}

#[cfg(test)]
fn problems(value: &Value, schema: &Value, at: &str) -> Vec<String> {
    let typed = match schema["type"].as_str() {
        Some("object") => value.is_object(),
        Some("array") => value.is_array(),
        Some("string") => value.is_string(),
        Some("boolean") => value.is_boolean(),
        Some("integer") => value.is_u64() || value.is_i64(),
        Some("number") => value.is_number(),
        _ => true,
    };
    if !typed {
        return vec![format!("{at}: {value} is not {}", schema["type"])];
    }
    let mut found = Vec::new();
    if let Some(allowed) = schema["enum"].as_array()
        && !allowed.contains(value)
    {
        found.push(format!("{at}: {value} is not one of {allowed:?}"));
    }
    if let Some(items) = value.as_array() {
        for (i, item) in items.iter().enumerate() {
            found.extend(problems(item, &schema["items"], &format!("{at}[{i}]")));
        }
    }
    if let (Some(object), Some(properties)) = (value.as_object(), schema["properties"].as_object())
    {
        for required in schema["required"].as_array().into_iter().flatten() {
            if !object.contains_key(required.as_str().unwrap_or_default()) {
                found.push(format!("{at}: {required} is missing"));
            }
        }
        for (key, item) in object {
            match properties.get(key) {
                Some(property) => found.extend(problems(item, property, &format!("{at}.{key}"))),
                None => found.push(format!("{at}.{key} is not declared")),
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_validator_rejects_missing_undeclared_and_mistyped_fields() {
        let schema = json!({"type": "object", "properties": {"a": {"type": "integer"}, "b": {"type": "string", "enum": ["x"]}}, "required": ["a"]});
        assert!(problems(&json!({"a": 1, "b": "x"}), &schema, "$").is_empty());
        assert_eq!(
            problems(&json!({"b": "y", "c": 1.5}), &schema, "$").len(),
            3
        );
        assert_eq!(problems(&json!({"a": 1.5}), &schema, "$").len(), 1);
    }
}
