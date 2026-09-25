use super::{
    secret::Secret,
    store::{NativeBackend, SavedCredentials, StorageMode},
};
use crate::options::Provider;
use anyhow::{Context, Result, bail, ensure};
use std::{io::Read, path::Path};

pub struct Credential {
    pub key: Secret,
    pub source: String,
}

pub fn resolve(path: &Path, explicit: bool) -> Result<Credential> {
    resolve_for(Provider::TypeSafe, path, explicit)
}

pub fn resolve_for(provider: Provider, path: &Path, explicit: bool) -> Result<Credential> {
    let environment = environment_for(provider)?;
    if provider == Provider::TypeSafe {
        resolve_with(environment, path, explicit, || {
            SavedCredentials::<NativeBackend>::native(StorageMode::configured()?)?.get()
        })
    } else {
        resolve_with_named(environment, path, explicit, provider, || Ok(None))
    }
}

pub fn environment() -> Result<Option<String>> {
    environment_for(Provider::TypeSafe)
}

fn environment_for(provider: Provider) -> Result<Option<String>> {
    let name = provider.api_key_env();
    match std::env::var(name) {
        Ok(value) if value.trim().is_empty() => Ok(None),
        Ok(value) => Ok(Some(value)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(_) => bail!("{name} must contain UTF-8 text"),
    }
}

pub fn resolve_with(
    environment: Option<String>,
    path: &Path,
    explicit: bool,
    saved: impl FnOnce() -> Result<Option<(Secret, String)>>,
) -> Result<Credential> {
    resolve_with_named(environment, path, explicit, Provider::TypeSafe, saved)
}

pub(super) fn resolve_with_named(
    environment: Option<String>,
    path: &Path,
    explicit: bool,
    provider: Provider,
    saved: impl FnOnce() -> Result<Option<(Secret, String)>>,
) -> Result<Credential> {
    let key_name = provider.api_key_env();
    if let Some(value) = environment {
        return Ok(Credential {
            key: Secret::parse(value)?,
            source: format!("{key_name} environment variable"),
        });
    }
    if let Some(key) = key_from_file_for(provider, path)? {
        return Ok(Credential {
            key,
            source: format!(
                "{}: {}",
                if explicit {
                    "--env-file"
                } else {
                    "repository .env"
                },
                path.display()
            ),
        });
    }
    if explicit {
        let login = if key_name == "TYPESAFE_API_KEY" {
            " or run jevgate auth login without --env-file"
        } else {
            ""
        };
        bail!("Selected --env-file is missing or has no {key_name}; correct the path{login}");
    }
    match saved()
        .context("Saved credential unavailable; run jevgate auth login or set TYPESAFE_API_KEY")?
    {
        Some((key, source)) => Ok(Credential { key, source }),
        None if key_name == "TYPESAFE_API_KEY" => bail!(
            "No API key configured. Run jevgate auth login, set TYPESAFE_API_KEY, or provide --env-file PATH"
        ),
        None => bail!(
            "No OpenRouter API key configured. Set OPENROUTER_API_KEY or provide --env-file PATH with an OPENROUTER_API_KEY entry"
        ),
    }
}

pub fn key_from_file(path: &Path) -> Result<Option<Secret>> {
    key_from_file_named(path, "TYPESAFE_API_KEY")
}

fn key_from_file_for(provider: Provider, path: &Path) -> Result<Option<Secret>> {
    key_from_file_named(path, provider.api_key_env())
}

fn key_from_file_named(path: &Path, key_name: &str) -> Result<Option<Secret>> {
    match read_limited(path)? {
        Some(text) => parse_key(&text, key_name),
        None => Ok(None),
    }
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

/// The single provider-key definition, optionally exported or quoted.
fn parse_key(text: &str, key_name: &str) -> Result<Option<Secret>> {
    let mut key = None;
    for line in text.lines() {
        let line = line.trim().strip_prefix("export ").unwrap_or(line.trim());
        let Some((name, value)) = line.split_once('=') else {
            continue;
        };
        if name.trim() != key_name {
            continue;
        }
        ensure!(
            key.is_none(),
            "Credential file contains duplicate {key_name} definitions"
        );
        let value = value.trim().trim_matches(['\'', '"']);
        key = Some(Secret::parse(value.to_owned())?);
    }
    Ok(key)
}
