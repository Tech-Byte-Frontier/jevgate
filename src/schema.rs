use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

pub const RUBRIC: &str = "jevgate-quality-v33";
/// Changes how saved answers become a status. Included in the report identity
/// and not in the judgment cache, so unchanged questions are not sent again.
pub const COMPOSITION: &str = "file-wide-status-v2";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    Pending,
    NotApplicable,
    Clear,
    Review,
    Uncertain,
    NeedsContext,
    Error,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dimension {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refactoring_assessment: Option<crate::maintainability::Assessment>,
    pub score: f64,
    pub confidence: f64,
    pub probabilities: BTreeMap<String, f64>,
    pub concern_probability: f64,
    #[serde(default)]
    pub concern_basis: String,
    pub missing_context: f64,
    #[serde(default)]
    pub evidence_sufficiency: Option<f64>,
    #[serde(default)]
    pub protection_probability: Option<f64>,
    #[serde(default)]
    pub protection_checks: BTreeMap<String, f64>,
    #[serde(default)]
    pub decision_basis: String,
    pub status: Status,
    pub rule_version: String,
    pub applicability: String,
    #[serde(default)]
    pub applicability_probability: Option<f64>,
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
    pub line: usize,
    pub message: String,
    pub action: String,
    pub symbol: Option<String>,
    pub rule_version: String,
    pub concern_probability: f64,
    pub evidence_complete: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role_assessment: Option<crate::roles::Assessment>,
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
    /// Original file-level judgments before focused follow-up; never discarded.
    #[serde(default)]
    pub file_dimensions: BTreeMap<String, Dimension>,
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
    pub successful_requests: u64,
    pub failed_attempts: u64,
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
        let selected: Vec<_> = self
            .files
            .iter()
            .filter(|f| f.status != Status::Skipped)
            .collect();
        self.complete = self.errors.is_empty()
            && !self.dry_run
            && (!selected.is_empty() || self.base_revision.is_some())
            && selected.iter().all(|f| {
                matches!(
                    f.status,
                    Status::Clear
                        | Status::Review
                        | Status::Uncertain
                        | Status::NeedsContext
                        | Status::NotApplicable
                )
            });
        self.judgments_complete = self.complete
            && selected.iter().all(|f| {
                if self.command == "classify-roles" {
                    return f.status == Status::Clear;
                }
                f.dimensions.values().all(|d| {
                    matches!(
                        d.status,
                        Status::Clear | Status::Review | Status::NotApplicable
                    )
                })
            });
        self.status = if !self.complete {
            "incomplete"
        } else if selected.is_empty() && self.base_revision.is_some() {
            "no-changed-source"
        } else if selected.iter().all(|f| f.status == Status::NotApplicable) {
            "not-applicable"
        } else if selected.iter().any(|f| f.status == Status::Review) {
            "review"
        } else if selected.iter().any(|f| f.status == Status::NeedsContext) {
            "needs-context"
        } else if !self.judgments_complete {
            "uncertain"
        } else {
            "clear"
        }
        .into();
        if self.command == "classify-roles" && self.status == "clear" {
            self.status = "classified".into();
        }
    }
}

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

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
}
