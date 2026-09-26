//! GitHub Actions workflows: one unit per job that has something to judge.
//! A job whose `run` scripts hold `${{ }}` expressions is asked whether one can
//! hold text outside people write; a job of a workflow that runs on
//! `pull_request_target` or `workflow_run` is asked whether it runs pull
//! request code with secrets. Other jobs are not asked. A job left
//! undecided is asked, in a recheck, which expression holds outside text
//! and what code it runs, as Choices that can only clear.
use super::{Detail, FileContext, FilePlan, Planned, Presence, Questions, UnitPlan};
use super::{compact, identity, questions};
use crate::{analysis::workflow, catalog::WORKFLOWS, schema::Pass};
use serde_json::json;
use std::collections::BTreeMap;

pub(super) fn plan(file: &FileContext<'_>, out: &mut FilePlan, requests: &mut Vec<Planned>) {
    out.rules = BTreeMap::from([(WORKFLOWS, 0)]);
    let parsed = workflow::parse(file.source);
    for job in &parsed.jobs {
        let id = format!("job:{}", job.name);
        let expressions: Vec<String> = job.run_expressions.iter().map(|(_, e)| e.clone()).collect();
        let mut questions = Questions::default();
        let mut ask = |question: &'static str, body| {
            questions.ask(question.into(), body, &id, WORKFLOWS, question, Pass::First);
        };
        if !expressions.is_empty() {
            ask("outside", questions::workflow_outside());
        }
        if parsed.privileged_trigger() {
            ask("untrusted", questions::workflow_untrusted());
        }
        let workflow = json!({"triggers": parsed.triggers, "permissions": parsed.permissions});
        let state = json!({
            "file": file.file_state(),
            "workflow": workflow,
            "job": {"name": job.name, "source": job.source},
            "expressions": expressions,
        });
        let recheck = recheck(file, &id, &state, parsed.privileged_trigger());
        let (request, asked) = file.request("workflows", state, questions);
        if asked.questions.is_empty() {
            continue;
        }
        let fits = file.budget.fits(&request);
        out.units.push(UnitPlan {
            rule: WORKFLOWS,
            id: id.clone(),
            name: job.name.clone(),
            presence: if fits {
                Presence::Judged
            } else {
                Presence::NeedsContext
            },
            locations: vec![file.location(job.start_line, job.end_line, Some(&job.name))],
            quote: None,
            lines: job.end_line + 1 - job.start_line,
            identity: identity(&[&id, &compact(&job.source)]),
            detail: Detail::Job { expressions },
            recheck: recheck
                .filter(|(request, _)| file.budget.fits(request))
                .map(Into::into),
        });
        if fits {
            requests.push(Planned {
                owner: file.owner,
                request,
                asked,
            });
        }
    }
}

/// The Choices asked of a job that stays undecided: which expression of its
/// scripts holds outside text (the expressions listed with ids), and for a
/// workflow run on `pull_request_target` or `workflow_run`, what code the
/// job runs.
fn recheck(
    file: &FileContext<'_>,
    id: &str,
    state: &serde_json::Value,
    privileged: bool,
) -> Option<(serde_json::Value, super::Asked)> {
    let expressions = state["expressions"].as_array()?;
    let ids: Vec<String> = (0..expressions.len()).map(|i| format!("e{i}")).collect();
    let mut questions = Questions::default();
    if !expressions.is_empty() {
        questions.ask(
            "outside_source".into(),
            questions::workflow_outside_source(&ids),
            id,
            WORKFLOWS,
            "outside_source",
            Pass::Recheck,
        );
    }
    if privileged {
        questions.ask(
            "pull_request_code".into(),
            questions::workflow_code(),
            id,
            WORKFLOWS,
            "pull_request_code",
            Pass::Recheck,
        );
    }
    let mut state = state.clone();
    state["expressions"] = json!(
        ids.iter()
            .zip(expressions)
            .map(|(id, e)| json!({"id": id, "expression": e}))
            .collect::<Vec<_>>()
    );
    let (request, asked) = file.request("recheck", state, questions);
    (!asked.questions.is_empty()).then_some((request, asked))
}
