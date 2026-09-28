//! A finding as an agent reads it: the fields the MCP tools' structured
//! results give each finding, from which the agent hook also writes its
//! one-line findings, so an agent meets the same finding the same way
//! whichever door it came through.
use crate::{
    maturity::Labels,
    output::{self, Style},
    schema::{Finding, Gating, Strength},
};
use serde::Serialize;
use std::path::Path;

/// Probabilities to two decimals: an agent weighs 0.42, not 0.41999998.
pub(crate) fn rounded(probability: f64) -> f64 {
    (probability * 100.0).round() / 100.0
}

#[derive(Serialize)]
pub(crate) struct FindingView<'r> {
    pub id: &'r str,
    pub path: &'r Path,
    pub line: usize,
    pub end_line: usize,
    pub rule: &'r str,
    pub strength: Strength,
    /// Why it was found, then how often findings of its rule and level were
    /// right, as the agent text says it: "… Right 87% of the time (23
    /// labels)." or "… Not yet measured."; a note's message alone.
    pub message: String,
    pub action: &'r str,
    /// The probability of the answer that set its level: how sure that answer
    /// was, not how often such findings are right.
    pub probability: f64,
    /// Right of labeled findings of its rule and level on projects JevGate
    /// was never tuned on, as `jevgate rules` counts them; none for a note.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub precision: Option<Labels>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symbol: Option<&'r str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<&'r str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gate: Option<Gating>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub baselined: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suppressed: Option<&'r str>,
}

impl<'r> FindingView<'r> {
    pub(crate) fn new(path: &'r Path, finding: &'r Finding) -> Self {
        Self {
            id: &finding.fingerprint,
            path,
            line: finding.line,
            end_line: finding
                .locations
                .first()
                .map_or(finding.line, |l| l.end_line),
            rule: &finding.rule,
            strength: finding.strength,
            message: output::claim(path, finding, Style::PLAIN),
            action: &finding.action,
            probability: rounded(finding.concern_probability),
            precision: finding.precision,
            symbol: finding.symbol.as_deref(),
            category: finding.category.as_deref(),
            gate: finding.gate,
            baselined: finding.baselined,
            suppressed: finding.suppressed.as_deref(),
        }
    }

    /// Whether the gate counted it as a failure.
    pub(crate) fn fails(&self) -> bool {
        self.gate == Some(Gating::Fails)
    }
}
