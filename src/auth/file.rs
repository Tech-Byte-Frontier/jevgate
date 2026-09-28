//! The fallback is confined to an owner-only directory and never writes a
//! repository .env. Beside it, a plain file names the saved key's provider.
use super::secret::MAX_STORED_BYTES;
use crate::provider::Provider;
use anyhow::{Context, Result, ensure};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};
use zeroize::Zeroizing;

pub fn credential_path() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("JEVGATE_CONFIG_DIR") {
        let path = PathBuf::from(path);
        ensure!(
            path.is_absolute(),
            "JEVGATE_CONFIG_DIR must be an absolute directory"
        );
        return Ok(path.join("credentials"));
    }
    config_base()
}

fn required_env(key: &str) -> Result<std::ffi::OsString> {
    std::env::var_os(key).with_context(|| format!("{key} is unavailable; set JEVGATE_CONFIG_DIR"))
}

fn config_base() -> Result<PathBuf> {
    #[cfg(target_os = "windows")]
    let base = PathBuf::from(required_env("APPDATA")?);
    #[cfg(target_os = "macos")]
    let base = PathBuf::from(required_env("HOME")?).join("Library/Application Support");
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let base = match std::env::var_os("XDG_CONFIG_HOME").filter(|p| !p.is_empty()) {
        Some(path) => {
            let path = PathBuf::from(path);
            ensure!(path.is_absolute(), "XDG_CONFIG_HOME must be absolute");
            path
        }
        None => PathBuf::from(required_env("HOME")?).join(".config"),
    };
    ensure!(
        base.is_absolute(),
        "User configuration directory must be absolute"
    );
    Ok(base.join("jevgate/credentials"))
}

#[cfg(unix)]
fn private_metadata(path: &Path, directory: bool) -> Result<fs::Metadata> {
    use std::os::unix::fs::MetadataExt;
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        !metadata.file_type().is_symlink()
            && if directory {
                metadata.is_dir()
            } else {
                metadata.is_file()
            },
        "Credential storage must use real directories and regular files"
    );
    // geteuid is a side-effect-free process identity query.
    ensure!(
        metadata.uid() == unsafe { libc::geteuid() } && metadata.mode() & 0o077 == 0,
        "Credential storage must be owned by the current user with directory mode 0700 and file mode 0600"
    );
    ensure!(
        directory || metadata.nlink() == 1,
        "Credential file must not have hard links"
    );
    Ok(metadata)
}

#[cfg(not(unix))]
fn private_metadata(_path: &Path, _directory: bool) -> Result<fs::Metadata> {
    anyhow::bail!(
        "Protected-file credentials are unsupported on this platform; use the system credential store or TYPESAFE_API_KEY"
    )
}

/// The saved credential's text, as `store` wrote it.
pub fn load(path: &Path) -> Result<Option<Zeroizing<String>>> {
    if !path.try_exists()? && !path.is_symlink() {
        return Ok(None);
    }
    private_metadata(path.parent().context("Missing credential directory")?, true)?;
    let metadata = private_metadata(path, false)?;
    ensure!(
        metadata.len() <= MAX_STORED_BYTES as u64,
        "Saved credential file is too large"
    );
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let mut value = Zeroizing::new(String::new());
    options
        .open(path)?
        .take((MAX_STORED_BYTES + 1) as u64)
        .read_to_string(&mut value)
        .map_err(|_| anyhow::anyhow!("Cannot read saved credential; run jevgate auth login"))?;
    ensure!(
        value.len() <= MAX_STORED_BYTES,
        "Saved credential file is too large"
    );
    Ok(Some(value))
}

pub fn save(path: &Path, text: &str) -> Result<()> {
    #[cfg(not(unix))]
    {
        let _ = (path, text);
        anyhow::bail!(
            "Protected-file storage is unavailable; use the system credential store or TYPESAFE_API_KEY"
        );
    }
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let parent = path.parent().context("Missing credential directory")?;
        private_directory(parent)?;
        if path.exists() || path.is_symlink() {
            private_metadata(path, false)?;
        }
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos();
        let temporary = parent.join(format!(".credential-{}-{nonce}.tmp", std::process::id()));
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)?;
        let result = (|| -> Result<()> {
            file.write_all(text.as_bytes())?;
            file.sync_all()?;
            fs::rename(&temporary, path)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(temporary);
        }
        result.context("Could not save protected credential file")
    }
}

pub fn remove(path: &Path) -> Result<bool> {
    if !path.try_exists()? && !path.is_symlink() {
        return Ok(false);
    }
    private_metadata(path.parent().context("Missing credential directory")?, true)?;
    private_metadata(path, false)?;
    fs::remove_file(path)?;
    Ok(true)
}

/// The directory of saved credentials, created owner-only.
fn private_directory(directory: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(directory)?;
        private_metadata(directory, true)?;
    }
    #[cfg(not(unix))]
    fs::create_dir_all(directory)?;
    Ok(())
}

/// The file beside the saved credential that names its provider, so a check
/// can choose its model before it reads the credential itself.
fn provider_path(credential: &Path) -> PathBuf {
    credential.with_file_name("provider")
}

/// The most of that file read: a provider's name is a short word.
const MAX_PROVIDER_RECORD_BYTES: u64 = 64;

/// The provider recorded beside the saved credential; none when no key was
/// saved with one, as before 0.26.
pub fn recorded_provider(credential: &Path) -> Option<Provider> {
    let path = provider_path(credential);
    if path.is_symlink() {
        return None;
    }
    let mut name = String::new();
    fs::File::open(path)
        .ok()?
        .take(MAX_PROVIDER_RECORD_BYTES)
        .read_to_string(&mut name)
        .ok()?;
    Provider::named(name.trim())
}

pub fn record_provider(credential: &Path, provider: Provider) -> Result<()> {
    let path = provider_path(credential);
    private_directory(path.parent().context("Missing credential directory")?)?;
    ensure!(
        !path.is_symlink(),
        "The saved key's provider record must be a regular file"
    );
    fs::write(&path, provider.name()).context("Could not record the saved key's provider")
}

pub fn forget_provider(credential: &Path) -> Result<()> {
    match fs::remove_file(provider_path(credential)) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error.into()),
        _ => Ok(()),
    }
}
