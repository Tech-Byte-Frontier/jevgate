//! GitHub Actions workflows: one unit per job that has something to judge.
//! A job whose `run` scripts hold `${{ }}` expressions is asked whether one can
//! hold text outside people write; a job of a workflow that runs on
//! `pull_request_target` or `workflow_run` is asked whether it runs pull
//! request code with secrets. Other jobs are not asked.
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
        let state = json!({
            "file": file.file_state(),
            "workflow": {"triggers": parsed.triggers, "permissions": parsed.permissions},
            "job": {"name": job.name, "source": job.source},
            "expressions": expressions,
        });
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
            recheck: None,
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
