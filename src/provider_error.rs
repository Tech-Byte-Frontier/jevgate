//! What a failed provider response means: an unsent request, a context
//! limit, an edge-firewall block, or another HTTP status. Only verified
//! machine codes are recognized; provider text is never echoed.
use serde_json::Value;

/// The connection failed before any request bytes were sent.
#[derive(Debug)]
pub(crate) struct Unsent;

impl std::error::Error for Unsent {}
impl std::fmt::Display for Unsent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Cannot connect to TypeSafe; request was not sent")
    }
}

#[derive(Debug)]
pub(crate) struct ProviderError {
    pub status: u16,
    pub context_limit: bool,
    /// A Cloudflare `error code: 10xx` page: the edge refused the client.
    pub edge_block: bool,
    pub retry_after: Option<u64>,
}

impl std::error::Error for ProviderError {}
impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let detail = if self.context_limit {
            " (model context limit exceeded)"
        } else if self.edge_block {
            " (blocked by the provider's edge protection)"
        } else {
            ""
        };
        let retried = if matches!(self.status, 429 | 502 | 503 | 504 | 529) {
            ""
        } else {
            "; request was not retried"
        };
        write!(f, "TypeSafe HTTP {}{detail}{retried}", self.status)
    }
}

pub(crate) fn provider_error(
    status: u16,
    body: Option<&str>,
    retry_after: Option<u64>,
) -> ProviderError {
    // Recognize only verified machine codes; do not echo arbitrary provider text.
    let json = body.and_then(|text| serde_json::from_str::<Value>(text).ok());
    ProviderError {
        status,
        context_limit: status == 400
            && json.is_some_and(|body| body["detail"]["error_type"] == "max_tokens_exceeded"),
        edge_block: status == 403
            && body.is_some_and(|text| {
                text.trim_start().starts_with("error code: 10")
                    || text.contains("<title>Attention Required! | Cloudflare</title>")
            }),
        retry_after,
    }
}
