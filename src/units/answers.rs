//! The questions each request asks and recording their answers.
use super::questions;
use crate::schema::{Answer, FileResult, Judgment, Pass};
use anyhow::{Context, Result};
use serde_json::{Map, Value};

/// The questions one request asks, mapped back to units.
#[derive(Clone, Debug, Default)]
pub struct Asked {
    pub questions: Vec<AskedQuestion>,
}

#[derive(Clone, Debug)]
pub struct AskedQuestion {
    pub key: String,
    pub rule: &'static str,
    pub unit: String,
    pub question: &'static str,
    pub pass: Pass,
}

/// The questions of one request as they are built: the uploaded bodies and
/// what each maps back to.
#[derive(Default)]
pub(super) struct Questions {
    bodies: Map<String, Value>,
    asked: Asked,
}

impl Questions {
    pub(super) fn ask(
        &mut self,
        key: String,
        body: Value,
        unit: &str,
        rule: &'static str,
        question: &'static str,
        pass: Pass,
    ) {
        self.bodies.insert(key.clone(), body);
        self.asked.questions.push(AskedQuestion {
            key,
            rule,
            unit: unit.into(),
            question,
            pass,
        });
    }

    pub(super) fn finish(self) -> (Map<String, Value>, Asked) {
        (self.bodies, self.asked)
    }
}

/// Record the answers of one request as typed judgments on its owner file.
pub fn record(file: &mut FileResult, asked: &Asked, body: &Value) -> Result<()> {
    file.model = body["model"].as_str().map(str::to_owned);
    for question in &asked.questions {
        let answer: Answer = serde_json::from_value(body["answers"][&question.key].clone())
            .with_context(|| format!("Invalid answer for {}", question.key))?;
        file.judgments.retain(|j| {
            !(j.unit == question.unit && j.question == question.question && j.pass == question.pass)
        });
        file.judgments.push(Judgment {
            rule: question.rule.into(),
            unit: question.unit.clone(),
            question: question.question.into(),
            version: questions::VERSION.into(),
            pass: question.pass,
            answer,
        });
    }
    Ok(())
}
