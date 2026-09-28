//! Validation of provider responses against the request, and the cached form of an answer.
use anyhow::{Context, Result, ensure};
use serde_json::{Map, Value};

/// Providers round each probability to about two decimals: allow half a
/// hundredth per rounded value, at most `MAX_MASS_ERROR` in total, plus float noise.
const ROUNDING_PER_VALUE: f64 = 0.005;
const MAX_MASS_ERROR: f64 = 0.05;
const FLOAT_NOISE: f64 = 1e-9;
/// A usage count above this is corrupt, not a real count, and is ignored.
const MAX_REPORTED_TOKENS: u64 = 1_000_000_000;

/// A response whose answers match the request's questions. Its `usage` is not
/// required: a gateway need not pass TypeSafe's through, and such an answer
/// counts as unmetered rather than as free.
pub fn validate(response: &Value, request: &Value) -> Result<()> {
    validate_model(response, request)?;
    let answers = response["answers"].as_object().context("Missing answers")?;
    let questions = request["questions"]
        .as_object()
        .context("Missing questions")?;
    ensure!(answers.len() == questions.len(), "Missing or extra answers");
    for (name, question) in questions {
        let answer = &response["answers"][name];
        ensure!(
            answer["type"] == question["type"],
            "Wrong answer type for {name}"
        );
        if question["type"] == "noul" {
            probability(&answer["noul"])?;
        } else {
            validate_distribution(answer, question)?;
        }
    }
    Ok(())
}

/// The billed input tokens a response reports; none when it reports no usage.
pub fn input_tokens(response: &Value) -> Option<u64> {
    token_count(response, "input_tokens")
}

/// The output tokens a response reports, zero when it reports none: they are free.
pub fn output_tokens(response: &Value) -> u64 {
    token_count(response, "output_tokens").unwrap_or(0)
}

fn token_count(response: &Value, field: &str) -> Option<u64> {
    response["usage"][field]
        .as_u64()
        .filter(|n| *n <= MAX_REPORTED_TOKENS)
}

/// A well-formed model name; when a pinned version was requested, that
/// version, with or without a gateway's namespace (`typesafe-ai/jev-1.13.0`
/// asked, `jev-1.13.0` answered). An alias answers with whatever version it
/// points to.
fn validate_model(response: &Value, request: &Value) -> Result<()> {
    let model = response["model"]
        .as_str()
        .context("Missing model identity")?;
    ensure!(crate::model::valid_name(model), "Invalid model identity");
    if let Some(requested) = request["model"]
        .as_str()
        .filter(|name| crate::model::pinned(name))
    {
        ensure!(
            crate::model::base_name(model) == crate::model::base_name(requested),
            "Provider returned a different pinned model"
        );
    }
    Ok(())
}

fn probability(value: &Value) -> Result<f64> {
    let p = value.as_f64().context("Non-numeric probability")?;
    ensure!(
        p.is_finite() && (0.0..=1.0).contains(&p),
        "Invalid probability"
    );
    Ok(p)
}

/// A Score or Choice answer: probabilities over exactly the defined options,
/// summing to one, consistent with the reported score or choice.
fn validate_distribution(answer: &Value, question: &Value) -> Result<()> {
    probability(&answer["confidence"])?;
    let probabilities = answer["probabilities"]
        .as_object()
        .context("Missing probabilities")?;
    let keys = option_keys(question)?;
    ensure!(
        keys.len() == probabilities.len() && keys.iter().all(|k| probabilities.contains_key(k)),
        "Wrong probability keys"
    );
    let sum = probabilities
        .values()
        .map(probability)
        .collect::<Result<Vec<_>>>()?
        .iter()
        .sum::<f64>();
    ensure!(
        (sum - 1.0).abs()
            <= (ROUNDING_PER_VALUE * keys.len() as f64).min(MAX_MASS_ERROR) + FLOAT_NOISE,
        "Invalid probability mass"
    );
    if question["type"] == "score" {
        validate_score(answer, probabilities, &keys)
    } else {
        validate_choice(answer, probabilities)
    }
}

/// Score levels by index, or Choice options by name.
fn option_keys(question: &Value) -> Result<Vec<String>> {
    Ok(if question["type"] == "score" {
        let levels = question["criteria"]
            .as_array()
            .context("Invalid rubric")?
            .len();
        (0..levels).map(|i| i.to_string()).collect()
    } else {
        question["criteria"]
            .as_object()
            .context("Invalid choices")?
            .keys()
            .cloned()
            .collect()
    })
}

/// The score is the probability-weighted level, within rounding.
fn validate_score(
    answer: &Value,
    probabilities: &Map<String, Value>,
    keys: &[String],
) -> Result<()> {
    let score = answer["score"].as_f64().context("Missing score")?;
    let expected = keys
        .iter()
        .enumerate()
        .map(|(i, k)| i as f64 * probabilities[k].as_f64().unwrap())
        .sum::<f64>();
    let rounding = ROUNDING_PER_VALUE * (1 + (0..keys.len()).sum::<usize>()) as f64 + FLOAT_NOISE;
    ensure!(
        (0.0..=(keys.len() - 1) as f64).contains(&score) && (score - expected).abs() <= rounding,
        "Inconsistent score"
    );
    Ok(())
}

/// The choice is an option with the highest probability, within rounding.
fn validate_choice(answer: &Value, probabilities: &Map<String, Value>) -> Result<()> {
    let choice = answer["choice"].as_str().context("Missing choice")?;
    ensure!(probabilities.contains_key(choice), "Invalid choice");
    let chosen = probability(&probabilities[choice])?;
    ensure!(
        probabilities
            .values()
            .all(|v| v.as_f64().unwrap() <= chosen + 0.01),
        "Choice is not a highest-probability option"
    );
    Ok(())
}

/// The cached form of a validated response: its model, the typed answers and,
/// when it reported one, its usage.
pub fn cache_value(response: &Value, request: &Value) -> Value {
    let mut answers = serde_json::Map::new();
    for (key, question) in request["questions"].as_object().unwrap() {
        let kind = question["type"].as_str().unwrap();
        answers.insert(key.clone(), typed_fields(&response["answers"][key], kind));
    }
    let mut value = serde_json::json!({"model": response["model"], "answers": answers});
    if let Some(input) = input_tokens(response) {
        value["usage"] =
            serde_json::json!({"input_tokens": input, "output_tokens": output_tokens(response)});
    }
    value
}

/// Only the fields a typed answer defines; anything else the provider sent is dropped.
fn typed_fields(answer: &Value, kind: &str) -> Value {
    let fields: &[&str] = match kind {
        "score" => &["type", "score", "confidence", "probabilities"],
        "choice" => &["type", "choice", "confidence", "probabilities"],
        _ => &["type", "noul"],
    };
    Value::Object(
        fields
            .iter()
            .map(|f| (f.to_string(), answer[*f].clone()))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A one-question request for `requested`, and a valid answer from `answered`.
    fn exchange(requested: &str, answered: &str) -> (Value, Value) {
        let request = json!({"model": requested, "state": "x",
            "questions": {"q": {"type": "noul", "instructions": "?"}}});
        let response = json!({"model": answered, "answers": {"q": {"type": "noul", "noul": 0.1}},
            "usage": {"input_tokens": 10, "output_tokens": 1}});
        (request, response)
    }

    #[test]
    fn an_alias_accepts_any_version_and_a_pinned_name_only_its_own() {
        for (requested, answered) in [
            ("jev-1.13.0", "jev-1.13.0"),
            ("jev-latest", "jev-1.13.0"),
            ("jev-1.13", "jev-1.13.0"),
            ("~typesafe/jev-latest", "typesafe/jev-1.13"),
            ("typesafe/jev-1.13", "typesafe/jev-1.13"),
            ("typesafe-ai/jev", "typesafe-ai/jev"),
            ("typesafe-ai/jev-1.13.0", "jev-1.13.0"),
        ] {
            let (request, response) = exchange(requested, answered);
            assert!(
                validate(&response, &request).is_ok(),
                "{requested} {answered}"
            );
        }
        for (requested, answered) in [
            ("jev-1.13.0", "jev-1.14.0"),
            ("jev-1.13.0", "typesafe/jev-1.13"),
            ("jev-latest", "jev 1.13.0"),
            ("jev-latest", ""),
        ] {
            let (request, response) = exchange(requested, answered);
            assert!(
                validate(&response, &request).is_err(),
                "{requested} {answered}"
            );
        }
    }

    #[test]
    fn an_answer_without_usage_is_accepted_as_unmetered_and_cached_without_it() {
        let (request, mut response) = exchange("typesafe-ai/jev", "typesafe-ai/jev");
        assert_eq!(input_tokens(&response), Some(10));
        let cached = cache_value(&response, &request);
        assert_eq!(
            cached["usage"],
            json!({"input_tokens": 10, "output_tokens": 1})
        );
        for usage in [
            Value::Null,
            json!({"inputTokens": 10}),
            json!({"input_tokens": -1}),
        ] {
            response["usage"] = usage;
            assert!(validate(&response, &request).is_ok(), "{response}");
            assert_eq!(input_tokens(&response), None);
            assert!(cache_value(&response, &request).get("usage").is_none());
            assert!(validate(&cache_value(&response, &request), &request).is_ok());
        }
    }
}
