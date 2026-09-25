//! Validation of provider responses against the request, and the cached form of an answer.
use anyhow::{Context, Result, ensure};
use serde_json::{Map, Value};

/// Providers round each probability to about two decimals: allow half a
/// hundredth per rounded value, at most `MAX_MASS_ERROR` in total, plus float noise.
const ROUNDING_PER_VALUE: f64 = 0.005;
const MAX_MASS_ERROR: f64 = 0.05;
const FLOAT_NOISE: f64 = 1e-9;

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
    for field in ["input_tokens", "output_tokens"] {
        ensure!(
            response["usage"][field]
                .as_u64()
                .is_some_and(|n| n <= 1_000_000_000),
            "Missing token usage"
        );
    }
    Ok(())
}

/// A well-formed model name, equal to the pinned model when one was requested.
fn validate_model(response: &Value, request: &Value) -> Result<()> {
    let model = response["model"]
        .as_str()
        .context("Missing model identity")?;
    ensure!(
        !model.is_empty()
            && model.len() <= 128
            && model.matches('/').count() <= 1
            && model.split('/').all(|part| {
                !part.is_empty()
                    && part
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
            }),
        "Invalid model identity"
    );
    if let Some(requested) = request["model"].as_str() {
        ensure!(
            model_matches_request(model, requested),
            "Provider returned a different pinned model"
        );
    }
    Ok(())
}

fn model_matches_request(model: &str, requested: &str) -> bool {
    if matches!(requested, "jev-latest" | "jev-preview") || model == requested {
        return true;
    }
    if matches!(requested, "typesafe/jev-latest" | "typesafe/jev-preview") {
        return dated_typesafe_model(model);
    }
    let Some(version) = requested
        .strip_prefix("typesafe/")
        .unwrap_or(requested)
        .strip_prefix("jev-")
    else {
        return false;
    };
    dated_typesafe_version(model, version)
}

fn dated_typesafe_model(model: &str) -> bool {
    model
        .strip_prefix("typesafe/jev-")
        .and_then(|s| s.rsplit_once('-'))
        .is_some_and(|(version, date)| {
            !version.is_empty() && date.len() == 8 && date.bytes().all(|b| b.is_ascii_digit())
        })
}

fn dated_typesafe_version(model: &str, version: &str) -> bool {
    model
        .strip_prefix(&format!("typesafe/jev-{version}-"))
        .is_some_and(|date| date.len() == 8 && date.bytes().all(|b| b.is_ascii_digit()))
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

pub fn cache_value(response: &Value, request: &Value) -> Value {
    let mut answers = serde_json::Map::new();
    for (key, question) in request["questions"].as_object().unwrap() {
        let kind = question["type"].as_str().unwrap();
        answers.insert(key.clone(), typed_fields(&response["answers"][key], kind));
    }
    serde_json::json!({"model":response["model"], "answers":answers,
        "usage":{"input_tokens":response["usage"]["input_tokens"], "output_tokens":response["usage"]["output_tokens"]}})
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

    #[test]
    fn openrouter_alias_accepts_its_versioned_typed_answers() {
        let request = json!({
            "model":"typesafe/jev-latest",
            "questions":{
                "kind":{"type":"choice","criteria":{"ball":"Ball","book":"Book"}},
                "size":{"type":"score","criteria":["Small","Medium","Large"]}
            }
        });
        let response = json!({
            "model":"typesafe/jev-1.13-20260917",
            "answers":{
                "kind":{"type":"choice","choice":"ball","confidence":1.0,
                    "probabilities":{"ball":1.0,"book":0.0}},
                "size":{"type":"score","score":0.0,"confidence":1.0,
                    "probabilities":{"0":1.0,"1":0.0,"2":0.0},
                    "legend":{"0":"Small","1":"Medium","2":"Large"}}
            },
            "usage":{"input_tokens":354,"output_tokens":45,"cost":0.000014868}
        });
        assert!(validate(&response, &request).is_ok());
    }

    #[test]
    fn openrouter_versioned_models_require_the_requested_version() {
        let response = json!({
            "model":"typesafe/jev-1.13-20260917",
            "answers":{},
            "usage":{"input_tokens":1,"output_tokens":1}
        });
        let wrong = json!({"model":"typesafe/jev-1.14-20260917","answers":{},
            "usage":{"input_tokens":1,"output_tokens":1}});
        for requested in ["typesafe/jev-1.13", "jev-1.13"] {
            let request = json!({"model":requested,"questions":{}});
            assert!(validate(&response, &request).is_ok());
            assert!(validate(&wrong, &request).is_err());
        }
    }
}
