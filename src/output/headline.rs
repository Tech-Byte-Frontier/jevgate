//! The headline, the first line of every format's output: the run's status,
//! the gate, what a change-scoped check judged, the requests and their
//! cost; for a dry run, what it plans to ask.
use crate::schema::{Report, Scope};

/// The headline's cost: estimated dollars, or unknown, never a guessed $0.
pub(crate) fn cost(usd: Option<f64>) -> String {
    usd.map_or(" · cost unknown".into(), |usd| format!(" · ~${usd:.4}"))
}

/// What a dry run plans: first-pass requests and their questions, those the
/// cache answers, and the estimated input tokens and dollars of what the
/// rest send: a request sends only the questions the cache lacks.
#[derive(serde::Serialize)]
pub(crate) struct Preview {
    pub requests: u64,
    pub cached: u64,
    pub questions: u64,
    pub cached_questions: u64,
    pub tokens: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usd: Option<f64>,
}

pub(crate) fn preview(report: &Report) -> Preview {
    let total = |field: fn(&crate::schema::StageMetrics) -> u64| {
        report.stages.values().map(field).sum::<u64>()
    };
    let tokens = total(|s| s.planned_tokens);
    Preview {
        requests: total(|s| s.planned_requests),
        cached: total(|s| s.planned_cached),
        questions: total(|s| s.planned_questions),
        cached_questions: total(|s| s.planned_cached_questions),
        tokens,
        usd: crate::model::usd(&report.requested_model, tokens),
    }
}

/// Status, gate, scope and cost on one line; for a dry run, the planned
/// requests and questions and the cost of the questions the cache does not
/// answer.
pub(crate) fn headline(report: &Report) -> String {
    if report.dry_run {
        let Preview {
            requests,
            cached,
            questions,
            cached_questions,
            tokens,
            usd,
        } = preview(report);
        return format!(
            "JevGate: dry run · {} files{} · {requests} first-pass requests, {cached} answered by the cache · {questions} questions, {cached_questions} answered by the cache · ~{tokens} new input tokens{}; follow-ups depend on the answers",
            report.files.len(),
            since(report),
            cost(usd)
        );
    }
    let gate = match &report.gate {
        Some(gate) if gate.passed => "gate passed".to_string(),
        Some(gate) => format!("gate failed: {}", gate.reasons.join("; ")),
        None => "gate not evaluated".to_string(),
    };
    let cost = cost(report.estimated_usd);
    format!(
        "JevGate: {} · {gate} · {} files{} · {} API requests{} · {} input tokens{cost}",
        report.status,
        report.files.len(),
        since(report),
        report.api_requests,
        via(report),
        report.paid_input_tokens
    )
}

/// ` via OpenRouter` when a gateway answered, so a key found in the
/// environment never bills another account unseen; nothing for TypeSafe.
fn via(report: &Report) -> String {
    crate::provider::Provider::named(&report.provider).map_or(String::new(), through)
}

/// ` via <gateway>` for a gateway's key; nothing for TypeSafe's.
pub(crate) fn through(provider: crate::provider::Provider) -> String {
    if provider == crate::provider::Provider::Typesafe {
        String::new()
    } else {
        format!(" via {}", provider.service().label)
    }
}

/// Characters of a commit id shown, as Git abbreviates it.
const SHORT_COMMIT: usize = 7;

/// With a base revision, what the check judged since it: ` · changed lines
/// since 1a2b3c4` or ` · whole files changed since 1a2b3c4`; ` · staged
/// lines since 1a2b3c4` for the index, and ` · changed lines from 1a2b3c4
/// to 9f8e7d6` for a pushed commit.
fn since(report: &Report) -> String {
    let Some(base) = &report.base_revision else {
        return String::new();
    };
    let short = |id: &str| id.get(..SHORT_COMMIT).unwrap_or(id).to_string();
    let judged = match (report.scope, report.staged) {
        (Scope::ChangedLines, false) => "changed lines",
        (Scope::WholeFiles, false) => "whole files changed",
        (Scope::ChangedLines, true) => "staged lines",
        (Scope::WholeFiles, true) => "whole files staged",
    };
    match &report.pushed_revision {
        Some(pushed) => format!(" · {judged} from {} to {}", short(base), short(pushed)),
        None => format!(" · {judged} since {}", short(base)),
    }
}
