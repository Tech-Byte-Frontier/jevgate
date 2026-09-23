//! Token estimates from request bytes, calibrated from observed usage. They
//! decide packing and whether a unit fits the provider limits, never a verdict.
use crate::requests::provider_request;
use anyhow::Result;
use serde_json::Value;

/// Provider context limits: all questions plus state, and state plus the longest question.
const TOTAL_TOKENS: f64 = 64_000.0;
const STATE_TOKENS: f64 = 32_000.0;
/// Headroom for estimation error.
const MARGIN: f64 = 0.9;
const BUDGET_FILE: &str = "token-budget.json";

/// The bytes-per-token ratio, calibrated from observed `usage.input_tokens` and
/// saved in `.jevgate/`.
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct TokenBudget {
    pub bytes_per_token: f64,
}

impl Default for TokenBudget {
    fn default() -> Self {
        Self {
            bytes_per_token: 3.0,
        }
    }
}

impl TokenBudget {
    pub fn load(root: &std::path::Path) -> Self {
        crate::inventory::read_source(&root.join(".jevgate").join(BUDGET_FILE), 4096)
            .ok()
            .and_then(|text| serde_json::from_str::<Self>(&text).ok())
            .map(|b| Self::calibrated(b.bytes_per_token))
            .unwrap_or_default()
    }

    fn calibrated(bytes_per_token: f64) -> Self {
        Self {
            bytes_per_token: if bytes_per_token.is_finite() {
                bytes_per_token.clamp(2.0, 6.0)
            } else {
                Self::default().bytes_per_token
            },
        }
    }

    /// Replace the ratio with one observed over a batch of fresh requests.
    pub fn observe(&mut self, bytes: u64, tokens: u64) {
        if tokens > 0 {
            *self = Self::calibrated(bytes as f64 / tokens as f64);
        }
    }

    pub fn save(&self, store: &crate::storage::Store) -> Result<()> {
        store.write(BUDGET_FILE, &serde_json::to_vec(self)?)
    }

    pub fn tokens(&self, bytes: usize) -> usize {
        (bytes as f64 / self.bytes_per_token).ceil() as usize
    }

    pub fn tokens_of(&self, value: &Value) -> usize {
        self.tokens(serde_json::to_vec(value).map_or(0, |v| v.len()))
    }

    /// Estimated uploaded tokens of a request.
    pub fn request_tokens(&self, request: &Value) -> usize {
        self.tokens_of(&provider_request(request))
    }

    pub fn fits(&self, request: &Value) -> bool {
        let provider = provider_request(request);
        let state = self.tokens_of(&provider["state"]) as f64;
        let longest = provider["questions"]
            .as_object()
            .into_iter()
            .flat_map(|q| q.values())
            .map(|q| self.tokens_of(q))
            .max()
            .unwrap_or(0) as f64;
        (self.tokens_of(&provider) as f64) <= TOTAL_TOKENS * MARGIN
            && state + longest <= STATE_TOKENS * MARGIN
    }
}
