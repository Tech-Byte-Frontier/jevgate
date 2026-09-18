use super::{
    secret::Secret,
    store::{NativeBackend, SavedCredentials, StorageMode},
};
use anyhow::{Context, Result, bail, ensure};
use std::{io::Read, path::Path};

pub struct Credential {
    pub key: Secret,
    pub source: String,
}

pub fn resolve(path: &Path, explicit: bool) -> Result<Credential> {
    let environment = environment()?;
    resolve_with(environment, path, explicit, || {
        SavedCredentials::<NativeBackend>::native(StorageMode::configured()?)?.get()
    })
}

pub fn environment() -> Result<Option<String>> {
    match std::env::var("TYPESAFE_API_KEY") {
        Ok(value) if value.trim().is_empty() => Ok(None),
        Ok(value) => Ok(Some(value)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(_) => bail!("TYPESAFE_API_KEY must contain UTF-8 text"),
    }
}

pub fn resolve_with(
    environment: Option<String>,
    path: &Path,
    explicit: bool,
    saved: impl FnOnce() -> Result<Option<(Secret, String)>>,
) -> Result<Credential> {
    if let Some(value) = environment {
        return Ok(Credential {
            key: Secret::parse(value)?,
            source: "TYPESAFE_API_KEY environment variable".into(),
        });
    }
    if let Some(key) = key_from_file(path)? {
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
    ensure!(
        !explicit,
        "Selected --env-file is missing or has no TYPESAFE_API_KEY; correct the path or run jevgate auth login without --env-file"
    );
    match saved()
        .context("Saved credential unavailable; run jevgate auth login or set TYPESAFE_API_KEY")?
    {
        Some((key, source)) => Ok(Credential { key, source }),
        None => bail!(
            "No API key configured. Run jevgate auth login, set TYPESAFE_API_KEY, or provide --env-file PATH"
        ),
    }
}

pub fn key_from_file(path: &Path) -> Result<Option<Secret>> {
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
    let mut key = None;
    for line in text.lines() {
        let line = line.trim().strip_prefix("export ").unwrap_or(line.trim());
        let Some((name, value)) = line.split_once('=') else {
            continue;
        };
        if name.trim() != "TYPESAFE_API_KEY" {
            continue;
        }
        ensure!(
            key.is_none(),
            "Credential file contains duplicate TYPESAFE_API_KEY definitions"
        );
        let value = value.trim().trim_matches(['\'', '"']);
        key = Some(Secret::parse(value.to_owned())?);
    }
    Ok(key)
}
