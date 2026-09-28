//! Where a check finds its key: the environment, then a credential file, then
//! the key saved by `jevgate auth login`. The first key found is used, and it
//! goes only to the provider that issued it.
use super::{
    secret::Secret,
    store::{self, NativeBackend, SavedCredentials, SavedKey, StorageMode},
};
use crate::provider::Provider;
use anyhow::{Context, Result, bail, ensure};
use std::{io::Read, path::Path};

pub struct Credential {
    pub key: Secret,
    pub provider: Provider,
    pub source: String,
}

/// The credential file a check reads: the one `--env-file` names, else the
/// repository's `.env`.
#[derive(Clone, Copy)]
pub struct CredentialFile<'a> {
    pub path: &'a Path,
    pub explicit: bool,
}

impl CredentialFile<'_> {
    /// The providers whose keys this file may hold: all three in a file named
    /// with `--env-file`; only TYPESAFE_API_KEY in the repository's `.env`,
    /// where a gateway's key is usually the application's own and would bill
    /// its account.
    fn providers(self) -> &'static [Provider] {
        if self.explicit {
            &Provider::ALL
        } else {
            &Provider::ALL[..1]
        }
    }

    /// Its keys for those providers, in the order a check reads them.
    fn keys(self) -> Result<Vec<(Provider, Secret)>> {
        match read_limited(self.path)? {
            Some(text) => parse_keys(&text, self.providers()),
            None => Ok(Vec::new()),
        }
    }

    fn source(self, provider: Provider) -> String {
        let kind = if self.explicit {
            "--env-file"
        } else {
            "repository .env"
        };
        match provider {
            Provider::Typesafe => format!("{kind}: {}", self.path.display()),
            gateway => format!(
                "{kind}: {} ({})",
                self.path.display(),
                gateway.service().variable
            ),
        }
    }

    /// For the repository's `.env`, the gateway keys it holds that a check
    /// does not read, as a hint after "No API key configured".
    fn unread_hint(self) -> String {
        if self.explicit {
            return String::new();
        }
        let unread: Vec<&str> = read_limited(self.path)
            .ok()
            .flatten()
            .and_then(|text| parse_keys(&text, &Provider::ALL[1..]).ok())
            .into_iter()
            .flatten()
            .map(|(provider, _)| provider.service().variable)
            .collect();
        if unread.is_empty() {
            return String::new();
        }
        format!(
            ". The repository .env's {} is read only with --env-file .env",
            unread.join(" and ")
        )
    }
}

/// Where the key a check will use is, found without reading the saved key.
pub enum Located {
    Key(Credential),
    /// The key saved by `jevgate auth login`, of the provider recorded beside it.
    Saved(Provider),
}

impl Located {
    pub fn provider(&self) -> Provider {
        match self {
            Self::Key(credential) => credential.provider,
            Self::Saved(provider) => *provider,
        }
    }
}

/// Keys set in the environment, in the order a check reads them; an empty
/// variable counts as unset.
pub fn environment() -> Result<Vec<(Provider, String)>> {
    let mut keys = Vec::new();
    for provider in Provider::ALL {
        let name = provider.service().variable;
        match std::env::var(name) {
            Ok(value) if value.trim().is_empty() => {}
            Ok(value) => keys.push((provider, value)),
            Err(std::env::VarError::NotPresent) => {}
            Err(_) => bail!("{name} must contain UTF-8 text"),
        }
    }
    Ok(keys)
}

fn environment_source(provider: Provider) -> String {
    format!("{} environment variable", provider.service().variable)
}

/// The first key in a check's order: the environment's, then the credential
/// file's; else the saved key, whose provider `saved` gives.
pub fn locate(
    environment: Vec<(Provider, String)>,
    file: CredentialFile<'_>,
    saved: impl FnOnce() -> Provider,
) -> Result<Located> {
    if let Some((provider, value)) = environment.into_iter().next() {
        let key = Secret::parse(value)?;
        return credential(provider, key, environment_source(provider)).map(Located::Key);
    }
    if let Some((provider, key)) = file.keys()?.into_iter().next() {
        return credential(provider, key, file.source(provider)).map(Located::Key);
    }
    ensure!(
        !file.explicit,
        "Selected --env-file is missing or has no TYPESAFE_API_KEY, OPENROUTER_API_KEY or AI_GATEWAY_API_KEY; correct the path or run jevgate auth login without --env-file"
    );
    Ok(Located::Saved(saved()))
}

/// A key found for `provider`, unless its prefix shows another provider
/// issued it: sent to the wrong host, it would fail there and hand that host
/// the key.
fn credential(provider: Provider, key: Secret, source: String) -> Result<Credential> {
    refuse_foreign(provider, &key, &source)?;
    Ok(Credential {
        key,
        provider,
        source,
    })
}

/// Fail when `key` starts as another provider's keys do; `holder` names where it was found.
pub fn refuse_foreign(provider: Provider, key: &Secret, holder: &str) -> Result<()> {
    if let Some(issuer) = Provider::issuer(key.expose()).filter(|issuer| *issuer != provider) {
        let issued = issuer.service();
        bail!(
            "{holder}: the key was issued by {} (it starts with {}), not by {}; set it as {}, or save it with jevgate auth login --provider {}",
            issued.label,
            issued.key_prefix.unwrap_or_default(),
            provider.service().label,
            issued.variable,
            issuer.name()
        );
    }
    Ok(())
}

/// The provider recorded beside the saved key; TypeSafe when none is, as for
/// keys saved before 0.26.
fn recorded() -> Provider {
    store::recorded_provider().unwrap_or_default()
}

/// The provider of the key a check will use, found without reading the saved
/// key, since a check's default model depends on it; TypeSafe when no key is
/// found or its source cannot be read, which then fails where it is used.
pub fn planned_provider(path: &Path, explicit: bool) -> Provider {
    let file = CredentialFile { path, explicit };
    environment()
        .and_then(|environment| locate(environment, file, recorded))
        .map_or(Provider::Typesafe, |located| located.provider())
}

/// The key a check uses; the saved key is read only when nothing before it
/// holds one.
pub fn resolve(path: &Path, explicit: bool) -> Result<Credential> {
    let file = CredentialFile { path, explicit };
    match locate(environment()?, file, recorded)? {
        Located::Key(credential) => Ok(credential),
        Located::Saved(provider) => saved(provider, file, || {
            SavedCredentials::<NativeBackend>::native(StorageMode::configured()?)?.get()
        }),
    }
}

/// The saved key, which must be of the provider recorded beside it: a check
/// planned its model for that provider.
pub fn saved(
    recorded: Provider,
    file: CredentialFile<'_>,
    read: impl FnOnce() -> Result<Option<SavedKey>>,
) -> Result<Credential> {
    let saved = read()
        .context("Saved credential unavailable; run jevgate auth login or set TYPESAFE_API_KEY")?
        .with_context(|| {
            format!(
                "No API key configured. Run jevgate auth login, set TYPESAFE_API_KEY (or OPENROUTER_API_KEY, AI_GATEWAY_API_KEY), or provide --env-file PATH{}",
                file.unread_hint()
            )
        })?;
    ensure!(
        saved.provider == recorded,
        "The saved key is for {}, but the provider recorded beside it is {}; run jevgate auth login again",
        saved.provider.service().label,
        recorded.service().label
    );
    // A key saved before 0.26 was saved as TypeSafe's, whatever it was.
    credential(saved.provider, saved.key, saved.description)
}

/// The keys set besides the one a check uses, in the order a check reads them.
pub fn unused(path: &Path, explicit: bool) -> Result<Vec<String>> {
    let file = CredentialFile { path, explicit };
    let mut found: Vec<String> = environment()?
        .into_iter()
        .map(|(provider, _)| environment_source(provider))
        .collect();
    found.extend(
        file.keys()?
            .into_iter()
            .map(|(provider, _)| file.source(provider)),
    );
    if let Some(provider) = store::recorded_provider() {
        found.push(format!(
            "the {} key saved by jevgate auth login",
            provider.service().label
        ));
    }
    Ok(found.into_iter().skip(1).collect())
}

/// The key that a check uses instead of the saved one, when there is one:
/// an environment variable or the repository's `.env`.
pub fn override_source(path: &Path) -> Result<Option<String>> {
    let file = CredentialFile {
        path,
        explicit: false,
    };
    Ok(match locate(environment()?, file, Provider::default)? {
        Located::Key(credential) => Some(credential.source),
        Located::Saved(_) => None,
    })
}

/// The file's text, at most 64 KiB and zeroed on drop; `None` when it does not exist.
fn read_limited(path: &Path) -> Result<Option<zeroize::Zeroizing<String>>> {
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => bail!("Cannot access the credential environment file"),
    };
    ensure!(
        metadata.is_file() && metadata.len() <= 65536,
        "Credential environment file must be a regular file of at most 64 KiB"
    );
    let mut text = zeroize::Zeroizing::new(String::new());
    std::fs::File::open(path)
        .context("Cannot open credential environment file")?
        .take(65537)
        .read_to_string(&mut text)
        .map_err(|_| anyhow::anyhow!("Cannot read credential environment file as UTF-8"))?;
    ensure!(
        text.len() <= 65536,
        "Credential environment file is too large"
    );
    Ok(Some(text))
}

/// The keys a credential file defines for `providers`, in the order a check
/// reads them: each variable once, optionally exported or quoted. An empty
/// value counts as unset, as in the environment, so a template's
/// `OPENROUTER_API_KEY=` beside a real key does not fail the file.
fn parse_keys(text: &str, providers: &[Provider]) -> Result<Vec<(Provider, Secret)>> {
    let mut keys: Vec<(Provider, Secret)> = Vec::new();
    for line in text.lines() {
        let line = line.trim().strip_prefix("export ").unwrap_or(line.trim());
        let Some((name, value)) = line.split_once('=') else {
            continue;
        };
        let Some(provider) = providers
            .iter()
            .copied()
            .find(|provider| provider.service().variable == name.trim())
        else {
            continue;
        };
        let value = value.trim().trim_matches(['\'', '"']);
        if value.is_empty() {
            continue;
        }
        ensure!(
            !keys.iter().any(|(found, _)| *found == provider),
            "Credential file contains duplicate {} definitions",
            provider.service().variable
        );
        keys.push((provider, Secret::parse(value.to_owned())?));
    }
    keys.sort_by_key(|(provider, _)| Provider::ALL.iter().position(|p| p == provider));
    Ok(keys)
}
