use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

pub const RUBRIC: &str = "jevgate-units-v1";
/// Changes how saved answers become a status. Included in the report identity
/// and not in the judgment cache, so unchanged questions are not sent again.
pub const COMPOSITION: &str = "unit-composition-v6";
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

/// One rule's composed result for a file, over every unit it judged.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dimension {
    pub status: Status,
    /// Highest concern among the judged units.
    pub concern_probability: f64,
    pub decision_basis: String,
    pub rule_version: String,
    pub units: UnitCounts,
    /// Judged units whose answers stayed undecided, so a reader can see what
    /// an uncertain status is about.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub undecided: Vec<Undecided>,
}

/// A judged unit that stayed undecided, and the questions left undecided.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Undecided {
    pub unit: String,
    pub line: usize,
    pub questions: Vec<String>,
    /// The candidate values, for a hardcoded-value unit with only a few.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub values: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct UnitCounts {
    pub judged: usize,
    pub review: usize,
    pub consider: usize,
    /// Optional improvements: shown with `--verbose`, never failing the gate.
    #[serde(default)]
    pub note: usize,
    pub clear: usize,
    pub uncertain: usize,
    pub needs_context: usize,
    /// Bodies below the minimum size; never counted as clear.
    pub too_small: usize,
    /// Candidates beyond the per-file or per-run caps.
    pub omitted: usize,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct Location {
    pub path: PathBuf,
    pub start_line: usize,
    pub end_line: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbol: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
pub enum Strength {
    /// Optional: the code reads well as it is. Hidden unless `--verbose` and
    /// never counted by the gate.
    Note,
    Consider,
    Review,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ContextFile {
    pub path: PathBuf,
    pub source_hash: String,
    pub start_line: usize,
    pub end_line: usize,
    pub reason: String,
    pub dependent_rules: Vec<String>,
    pub complete: bool,
    #[serde(default)]
    pub line_ranges: Vec<SourceRange>,
    #[serde(default)]
    pub selection: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SourceRange {
    pub start_line: usize,
    pub end_line: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    pub rule: String,
    pub strength: Strength,
    pub line: usize,
    pub message: String,
    pub action: String,
    pub symbol: Option<String>,
    pub rule_version: String,
    pub concern_probability: f64,
    pub locations: Vec<Location>,
    /// Whole statements from the first location, when the evidence is a quote.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quote: Option<String>,
    /// The weakness a security finding names, such as "CWE-89 SQL injection".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// Rule, path, unit and normalized evidence; stable across unrelated edits.
    pub fingerprint: String,
    /// Probability times the log of the lines involved; orders findings.
    pub rank: f64,
    #[serde(default)]
    pub baselined: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileResult {
    pub path: PathBuf,
    pub role: String,
    pub contains_tests: bool,
    pub source_hash: String,
    #[serde(default)]
    pub catalog_hash: String,
    pub context_files: Vec<ContextFile>,
    pub syntax_checked: bool,
    pub context_complete: bool,
    #[serde(default)]
    pub context_expanded: bool,
    pub context_limitations: Vec<String>,
    pub context_requests: Vec<ContextNeed>,
    pub content_identity: String,
    pub symbols: Vec<String>,
    pub semantic_size: usize,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub status: Status,
    pub cached: bool,
    pub evaluated_at: Option<u64>,
    pub model: Option<String>,
    pub elapsed_ms: u64,
    pub dimensions: BTreeMap<String, Dimension>,
    /// Raw answers for every judged unit, first pass and recheck.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub judgments: Vec<Judgment>,
    pub findings: Vec<Finding>,
    pub error: Option<String>,
    /// Base file classification used to choose what the gates judge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub classification: Option<crate::file_kind::Classification>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StageMetrics {
    pub planned_requests: u64,
    pub planned_evidence_bytes: u64,
    pub elapsed_ms: u64,
    pub service_ms: u64,
    pub queue_wait_ms: u64,
    #[serde(default)]
    pub planned_tokens: u64,
    pub successful_requests: u64,
    pub failed_attempts: u64,
    /// Extra sends after rate limits, overload or connection failures.
    #[serde(default)]
    pub retries: u64,
    pub cache_hits: u64,
    pub cached_judgments: u64,
    pub evaluated_judgments: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub evidence_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    #[serde(default)]
    pub quick: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_revision: Option<String>,
    #[serde(default)]
    pub deleted_files: Vec<PathBuf>,
    pub schema_version: u32,
    pub command: String,
    pub rubric_version: String,
    pub root: PathBuf,
    pub generation: u64,
    pub watcher_pid: Option<u32>,
    pub errors: Vec<String>,
    pub generated_at: u64,
    pub status: String,
    pub complete: bool,
    pub judgments_complete: bool,
    pub acceptance_evaluated: bool,
    pub dry_run: bool,
    /// Explicit offline preview only. Later response-dependent requests are not yet known.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub initial_requests: Vec<serde_json::Value>,
    pub requested_model: String,
    pub decision_policy: BTreeMap<String, f64>,
    /// The configured gate policy; classification does not depend on it.
    #[serde(default)]
    pub fail_on: Vec<String>,
    /// Rules whose gate levels differ from `fail_on`, by rule ID.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub fail_on_rules: BTreeMap<String, Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate: Option<crate::gate::Gate>,
    pub api_requests: u32,
    #[serde(default)]
    pub concurrency: u32,
    pub paid_input_tokens: u64,
    pub paid_output_tokens: u64,
    #[serde(default)]
    pub stages: BTreeMap<String, StageMetrics>,
    pub settled: bool,
    pub files: Vec<FileResult>,
    pub changes: Vec<Change>,
}

impl Report {
    pub fn update_status(&mut self) {
        let selected: Vec<&FileResult> = self
            .files
            .iter()
            .filter(|f| f.status != Status::Skipped)
            .collect();
        // Skipped files (unsupported, unparseable or excluded) never make a run incomplete.
        self.complete = self.errors.is_empty()
            && !self.dry_run
            && (!self.files.is_empty() || self.base_revision.is_some())
            && selected.iter().all(|f| {
                matches!(
                    f.status,
                    Status::Clear
                        | Status::Consider
                        | Status::Note
                        | Status::Review
                        | Status::Uncertain
                        | Status::NeedsContext
                        | Status::NotApplicable
                )
            });
        self.judgments_complete = self.complete
            && selected.iter().all(|f| {
                f.dimensions.values().all(|d| {
                    matches!(
                        d.status,
                        Status::Clear
                            | Status::Note
                            | Status::Consider
                            | Status::Review
                            | Status::NotApplicable
                    )
                })
            });
        self.status = self.status_label(&selected).into();
    }

    /// The most severe outcome across the selected files.
    fn status_label(&self, selected: &[&FileResult]) -> &'static str {
        let any = |status: Status| selected.iter().any(|f| f.status == status);
        if !self.complete {
            "incomplete"
        } else if self.files.is_empty() && self.base_revision.is_some() {
            "no-changed-source"
        } else if selected.iter().all(|f| f.status == Status::NotApplicable) {
            "not-applicable"
        } else if any(Status::Review) {
            "review"
        } else if any(Status::Consider) {
            "consider"
        } else if any(Status::NeedsContext) {
            "needs-context"
        } else if !self.judgments_complete {
            "uncertain"
        } else if any(Status::Note) {
            "note"
        } else {
            "clear"
        }
    }
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextNeed {
    pub rule: String,
    pub kind: String,
    pub action: String,
    #[serde(default)]
    pub missing_fact: String,
    #[serde(default)]
    pub operation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Change {
    pub rule: String,
    pub path: PathBuf,
    pub previous_path: Option<PathBuf>,
    pub previous_generation: Option<u64>,
    pub state: String,
    pub reason: String,
    #[serde(default)]
    pub fingerprint: String,
}
