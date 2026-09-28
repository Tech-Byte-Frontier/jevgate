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
/// The saved calibration is a few bytes; a larger file is not read.
const BUDGET_READ_BYTES: u64 = 4096;
/// Bytes per token before calibration, and the range a calibration may set.
const DEFAULT_BYTES_PER_TOKEN: f64 = 3.0;
const MIN_BYTES_PER_TOKEN: f64 = 2.0;
const MAX_BYTES_PER_TOKEN: f64 = 6.0;
/// Bytes per token of evidence that is mostly JSON structure, such as an
/// outline's member list, at most: TypeScript and Bend 2 outlines measured
/// 2.29 and 2.19 bytes per token against 3.37 and 2.98 for their functions,
/// so a project's calibrated average let a 328-member outline of 77 KB
/// through that the provider refused as beyond its context.
const STRUCTURED_BYTES_PER_TOKEN: f64 = 2.0;

/// The budget as a run applies it to one request: the calibrated estimate of
/// what a run would send of it, the questions the answer cache does not
/// answer with their state. The calibration follows the fresh requests of the
/// last run, so a request near the limit fit in one run and not the next: two
/// runs of one release on a pinned project differed in a file's recheck, and
/// so in its finding. A request answered once fits from then on, and one
/// whose questions are partly answered is judged by the rest.
#[derive(Clone, Copy)]
pub struct Limits<'a> {
    budget: &'a TokenBudget,
    /// The request with only its unanswered questions; none when the cache
    /// answers them all.
    unanswered: &'a dyn Fn(&Value) -> Option<Value>,
}

impl<'a> Limits<'a> {
    pub fn new(budget: &'a TokenBudget, unanswered: &'a dyn Fn(&Value) -> Option<Value>) -> Self {
        Self { budget, unanswered }
    }

    pub fn fits(&self, request: &Value) -> bool {
        self.budget.fits(request)
            || (self.unanswered)(request).is_none_or(|sent| self.budget.fits(&sent))
    }

    pub fn fits_structured(&self, request: &Value) -> bool {
        self.budget.fits_structured(request)
            || (self.unanswered)(request).is_none_or(|sent| self.budget.fits_structured(&sent))
    }
}

#[cfg(test)]
impl TokenBudget {
    /// The estimate alone, for plans made without an answer cache.
    pub fn uncached(&self) -> Limits<'_> {
        fn everything(request: &Value) -> Option<Value> {
            Some(request.clone())
        }
        Limits::new(self, &everything)
    }
}

/// The bytes-per-token ratio, calibrated from observed `usage.input_tokens` and
/// saved in `.jevgate/`.
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct TokenBudget {
    pub bytes_per_token: f64,
}

impl Default for TokenBudget {
    fn default() -> Self {
        Self {
            bytes_per_token: DEFAULT_BYTES_PER_TOKEN,
        }
    }
}

impl TokenBudget {
    pub fn load(root: &std::path::Path) -> Self {
        crate::inventory::read_source(&root.join(".jevgate").join(BUDGET_FILE), BUDGET_READ_BYTES)
            .ok()
            .and_then(|text| serde_json::from_str::<Self>(&text).ok())
            .map(|b| Self::calibrated(b.bytes_per_token))
            .unwrap_or_default()
    }

    fn calibrated(bytes_per_token: f64) -> Self {
        Self {
            bytes_per_token: if bytes_per_token.is_finite() {
                bytes_per_token.clamp(MIN_BYTES_PER_TOKEN, MAX_BYTES_PER_TOKEN)
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

    /// `fits` for a request whose evidence is mostly JSON structure.
    pub fn fits_structured(&self, request: &Value) -> bool {
        Self {
            bytes_per_token: self.bytes_per_token.min(STRUCTURED_BYTES_PER_TOKEN),
        }
        .fits(request)
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_request_fits_when_what_it_would_send_fits() {
        // About 60,000 tokens at 3 bytes a token: past the limit whole, while
        // its state with one question is about 21,000.
        let question = json!({"type": "noul", "instructions": {"question": "x".repeat(3_000)}});
        let questions: serde_json::Map<String, Value> = (0..40)
            .map(|i| (format!("q{i}"), question.clone()))
            .collect();
        let request =
            json!({"model": "m", "state": {"source": "y".repeat(60_000)}, "questions": questions});
        let budget = TokenBudget::default();
        assert!(!budget.uncached().fits(&request));
        let one_left = |request: &Value| {
            let mut sent = request.clone();
            sent["questions"] = json!({"q0": request["questions"]["q0"]});
            Some(sent)
        };
        assert!(Limits::new(&budget, &one_left).fits(&request));
        let answered = |_: &Value| None;
        assert!(Limits::new(&budget, &answered).fits(&request));
    }
}
