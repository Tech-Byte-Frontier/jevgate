//! The services that answer Jev's questions: TypeSafe itself, and the
//! gateways that serve its API under their own keys, OpenRouter and Vercel AI
//! Gateway, at the same price. A key goes only to the host of the provider
//! that issued it, or to `JEVGATE_BASE_URL`, which only the environment sets:
//! nothing in a repository, which the change under review can edit, chooses
//! where a key is sent.
use anyhow::{Result, bail, ensure};
use clap::ValueEnum;

/// Whose key a check uses.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
pub enum Provider {
    /// A TypeSafe key
    #[default]
    Typesafe,
    /// An OpenRouter key
    Openrouter,
    /// A Vercel AI Gateway key
    Vercel,
}

impl Provider {
    /// Every provider, in the order a check reads their keys.
    pub const ALL: [Self; 3] = [Self::Typesafe, Self::Openrouter, Self::Vercel];

    pub fn service(self) -> &'static Service {
        match self {
            Self::Typesafe => &TYPESAFE,
            Self::Openrouter => &OPENROUTER,
            Self::Vercel => &VERCEL,
        }
    }

    /// The name `--provider`, the saved credential and the report use.
    pub fn name(self) -> &'static str {
        self.service().name
    }

    pub fn named(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|provider| provider.name() == name)
    }

    /// The provider whose keys start the way `key` does, when that is known.
    pub fn issuer(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|provider| {
            provider
                .service()
                .key_prefix
                .is_some_and(|prefix| key.starts_with(prefix))
        })
    }
}

/// One provider of TypeSafe's API.
#[derive(Debug)]
pub struct Service {
    pub name: &'static str,
    /// The name people read in messages.
    pub label: &'static str,
    /// The environment variable that holds its key.
    pub variable: &'static str,
    /// The API root; requests go to `<root>/v1/systemone`.
    pub api_root: &'static str,
    /// The model asked when neither `--model` nor `model` names one.
    pub default_model: &'static str,
    /// The most requests sent at once when neither `--concurrency` nor
    /// `concurrency` sets it.
    pub default_concurrency: u32,
    /// A free request that succeeds only with a valid key and sends no source.
    pub key_check: KeyCheck,
    /// Where to create a key.
    pub keys_page: &'static str,
    /// What to do when a request is refused for want of credits (HTTP 402).
    pub credits: &'static str,
    /// How its keys start, when that is known: a key with another
    /// provider's prefix is refused rather than sent to the wrong host.
    pub key_prefix: Option<&'static str>,
}

/// Where a key is checked, and what a valid answer holds.
#[derive(Debug)]
pub struct KeyCheck {
    pub url: &'static str,
    pub answer: KeyAnswer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyAnswer {
    /// TypeSafe's model list: `{"models": [{"name": …}]}`.
    Models,
    /// OpenRouter's key: `{"data": {…}}`.
    Key,
    /// Vercel's credit balance: `{"balance": "95.50", …}`.
    Credits,
}

/// TypeSafe itself. Its API documents no 402; its terms bill prepaid credits
/// that can refill automatically (MCA §8.2), managed in the console.
pub const TYPESAFE: Service = Service {
    name: "typesafe",
    label: "TypeSafe",
    variable: "TYPESAFE_API_KEY",
    api_root: "https://api.typesafe.ai",
    default_model: crate::options::DEFAULT_MODEL,
    default_concurrency: crate::options::MAX_CONCURRENCY,
    key_check: KeyCheck {
        url: "https://api.typesafe.ai/v1/models",
        answer: KeyAnswer::Models,
    },
    keys_page: "https://console.typesafe.ai/settings/keys",
    credits: "add credits or turn on auto-refill at https://console.typesafe.ai",
    key_prefix: None,
};

/// Requests sent at once by default with a gateway's key, half of TypeSafe's
/// 6. Six workers at JevGate's pacing make up to 1,200 requests a minute,
/// TypeSafe's limit for an account; through a gateway the account is the
/// gateway's, shared with its other customers. A precaution rather than a
/// measured fix: on 2026-09-28 OpenRouter answered 503 to about as many
/// attempts (63%) as TypeSafe's own endpoint did at the time (65%), and its
/// rounds of a few requests at once fared only a little better (50%, within
/// noise).
pub const GATEWAY_CONCURRENCY: u32 = 3;

/// OpenRouter serves TypeSafe's API at `/api/v1/systemone`. `typesafe/jev-1.13`
/// is its name for the Jev 1.13 line, the nearest to the `jev-1.13.0` whose
/// answers JevGate's thresholds were tuned on; `~typesafe/jev-latest` would
/// move to a new major version. It lists no pinned version.
pub const OPENROUTER: Service = Service {
    name: "openrouter",
    label: "OpenRouter",
    variable: "OPENROUTER_API_KEY",
    api_root: "https://openrouter.ai/api",
    default_model: "typesafe/jev-1.13",
    default_concurrency: GATEWAY_CONCURRENCY,
    key_check: KeyCheck {
        url: "https://openrouter.ai/api/v1/key",
        answer: KeyAnswer::Key,
    },
    keys_page: "https://openrouter.ai/settings/keys",
    credits: "add credits at https://openrouter.ai/settings/credits, or raise the key's limit",
    key_prefix: Some("sk-or-"),
};

/// Vercel AI Gateway serves TypeSafe's API under `/typesafe`, for Jev only
/// under the name `typesafe-ai/jev`.
pub const VERCEL: Service = Service {
    name: "vercel",
    label: "Vercel AI Gateway",
    variable: "AI_GATEWAY_API_KEY",
    api_root: "https://ai-gateway.vercel.sh/typesafe",
    default_model: "typesafe-ai/jev",
    default_concurrency: GATEWAY_CONCURRENCY,
    key_check: KeyCheck {
        url: "https://ai-gateway.vercel.sh/v1/credits",
        answer: KeyAnswer::Credits,
    },
    keys_page: "https://vercel.com/docs/ai-gateway/authentication-and-byok/api-keys",
    credits: "add AI Gateway credits in the Vercel dashboard, or raise its budget",
    key_prefix: Some("vck_"),
};

/// The environment variable that sends requests to another API root, for a
/// self-hosted proxy or a test server.
pub const BASE_URL: &str = "JEVGATE_BASE_URL";

/// Where a check sends its requests, and the provider that answers there.
#[derive(Debug)]
pub struct Endpoint {
    pub service: &'static Service,
    root: String,
    /// The root came from `JEVGATE_BASE_URL`.
    custom: bool,
}

impl Endpoint {
    /// The provider's API root, or `JEVGATE_BASE_URL` when the environment sets it.
    pub fn new(provider: Provider) -> Result<Self> {
        let service = provider.service();
        match std::env::var(BASE_URL) {
            Ok(value) if !value.trim().is_empty() => Self::custom(service, &value),
            Ok(_) | Err(std::env::VarError::NotPresent) => Ok(Self {
                service,
                root: service.api_root.into(),
                custom: false,
            }),
            Err(_) => bail!("{BASE_URL} must contain UTF-8 text"),
        }
    }

    /// Another API root: `https://` to any host, or `http://` to this machine
    /// only, so a key never crosses a network in clear text; no user, query
    /// or fragment.
    pub fn custom(service: &'static Service, value: &str) -> Result<Self> {
        let root = value.trim().trim_end_matches('/');
        let rest = root
            .strip_prefix("https://")
            .or_else(|| root.strip_prefix("http://").filter(|rest| loopback(rest)));
        ensure!(
            rest.is_some_and(|rest| !host(rest).is_empty()
                && rest
                    .bytes()
                    .all(|c| c.is_ascii_graphic() && !b"@?#\\".contains(&c))),
            "{BASE_URL} must be an https:// URL, or http:// to localhost, 127.0.0.1 or [::1], without a user, query or fragment"
        );
        Ok(Self {
            service,
            root: root.into(),
            custom: true,
        })
    }

    /// The URL questions are posted to.
    pub fn systemone(&self) -> String {
        format!("{}/v1/systemone", self.root)
    }

    /// Where the key is checked: the provider's own check, or at another
    /// root the TypeSafe model list that a proxy of TypeSafe's API serves.
    pub fn key_check(&self) -> (String, KeyAnswer) {
        if self.custom {
            (format!("{}/v1/models", self.root), KeyAnswer::Models)
        } else {
            let check = &self.service.key_check;
            (check.url.into(), check.answer)
        }
    }

    /// The API root, and whether `JEVGATE_BASE_URL` set it.
    pub fn describe(&self) -> String {
        if self.custom {
            format!("{} ({BASE_URL})", self.root)
        } else {
            self.root.clone()
        }
    }

    pub fn is_custom(&self) -> bool {
        self.custom
    }
}

/// The host of a URL after its scheme: `127.0.0.1` of `127.0.0.1:4010/api`.
fn host(rest: &str) -> &str {
    let authority = rest.split('/').next().unwrap_or_default();
    if authority.starts_with('[') {
        authority.split_inclusive(']').next().unwrap_or_default()
    } else {
        authority.split(':').next().unwrap_or_default()
    }
}

/// Whether a URL's host is this machine.
fn loopback(rest: &str) -> bool {
    matches!(host(rest), "localhost" | "127.0.0.1" | "[::1]")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_custom_root_is_https_or_this_machine_and_carries_no_credentials() {
        for root in [
            "https://proxy.example.com/typesafe/",
            "http://127.0.0.1:4010/api",
            "http://localhost:8080",
            "http://[::1]:9000",
        ] {
            let endpoint = Endpoint::custom(&TYPESAFE, root).unwrap();
            assert!(!endpoint.systemone().contains("//v1"), "{root}");
            assert!(endpoint.describe().ends_with("(JEVGATE_BASE_URL)"));
        }
        for root in [
            "http://proxy.example.com",
            "http://127.0.0.1.example.com",
            "http://localhost@example.com",
            "https://user:pass@example.com",
            "https://example.com/?key=1",
            "https://example.com/#x",
            "https://exa mple.com",
            "ftp://127.0.0.1",
            "https://",
            "127.0.0.1:4010",
        ] {
            assert!(Endpoint::custom(&TYPESAFE, root).is_err(), "{root}");
        }
        let custom = Endpoint::custom(&OPENROUTER, "http://127.0.0.1:1/api").unwrap();
        assert_eq!(custom.systemone(), "http://127.0.0.1:1/api/v1/systemone");
        assert_eq!(custom.key_check().0, "http://127.0.0.1:1/api/v1/models");
    }

    #[test]
    fn each_provider_has_its_own_names_and_keys() {
        for provider in Provider::ALL {
            let value = provider.to_possible_value().unwrap();
            assert_eq!(value.get_name(), provider.name());
            assert_eq!(Provider::named(provider.name()), Some(provider));
            if let Some(prefix) = provider.service().key_prefix {
                assert_eq!(Provider::issuer(&format!("{prefix}abc")), Some(provider));
            }
        }
        assert_eq!(Provider::issuer("tsk-abc"), None);
        assert_eq!(
            (OPENROUTER.api_root, OPENROUTER.default_model),
            ("https://openrouter.ai/api", "typesafe/jev-1.13")
        );
        assert_eq!(
            (VERCEL.api_root, VERCEL.default_model),
            ("https://ai-gateway.vercel.sh/typesafe", "typesafe-ai/jev")
        );
    }
}
