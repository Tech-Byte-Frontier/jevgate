use super::{file, secret::Secret};
use crate::provider::Provider;
use anyhow::{Context, Result, bail, ensure};
use clap::ValueEnum;
use std::{io::IsTerminal, path::PathBuf};
use zeroize::Zeroizing;

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum StorageMode {
    /// The OS credential store, else an owner-only file on Unix
    Auto,
    /// Only the OS credential store (Keychain, Credential Manager, Secret Service)
    Keyring,
    /// An owner-only file in the user configuration directory
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

/// Where the saved credential's text lives: the OS store, or a fake in tests.
pub trait Backend {
    fn get(&self) -> Result<Option<Zeroizing<String>>>;
    fn set(&self, text: &str) -> Result<()>;
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
    fn get(&self) -> Result<Option<Zeroizing<String>>> {
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
            Ok(value) => Ok(Some(Zeroizing::new(value))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => bail!(
                "Cannot read the system credential store; unlock it or run jevgate auth login"
            ),
        }
    }
    fn set(&self, text: &str) -> Result<()> {
        self.entry()?
            .set_password(text)
            .map_err(|_| anyhow::anyhow!("Cannot save to the system credential store"))?;
        ensure!(
            self.get()?.is_some_and(|saved| *saved == text),
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

/// The text a saved key is stored as: the bare key for TypeSafe, as before
/// 0.26, else the provider's name, a space and the key. Versions before 0.26
/// refuse a value with a space as an invalid key instead of sending a
/// gateway's key to TypeSafe.
fn stored_text(provider: Provider, key: &Secret) -> Zeroizing<String> {
    Zeroizing::new(match provider {
        Provider::Typesafe => key.expose().to_owned(),
        gateway => format!("{} {}", gateway.name(), key.expose()),
    })
}

/// A saved key and its provider, from the text `stored_text` wrote.
pub(super) fn saved_key(text: &str) -> Result<(Provider, Secret)> {
    let text = text.trim();
    let Some((name, key)) = text.split_once(' ') else {
        return Ok((Provider::Typesafe, Secret::parse(text.to_owned())?));
    };
    let provider = Provider::named(name)
        .context("The saved credential names an unknown provider; run jevgate auth login")?;
    Ok((provider, Secret::parse(key.to_owned())?))
}

/// A key saved by `jevgate auth login`, and where it was found.
pub struct SavedKey {
    pub provider: Provider,
    pub key: Secret,
    pub description: String,
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

/// The provider recorded beside the saved key, read without the key itself;
/// none when no key was saved with one, as before 0.26.
pub fn recorded_provider() -> Option<Provider> {
    file::credential_path()
        .ok()
        .and_then(|path| file::recorded_provider(&path))
}

impl<B: Backend> SavedCredentials<B> {
    pub fn get(&self) -> Result<Option<SavedKey>> {
        // A fallback written during a keyring outage must not later expose an older keyring key.
        if self.mode != StorageMode::Keyring
            && let Some(text) = file::load(&self.path)?
        {
            let (provider, key) = saved_key(&text)?;
            let description = format!("protected file: {}", self.path.display());
            return Ok(Some(SavedKey {
                provider,
                key,
                description,
            }));
        }
        if self.mode == StorageMode::File {
            return Ok(None);
        }
        let Some(text) = self.backend.get()? else {
            return Ok(None);
        };
        let (provider, key) = saved_key(&text)?;
        Ok(Some(SavedKey {
            provider,
            key,
            description: "system credential store".into(),
        }))
    }

    /// Save the key with its provider, then record the provider beside it.
    pub fn save(&self, provider: Provider, secret: &Secret) -> Result<SavedLocation> {
        let location = self.save_text(&stored_text(provider, secret))?;
        file::record_provider(&self.path, provider)?;
        Ok(location)
    }

    fn save_text(&self, text: &str) -> Result<SavedLocation> {
        if self.mode != StorageMode::File {
            match self.backend.set(text) {
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
        file::save(&self.path, text)?;
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
        file::forget_provider(&self.path)?;
        Ok(Removal {
            file: removed_file,
            keyring: native.as_ref().copied().unwrap_or(false),
            keyring_error: native.is_err(),
        })
    }
}
