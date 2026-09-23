//! The questions each request asks, recording their answers, and the
//! follow-up requests those answers call for.
use super::{Detail, Plan, Planned, UnitPlan, compose, questions};
use crate::schema::{Answer, FileResult, Judgment, Pass, Status};
use anyhow::{Context, Result};
use serde_json::{Map, Value};
use std::collections::BTreeSet;

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

impl Asked {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn ask(
        &mut self,
        questions: &mut Map<String, Value>,
        key: String,
        body: Value,
        unit: &str,
        rule: &'static str,
        question: &'static str,
        pass: Pass,
    ) {
        questions.insert(key.clone(), body);
        self.questions.push(AskedQuestion {
            key,
            rule,
            unit: unit.into(),
            question,
            pass,
        });
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

/// One locate follow-up per function whose split raised a review or consider.
pub fn locates(plan: &Plan, files: &[FileResult]) -> Vec<Planned> {
    follow_ups(plan, files, compose::unlocated_units, |unit| {
        match &unit.detail {
            Detail::Function { locate, .. } => locate.as_ref(),
            _ => None,
        }
    })
}

/// One recheck per unit that stayed uncertain after the first pass.
pub fn rechecks(plan: &Plan, files: &[FileResult]) -> Vec<Planned> {
    follow_ups(plan, files, compose::uncertain_units, |unit| {
        unit.recheck.as_ref()
    })
}

/// The planned follow-up of every selected unit, skipping failed files.
fn follow_ups(
    plan: &Plan,
    files: &[FileResult],
    select: fn(&super::FilePlan, &[Judgment]) -> BTreeSet<String>,
    follow_up: fn(&UnitPlan) -> Option<&(Value, Asked)>,
) -> Vec<Planned> {
    let mut planned = Vec::new();
    for (&owner, file_plan) in &plan.files {
        let file = &files[owner];
        if file.status == Status::Error {
            continue;
        }
        let selected = select(file_plan, &file.judgments);
        for unit in &file_plan.units {
            if let Some((request, asked)) = follow_up(unit)
                && selected.contains(&unit.id)
            {
                planned.push(Planned {
                    owner,
                    request: request.clone(),
                    asked: asked.clone(),
                });
            }
        }
    }
    planned
}
