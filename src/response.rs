use anyhow::{Context, Result, ensure};
use serde_json::Value;

pub fn validate(response: &Value, request: &Value) -> Result<()> {
    let model = response["model"]
        .as_str()
        .context("Missing model identity")?;
    ensure!(
        !model.is_empty()
            && model.len() <= 128
            && model
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c)),
        "Invalid model identity"
    );
    if let Some(requested) = request["model"].as_str() {
        ensure!(
            matches!(requested, "jev-latest" | "jev-preview") || model == requested,
            "Provider returned a different pinned model"
        );
    }
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

fn probability(value: &Value) -> Result<f64> {
    let p = value.as_f64().context("Non-numeric probability")?;
    ensure!(
        p.is_finite() && (0.0..=1.0).contains(&p),
        "Invalid probability"
    );
    Ok(p)
}

fn validate_distribution(answer: &Value, question: &Value) -> Result<()> {
    probability(&answer["confidence"])?;
    let probabilities = answer["probabilities"]
        .as_object()
        .context("Missing probabilities")?;
    let keys: Vec<String> = if question["type"] == "score" {
        (0..question["criteria"]
            .as_array()
            .context("Invalid rubric")?
            .len())
            .map(|i| i.to_string())
            .collect()
    } else {
        question["criteria"]
            .as_object()
            .context("Invalid choices")?
            .keys()
            .cloned()
            .collect()
    };
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
        (sum - 1.0).abs() <= (0.005 * keys.len() as f64).min(0.05) + 1e-9,
        "Invalid probability mass"
    );
    if question["type"] == "score" {
        let score = answer["score"].as_f64().context("Missing score")?;
        let expected = keys
            .iter()
            .enumerate()
            .map(|(i, k)| i as f64 * probabilities[k].as_f64().unwrap())
            .sum::<f64>();
        let rounding = 0.005 * (1 + (0..keys.len()).sum::<usize>()) as f64 + 1e-9;
        ensure!(
            (0.0..=(keys.len() - 1) as f64).contains(&score)
                && (score - expected).abs() <= rounding,
            "Inconsistent score"
        );
    } else {
        let choice = answer["choice"].as_str().context("Missing choice")?;
        ensure!(probabilities.contains_key(choice), "Invalid choice");
        let chosen = probability(&probabilities[choice])?;
        ensure!(
            probabilities
                .values()
                .all(|v| v.as_f64().unwrap() <= chosen + 0.01),
            "Choice is not a highest-probability option"
        );
    }
    Ok(())
}

pub(crate) const REVIEW_PROBABILITY: f64 = 0.80;
pub(crate) const LOCATION_PROBABILITY: f64 = 0.65;

/// Aggregating and normalizing binary floats can move an exact decimal boundary
/// by a few machine rounding units. This is only an arithmetic allowance, not a
/// confidence margin; raw probabilities and the configured thresholds stay intact.
pub(crate) fn probability_at_least(value: f64, threshold: f64) -> bool {
    value.is_finite()
        && threshold.is_finite()
        && (value >= threshold || threshold - value <= 8.0 * f64::EPSILON)
}

pub fn cache_value(response: &Value, request: &Value) -> Value {
    let mut answers = serde_json::Map::new();
    for (key, question) in request["questions"].as_object().unwrap() {
        let answer = &response["answers"][key];
        let fields: &[&str] = match question["type"].as_str().unwrap() {
            "score" => &["type", "score", "confidence", "probabilities"],
            "choice" => &["type", "choice", "confidence", "probabilities"],
            _ => &["type", "noul"],
        };
        let clean: serde_json::Map<_, _> = fields
            .iter()
            .map(|f| (f.to_string(), answer[*f].clone()))
            .collect();
        answers.insert(key.clone(), Value::Object(clean));
    }
    serde_json::json!({"model":response["model"], "answers":answers,
        "usage":{"input_tokens":response["usage"]["input_tokens"], "output_tokens":response["usage"]["output_tokens"]}})
}
