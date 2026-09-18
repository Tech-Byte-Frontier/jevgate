use super::secret::Secret;
use anyhow::{Result, bail, ensure};
use serde_json::Value;
use std::time::Duration;

pub trait Verifier {
    fn verify(&self, key: &Secret) -> Result<()>;
}
pub struct TypeSafe;
#[derive(Debug)]
pub struct RejectedKey(u16);
impl std::fmt::Display for RejectedKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "TypeSafe rejected this API key (HTTP {}); create a key at https://console.typesafe.ai/settings/keys and run jevgate auth login",
            self.0
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
impl Verifier for TypeSafe {
    fn verify(&self, key: &Secret) -> Result<()> {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(15)))
            .max_redirects(0)
            .build()
            .into();
        let response = agent
            .get("https://api.typesafe.ai/v1/models")
            .header("Authorization", format!("Bearer {}", key.expose()))
            .call();
        let mut response = match response {
            Ok(response) => response,
            Err(ureq::Error::StatusCode(code)) => return http_error(code),
            Err(_) => bail!(
                "Could not reach TypeSafe or the request timed out; check the connection and retry. The credential was not verified"
            ),
        };
        let body: Value = response.body_mut().with_config().limit(1_048_576).read_json()
            .map_err(|_| anyhow::anyhow!("TypeSafe returned invalid or oversized model-list JSON; credential was not verified"))?;
        validate_models(&body)
    }
}

pub fn http_error(code: u16) -> Result<()> {
    match code {
        401 | 403 => Err(RejectedKey(code).into()),
        _ => bail!(
            "TypeSafe returned HTTP {code}; credential was not verified and the request was not retried"
        ),
    }
}

pub fn validate_models(body: &Value) -> Result<()> {
    ensure!(
        body["models"].as_array().is_some_and(|models| models
            .iter()
            .all(|model| model["name"].as_str().is_some_and(|name| !name.is_empty()))),
        "TypeSafe returned an unexpected model list; credential was not verified"
    );
    Ok(())
}
