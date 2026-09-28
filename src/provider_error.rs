//! What a failed provider response means: an unsent request, a context
//! limit, an edge-firewall block, exhausted credits, an invalid request, or
//! another HTTP status. Only verified machine codes are recognized; provider
//! text is never echoed.
use crate::provider::Service;
use serde_json::Value;
use std::time::Duration;

/// The connection failed before any request bytes were sent.
#[derive(Debug)]
pub(crate) struct Unsent(pub &'static Service);

impl std::error::Error for Unsent {}
impl std::fmt::Display for Unsent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Cannot connect to {}; request was not sent",
            self.0.label
        )
    }
}

/// The request was sent but no answer arrived: it timed out or the connection
/// dropped. The provider may have run it, so it is retried only once.
#[derive(Debug)]
pub(crate) struct Interrupted(pub &'static Service);

impl std::error::Error for Interrupted {}
impl std::fmt::Display for Interrupted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} request timed out or its connection dropped",
            self.0.label
        )
    }
}

/// Statuses worth another attempt: rate limits, overload, and server or
/// gateway errors (including the edge's 520–524 origin errors), which pass
/// on a later send.
pub(crate) fn retryable(status: u16) -> bool {
    matches!(status, 408 | 429 | 500 | 502 | 503 | 504 | 520..=524 | 529)
}

/// A failed response as it arrived: its status, body and headers.
#[derive(Default)]
pub(crate) struct Failure<'a> {
    pub status: u16,
    pub body: Option<&'a str>,
    pub retry_after: Option<Duration>,
    pub request_id: Option<String>,
}

#[derive(Debug)]
pub(crate) struct ProviderError {
    pub service: &'static Service,
    pub status: u16,
    pub context_limit: bool,
    /// A Cloudflare `error code: 10xx` page: the edge refused the client.
    pub edge_block: bool,
    pub retry_after: Option<Duration>,
    pub request_id: Option<String>,
    /// Where a 422 found the request invalid: each field's path and error type.
    pub invalid: Vec<String>,
    /// The provider does not know the model asked for.
    pub unknown_model: bool,
}

impl ProviderError {
    /// What the status means, when JevGate knows: the bracketed part of the message.
    fn meaning(&self) -> Option<String> {
        if self.context_limit {
            return Some("model context limit exceeded".into());
        }
        if self.edge_block {
            return Some("blocked by the provider's edge protection".into());
        }
        if self.unknown_model {
            return Some("unknown model; check the model name".into());
        }
        match self.status {
            402 => Some(format!("credits exhausted; {}", self.service.credits)),
            404 => Some("not found; check the model name".into()),
            422 if !self.invalid.is_empty() => {
                Some(format!("invalid request: {}", self.invalid.join(", ")))
            }
            _ => None,
        }
    }
}

impl std::error::Error for ProviderError {}
impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} HTTP {}", self.service.label, self.status)?;
        if let Some(meaning) = self.meaning() {
            write!(f, " ({meaning})")?;
        }
        if !retryable(self.status) {
            write!(f, "; request was not retried")?;
        }
        if let Some(id) = &self.request_id {
            write!(f, "; request id {id}")?;
        }
        Ok(())
    }
}

pub(crate) fn provider_error(service: &'static Service, failure: Failure<'_>) -> ProviderError {
    // Recognize only verified machine codes; do not echo arbitrary provider text.
    let status = failure.status;
    let json = failure
        .body
        .and_then(|text| serde_json::from_str::<Value>(text).ok());
    ProviderError {
        service,
        status,
        // A gateway refuses an oversized request with 413 before the model sees it.
        context_limit: status == 413
            || (status == 400
                && json
                    .as_ref()
                    .is_some_and(|body| body["detail"]["error_type"] == "max_tokens_exceeded")),
        edge_block: status == 403
            && failure.body.is_some_and(|text| {
                text.trim_start().starts_with("error code: 10")
                    || text.contains("<title>Attention Required! | Cloudflare</title>")
            }),
        retry_after: failure.retry_after,
        request_id: failure.request_id,
        invalid: if status == 422 {
            invalid_fields(json.as_ref())
        } else {
            Vec::new()
        },
        // TypeSafe answered `jev-1.13`, a name its docs use, with this on
        // 2026-09-28; only the message's fixed opening is read.
        unknown_model: status == 400
            && json.as_ref().is_some_and(|body| {
                body["detail"]["error_type"] == "api_usage_error"
                    && body["detail"]["message"]
                        .as_str()
                        .is_some_and(|message| message.starts_with("Unknown model:"))
            }),
    }
}

/// Validation failures a 422 names, at most this many.
const MAX_INVALID: usize = 3;
/// A path segment or type longer than this is not a field name.
const MAX_FIELD_BYTES: usize = 64;

/// Each `detail[]` entry of a 422 as its `loc` joined by dots and its `type`:
/// `body.questions.q1.criteria missing`. `msg` and `input` are never read,
/// since they can echo the request's source.
fn invalid_fields(body: Option<&Value>) -> Vec<String> {
    let Some(details) = body.and_then(|body| body["detail"].as_array()) else {
        return Vec::new();
    };
    details
        .iter()
        .take(MAX_INVALID)
        .map(|detail| {
            let location: Vec<String> = detail["loc"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|part| match part {
                    Value::Number(index) => index.to_string(),
                    other => field_name(other.as_str()),
                })
                .collect();
            format!(
                "{} {}",
                location.join("."),
                field_name(detail["type"].as_str())
            )
        })
        .collect()
}

/// A field name or error type fit to print, else `?`.
fn field_name(text: Option<&str>) -> String {
    text.filter(|text| {
        !text.is_empty()
            && text.len() <= MAX_FIELD_BYTES
            && text
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c))
    })
    .unwrap_or("?")
    .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::TYPESAFE;
    use serde_json::json;

    fn message(status: u16, body: Option<&str>, request_id: Option<&str>) -> String {
        let failure = Failure {
            status,
            body,
            request_id: request_id.map(Into::into),
            ..Default::default()
        };
        provider_error(&TYPESAFE, failure).to_string()
    }

    #[test]
    fn a_422_names_the_invalid_fields_but_never_their_message_or_input() {
        let body = json!({"detail": [
            {"loc": ["body", "questions", "simplify_0", "criteria"], "msg": "private source",
             "type": "missing", "input": {"source": "private source"}},
            {"loc": ["body", "state", 3], "msg": "private", "type": "string_too_long"},
            {"loc": ["body", "private source text"], "msg": "private", "type": "x"},
            {"loc": ["body", "model"], "msg": "private", "type": "fourth"},
        ]})
        .to_string();
        let text = message(422, Some(&body), Some("req_7"));
        assert_eq!(
            text,
            "TypeSafe HTTP 422 (invalid request: body.questions.simplify_0.criteria missing, body.state.3 string_too_long, body.? x); request was not retried; request id req_7"
        );
        assert!(!text.contains("private"));
        assert_eq!(
            message(422, Some("{\"detail\":\"private\"}"), None),
            "TypeSafe HTTP 422; request was not retried"
        );
        // TypeSafe's answer to a request without `state`, as received on
        // 2026-09-28: its `input` echoes the whole request.
        let real = r#"{"detail":[{"type":"missing","loc":["body","state"],"msg":"Field required","input":{"questions":{"q":{"type":"noul","instructions":"private question"}},"model":"jev-1.13.0"}}]}"#;
        assert_eq!(
            message(422, Some(real), None),
            "TypeSafe HTTP 422 (invalid request: body.state missing); request was not retried"
        );
    }

    #[test]
    fn credits_not_found_and_oversized_requests_say_what_to_do() {
        assert_eq!(
            message(402, Some("{\"error\":\"private\"}"), None),
            "TypeSafe HTTP 402 (credits exhausted; add credits or turn on auto-refill at https://console.typesafe.ai); request was not retried"
        );
        assert!(message(404, None, None).contains("(not found; check the model name)"));
        // TypeSafe's answer to `"model": "jev-1.13"`, as received on 2026-09-28.
        let unknown =
            r#"{"detail":{"error_type":"api_usage_error","message":"Unknown model: jev-1.13"}}"#;
        assert_eq!(
            message(400, Some(unknown), Some("req_01")),
            "TypeSafe HTTP 400 (unknown model; check the model name); request was not retried; request id req_01"
        );
        let other = r#"{"detail":{"error_type":"api_usage_error","message":"private"}}"#;
        assert_eq!(
            message(400, Some(other), None),
            "TypeSafe HTTP 400; request was not retried"
        );
        let oversized = provider_error(
            &TYPESAFE,
            Failure {
                status: 413,
                ..Default::default()
            },
        );
        assert!(oversized.context_limit);
        assert_eq!(
            message(503, None, Some("abc")),
            "TypeSafe HTTP 503; request id abc"
        );
    }
}
