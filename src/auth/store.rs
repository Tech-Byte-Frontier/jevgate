use super::{file, secret::Secret};
use anyhow::{Context, Result, bail, ensure};
use clap::ValueEnum;
use std::{io::IsTerminal, path::PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum StorageMode {
    Auto,
    Keyring,
    File,
}
impl StorageMode {
    pub fn configured() -> Result<Self> {
        match std::env::var("JEVGATE_CREDENTIAL_STORE").as_deref() {
            Ok("auto") | Err(std::env::VarError::NotPresent) => Ok(Self::Auto),
            Ok("keyring") => Ok(Self::Keyring),
            Ok("file") => Ok(Self::File),
            _ => bail!("JEVGATE_CREDENTIAL_STORE must be auto, keyring or file"),
        }
    }
}

pub trait Backend {
    fn get(&self) -> Result<Option<Secret>>;
    fn set(&self, secret: &Secret) -> Result<()>;
    fn delete(&self) -> Result<bool>;
}

pub struct NativeBackend;
impl NativeBackend {
    fn entry(&self) -> Result<keyring::Entry> {
        // Credential providers may open system dialogs. Only Windows' Credential
        // Manager is used through this interface without an interactive terminal.
        ensure!(
            cfg!(windows) || (std::io::stdin().is_terminal() && std::io::stderr().is_terminal()),
            "System credential operation requires a terminal; use --storage file or TYPESAFE_API_KEY for automation"
        );
        keyring::Entry::new("jevgate", "typesafe-api-key")
            .map_err(|_| anyhow::anyhow!("System credential store is unavailable or locked"))
    }
}
impl Backend for NativeBackend {
    fn get(&self) -> Result<Option<Secret>> {
        #[cfg(all(
            unix,
            not(any(target_os = "macos", target_os = "ios", target_os = "android"))
        ))]
        return super::native_unix::get();
        #[cfg(not(all(
            unix,
            not(any(target_os = "macos", target_os = "ios", target_os = "android"))
        )))]
        match self.entry()?.get_password() {
            Ok(value) => Secret::parse(value).map(Some),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => bail!(
                "Cannot read the system credential store; unlock it or run jevgate auth login"
            ),
        }
    }
    fn set(&self, secret: &Secret) -> Result<()> {
        self.entry()?
            .set_password(secret.expose())
            .map_err(|_| anyhow::anyhow!("Cannot save to the system credential store"))?;
        ensure!(
            self.get()?
                .is_some_and(|saved| saved.expose() == secret.expose()),
            "System credential store did not retain the credential"
        );
        Ok(())
    }
    fn delete(&self) -> Result<bool> {
        #[cfg(all(
            unix,
            not(any(target_os = "macos", target_os = "ios", target_os = "android"))
        ))]
        return super::native_unix::delete();
        #[cfg(not(all(
            unix,
            not(any(target_os = "macos", target_os = "ios", target_os = "android"))
        )))]
        match self.entry()?.delete_credential() {
            Ok(()) => Ok(true),
            Err(keyring::Error::NoEntry) => Ok(false),
            Err(_) => bail!(
                "Cannot remove the system credential; unlock the credential store and retry jevgate auth logout"
            ),
        }
    }
}

pub struct SavedCredentials<B> {
    pub backend: B,
    pub path: PathBuf,
    pub mode: StorageMode,
}
pub struct SavedLocation {
    pub description: String,
    pub fallback: bool,
}
pub struct Removal {
    pub file: bool,
    pub keyring: bool,
    pub keyring_error: bool,
}
impl SavedCredentials<NativeBackend> {
    pub fn native(mode: StorageMode) -> Result<Self> {
        Ok(Self {
            backend: NativeBackend,
            path: file::credential_path()?,
            mode,
        })
    }
}
impl<B: Backend> SavedCredentials<B> {
    pub fn get(&self) -> Result<Option<(Secret, String)>> {
        // A fallback written during a keyring outage must not later expose an older keyring key.
        if self.mode != StorageMode::Keyring
            && let Some(key) = file::load(&self.path)?
        {
            return Ok(Some((
                key,
                format!("protected file: {}", self.path.display()),
            )));
        }
        if self.mode == StorageMode::File {
            return Ok(None);
        }
        Ok(self
            .backend
            .get()?
            .map(|key| (key, "system credential store".into())))
    }
    pub fn save(&self, secret: &Secret) -> Result<SavedLocation> {
        if self.mode != StorageMode::File {
            match self.backend.set(secret) {
                Ok(()) => {
                    file::remove(&self.path).context("Key saved in system credential store, but an older fallback file could not be removed")?;
                    return Ok(SavedLocation {
                        description: "system credential store".into(),
                        fallback: false,
                    });
                }
                Err(error) if self.mode == StorageMode::Keyring => return Err(error),
                Err(_) => (),
            }
        }
        file::save(&self.path, secret)?;
        Ok(SavedLocation {
            description: format!("protected file: {}", self.path.display()),
            fallback: self.mode == StorageMode::Auto,
        })
    }
    pub fn remove(&self) -> Result<Removal> {
        let native = if self.mode == StorageMode::File {
            Ok(false)
        } else {
            self.backend.delete()
        };
        let removed_file = file::remove(&self.path)?;
        Ok(Removal {
            file: removed_file,
            keyring: native.as_ref().copied().unwrap_or(false),
            keyring_error: native.is_err(),
        })
    }
}
