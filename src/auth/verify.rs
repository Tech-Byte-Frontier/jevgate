//! Checking a key with its provider: one free request that sends no source,
//! whose answer says whether the key is valid.
use super::secret::Secret;
use crate::provider::{Endpoint, KeyAnswer, Service};
use anyhow::{Result, bail, ensure};
use serde_json::Value;
use std::time::Duration;

/// How long a key check may take.
const CHECK_TIMEOUT: Duration = Duration::from_secs(15);
/// The largest key-check answer read.
const MAX_ANSWER_BYTES: u64 = 1_048_576;

pub trait Verifier {
    fn verify(&self, key: &Secret) -> Result<()>;
}

#[derive(Debug)]
pub struct RejectedKey {
    status: u16,
    service: &'static Service,
}
impl std::fmt::Display for RejectedKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{} rejected this API key (HTTP {}); create a key at {} and run jevgate auth login",
            self.service.label, self.status, self.service.keys_page
        )
    }
}
impl std::error::Error for RejectedKey {}

pub fn authenticated(result: &Result<()>, checked: bool) -> Option<bool> {
    if !checked {
        return None;
    }
    match result {
        Ok(()) => Some(true),
        Err(error) if error.is::<RejectedKey>() => Some(false),
        Err(_) => None,
    }
}

impl Verifier for Endpoint {
    fn verify(&self, key: &Secret) -> Result<()> {
        let label = self.service.label;
        let (url, answer) = self.key_check();
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(CHECK_TIMEOUT))
            .max_redirects(0)
            .build()
            .into();
        let response = agent
            .get(&url)
            .header("Authorization", format!("Bearer {}", key.expose()))
            .call();
        let mut response = match response {
            Ok(response) => response,
            Err(ureq::Error::StatusCode(code)) => return http_error(self.service, code),
            Err(_) => bail!(
                "Could not reach {label} or the request timed out; check the connection and retry. The credential was not verified"
            ),
        };
        let body: Value = response
            .body_mut()
            .with_config()
            .limit(MAX_ANSWER_BYTES)
            .read_json()
            .map_err(|_| {
                anyhow::anyhow!(
                    "{label} returned invalid or oversized JSON; credential was not verified"
                )
            })?;
        valid_answer(self.service, answer, &body)
    }
}

pub fn http_error(service: &'static Service, code: u16) -> Result<()> {
    match code {
        401 | 403 => Err(RejectedKey {
            status: code,
            service,
        }
        .into()),
        _ => bail!(
            "{} returned HTTP {code}; credential was not verified and the request was not retried",
            service.label
        ),
    }
}

/// Whether a key check's answer has the shape a valid key gets; its text is
/// never echoed.
pub fn valid_answer(service: &Service, answer: KeyAnswer, body: &Value) -> Result<()> {
    let valid = match answer {
        KeyAnswer::Models => body["models"].as_array().is_some_and(|models| {
            models
                .iter()
                .all(|model| model["name"].as_str().is_some_and(|name| !name.is_empty()))
        }),
        KeyAnswer::Key => body["data"].is_object(),
        KeyAnswer::Credits => body["balance"].is_string() || body["balance"].is_number(),
    };
    ensure!(
        valid,
        "{} returned an unexpected answer; credential was not verified",
        service.label
    );
    Ok(())
}
