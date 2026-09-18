mod file;
#[cfg(all(
    unix,
    not(any(target_os = "macos", target_os = "ios", target_os = "android"))
))]
mod native_unix;
mod provider;
mod secret;
pub(crate) mod sources;
mod store;

use anyhow::{Result, ensure};
use clap::{Args, Subcommand};
use provider::{TypeSafe, Verifier};
use secret::Secret;
use std::{io::IsTerminal, path::PathBuf};
use store::{Backend, NativeBackend, SavedCredentials, StorageMode};

#[derive(Subcommand)]
pub enum AuthCommand {
    /// Validate and save a TypeSafe API key for all repositories
    Login(LoginArgs),
    /// Show the active credential source and verify the connection
    Status(StatusArgs),
    /// Remove saved credentials; environment variables and repository files are preserved
    Logout,
}

#[derive(Args)]
pub struct LoginArgs {
    /// Read one key from stdin instead of opening a hidden terminal prompt
    #[arg(long)]
    with_key: bool,
    /// auto prefers the OS store, with a disclosed owner-only file fallback on Unix
    #[arg(long, value_enum)]
    storage: Option<StorageMode>,
}

#[derive(Args)]
pub struct StatusArgs {
    /// Inspect a specific credential file; TYPESAFE_API_KEY still takes precedence
    #[arg(long)]
    env_file: Option<PathBuf>,
    /// Show the source without contacting TypeSafe
    #[arg(long)]
    offline: bool,
    /// Emit machine-readable status (never the key)
    #[arg(long)]
    json: bool,
}

pub fn run(command: AuthCommand) -> Result<u8> {
    match command {
        AuthCommand::Login(args) => login(args),
        AuthCommand::Status(args) => status(args),
        AuthCommand::Logout => logout(),
    }
}

fn login(args: LoginArgs) -> Result<u8> {
    let key = if args.with_key {
        ensure!(
            !std::io::stdin().is_terminal(),
            "Pipe the key into jevgate auth login --with-key, or omit --with-key for hidden terminal entry"
        );
        secret::read_stdin(std::io::stdin().lock())?
    } else {
        ensure!(
            std::io::stdin().is_terminal() && std::io::stderr().is_terminal(),
            "Interactive login requires a terminal. For automation use jevgate auth login --with-key < key-file, or set TYPESAFE_API_KEY"
        );
        eprintln!("Create an API key at https://console.typesafe.ai/settings/keys");
        let value = rpassword::prompt_password("TypeSafe API key (hidden): ").map_err(|_| {
            anyhow::anyhow!("Could not read hidden input; use --with-key to read from stdin")
        })?;
        Secret::parse(value)?
    };
    let mode = args
        .storage
        .map(Ok)
        .unwrap_or_else(StorageMode::configured)?;
    let store = SavedCredentials::native(mode)?;
    eprintln!("Validating with TypeSafe; no source code is uploaded.");
    let saved = validate_and_save(&TypeSafe, &store, &key)?;
    if saved.fallback {
        eprintln!(
            "System credential store unavailable; using owner-only file storage (unencrypted)."
        );
    }
    println!("API key verified and saved in {}.", saved.description);
    println!("Ready: jevgate check . --dry-run");
    report_override();
    Ok(0)
}

fn validate_and_save(
    verifier: &impl Verifier,
    store: &SavedCredentials<impl Backend>,
    key: &Secret,
) -> Result<store::SavedLocation> {
    verifier.verify(key)?;
    store.save(key)
}

fn status(args: StatusArgs) -> Result<u8> {
    let path = environment_path(args.env_file.as_ref())?;
    let credential = sources::resolve(&path, args.env_file.is_some());
    let (source, result) = match credential {
        Ok(credential) => {
            let result = if args.offline {
                Ok(())
            } else {
                TypeSafe.verify(&credential.key)
            };
            (Some(credential.source), result)
        }
        Err(error) => (None, Err(error)),
    };
    let checked = !args.offline && source.is_some();
    let code = if result.is_ok() { 0 } else { 2 };
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "source":source,"configured":source.is_some(),"connection_checked":checked,
                "authenticated":provider::authenticated(&result, checked),
                "error":result.err().map(|e|format!("{e:#}")),
            }))?
        );
    } else {
        if let Some(source) = source {
            println!("Credential source: {source}");
        }
        match result {
            Ok(()) => println!(
                "{}",
                if args.offline {
                    "Connection: not checked (--offline)."
                } else {
                    "Connection: authenticated with TypeSafe. No source code was uploaded."
                }
            ),
            Err(error) => eprintln!("Authentication: {error:#}"),
        }
    }
    Ok(code)
}

fn logout() -> Result<u8> {
    let store = SavedCredentials::<NativeBackend>::native(StorageMode::configured()?)?;
    let removed = store.remove()?;
    if removed.file {
        println!("Removed saved credential file: {}", store.path.display());
    }
    if removed.keyring {
        println!("Removed the saved system credential.");
    }
    if !removed.file && !removed.keyring && !removed.keyring_error {
        println!("No saved credential to remove.");
    }
    report_override();
    ensure!(
        !removed.keyring_error,
        "System credential removal could not be confirmed; unlock the store and retry jevgate auth logout. A previously stored system credential may remain"
    );
    Ok(0)
}

fn environment_path(selected: Option<&PathBuf>) -> Result<PathBuf> {
    let invocation = std::env::current_dir()?.canonicalize()?;
    Ok(match selected {
        Some(path) => invocation.join(path),
        None => crate::config::repository_root(&invocation).join(".env"),
    })
}

fn report_override() {
    let override_source = (|| -> Result<Option<String>> {
        if sources::environment()?.is_some() {
            return Ok(Some("TYPESAFE_API_KEY environment variable".into()));
        }
        let path = environment_path(None)?;
        Ok(sources::key_from_file(&path)?.map(|_| format!("repository .env: {}", path.display())))
    })();
    match override_source {
        Ok(Some(source)) => println!(
            "Current override: {source}. Saved credentials are used when this override is absent."
        ),
        Err(_) => eprintln!(
            "A local credential override could not be read. Run jevgate auth status --offline to inspect it."
        ),
        Ok(None) => (),
    }
}

#[cfg(test)]
mod tests;
