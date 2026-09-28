//! Where a check finds its key: TYPESAFE_API_KEY in the environment, then a
//! credential file, then the key saved by `jevgate auth login`, then a
//! gateway's variable in the environment. The first key found is used, and it
//! goes only to the provider that issued it.
use super::{
    secret::Secret,
    store::{self, NativeBackend, SavedCredentials, SavedKey, StorageMode},
};
use crate::provider::Provider;
use anyhow::{Context, Result, bail, ensure};
use std::{cell::OnceCell, io::Read, path::Path};

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

/// Where the key a check will use is, found without reading the saved key
/// where its provider is recorded.
pub enum Located {
    Key(Credential),
    /// The key saved by `jevgate auth login`, of the provider recorded beside
    /// it; a key saved before 0.26 recorded none and was TypeSafe's.
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

/// Whether a key in the environment is read before the credential file and
/// the saved key: only TypeSafe's. Other tools read OPENROUTER_API_KEY and
/// AI_GATEWAY_API_KEY too, so one exported for them is read last: it must not
/// move a check off the key it was given, to another account's credits,
/// another data processor and a model name no cached answer was asked with.
fn read_first((provider, _): &(Provider, String)) -> bool {
    *provider == Provider::Typesafe
}

/// The first key in a check's order: TYPESAFE_API_KEY in the environment,
/// the credential file's keys, the saved key, then the gateways' variables.
/// `recorded` is the provider recorded beside the saved key; `stored` reads
/// the provider of a key saved before 0.26, which recorded none, and is
/// called only when a gateway's variable would otherwise be used.
pub fn locate(
    environment: Vec<(Provider, String)>,
    file: CredentialFile<'_>,
    recorded: Option<Provider>,
    stored: impl FnOnce() -> Option<Provider>,
) -> Result<Located> {
    let (first, last): (Vec<_>, Vec<_>) = environment.into_iter().partition(read_first);
    if let Some(found) = first.into_iter().next() {
        return environment_key(found);
    }
    if let Some((provider, key)) = file.keys()?.into_iter().next() {
        return credential(provider, key, file.source(provider)).map(Located::Key);
    }
    ensure!(
        !file.explicit,
        "Selected --env-file is missing or has no TYPESAFE_API_KEY, OPENROUTER_API_KEY or AI_GATEWAY_API_KEY; correct the path or run jevgate auth login without --env-file"
    );
    let Some(gateway) = last.into_iter().next() else {
        return Ok(Located::Saved(recorded.unwrap_or_default()));
    };
    match recorded.or_else(stored) {
        Some(provider) => Ok(Located::Saved(provider)),
        None => environment_key(gateway),
    }
}

fn environment_key((provider, value): (Provider, String)) -> Result<Located> {
    let key = Secret::parse(value)?;
    credential(provider, key, environment_source(provider)).map(Located::Key)
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

/// The key saved by `jevgate auth login`, from the store
/// `JEVGATE_CREDENTIAL_STORE` names.
fn read_saved() -> Result<Option<SavedKey>> {
    SavedCredentials::<NativeBackend>::native(StorageMode::configured()?)?.get()
}

/// The provider of a key saved before 0.26, which recorded none beside it,
/// read from the credential store; none when no key is saved or the store
/// cannot be read, as in CI, where the system store needs a terminal.
fn stored_provider() -> Option<Provider> {
    read_saved().ok().flatten().map(|saved| saved.provider)
}

/// The provider of the key a check will use, since a check's default model
/// depends on it: found without reading the credential store unless a
/// gateway's variable is set and no provider is recorded. TypeSafe when no
/// key is found or its source cannot be read, which then fails where it is
/// used.
pub fn planned_provider(path: &Path, explicit: bool) -> Provider {
    let file = CredentialFile { path, explicit };
    environment()
        .and_then(|environment| {
            locate(
                environment,
                file,
                store::recorded_provider(),
                stored_provider,
            )
        })
        .map_or(Provider::Typesafe, |located| located.provider())
}

/// The key a check uses; the saved key is read only when nothing before it
/// holds one, and at most once.
pub fn resolve(path: &Path, explicit: bool) -> Result<Credential> {
    let file = CredentialFile { path, explicit };
    let read = OnceCell::new();
    let stored = || {
        read.get_or_init(read_saved)
            .as_ref()
            .ok()
            .and_then(Option::as_ref)
            .map(|saved| saved.provider)
    };
    match locate(environment()?, file, store::recorded_provider(), stored)? {
        Located::Key(credential) => Ok(credential),
        Located::Saved(provider) => saved(provider, file, || {
            read.into_inner().unwrap_or_else(read_saved)
        }),
    }
}

/// The saved key, which must be of the provider `recorded` beside it (or
/// read from it): a check planned its model for that provider.
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
    let (first, last): (Vec<_>, Vec<_>) = environment()?.into_iter().partition(read_first);
    let source = |(provider, _): (Provider, String)| environment_source(provider);
    let mut found: Vec<String> = first.into_iter().map(source).collect();
    found.extend(
        file.keys()?
            .into_iter()
            .map(|(provider, _)| file.source(provider)),
    );
    let saved =
        store::recorded_provider().or_else(|| (!last.is_empty()).then(stored_provider).flatten());
    if let Some(provider) = saved {
        found.push(format!(
            "the {} key saved by jevgate auth login",
            provider.service().label
        ));
    }
    found.extend(last.into_iter().map(source));
    Ok(found.into_iter().skip(1).collect())
}

/// The key that a check uses instead of the saved one, when there is one:
/// TYPESAFE_API_KEY in the environment or the repository's `.env`. A
/// gateway's variable is read after the saved key, so it overrides nothing.
pub fn override_source(path: &Path) -> Result<Option<String>> {
    let file = CredentialFile {
        path,
        explicit: false,
    };
    // Located as if a key were saved, so only the keys read before it count.
    let as_if_saved = Some(Provider::default());
    Ok(match locate(environment()?, file, as_if_saved, || None)? {
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
