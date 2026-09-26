//! The report schema's versions, statuses, raw answers and judgments, and the
//! hash that names cached answers; `report` holds the report's structure.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

mod report;
pub use report::*;

pub const RUBRIC: &str = "jevgate-units-v1";
/// Changes how saved answers become a status. Included in the report identity
/// and not in the judgment cache, so unchanged questions are not sent again.
pub const COMPOSITION: &str = "unit-composition-v12";
pub const SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    Pending,
    NotApplicable,
    Clear,
    Consider,
    Review,
    /// Only optional improvements: the code reads well as it is.
    Note,
    Uncertain,
    NeedsContext,
    Error,
    Skipped,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
pub enum Pass {
    First,
    Recheck,
    /// A follow-up that locates the part of a finding to act on.
    Locate,
    /// A follow-up that judges where a security concern's values come from.
    Trace,
    /// A follow-up that asks where an undecided security check's URL comes
    /// from or its output goes, and can only clear that check.
    Settle,
}

/// A raw typed answer, kept exactly as the provider returned it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Answer {
    Noul {
        noul: f64,
    },
    Choice {
        choice: String,
        confidence: f64,
        probabilities: BTreeMap<String, f64>,
    },
    Score {
        score: f64,
        confidence: f64,
        probabilities: BTreeMap<String, f64>,
    },
}

/// One answer about one unit. First-pass and recheck answers are both kept.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Judgment {
    pub rule: String,
    pub unit: String,
    pub question: String,
    pub version: String,
    pub pass: Pass,
    pub answer: Answer,
}

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Joins the parts of a hashed identity; it cannot occur in a path or in source.
pub const HASH_SEPARATOR: &str = "\u{0}";

pub fn hash(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
