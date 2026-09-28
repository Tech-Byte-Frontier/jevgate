mod file;
#[cfg(all(
    unix,
    not(any(target_os = "macos", target_os = "ios", target_os = "android"))
))]
mod native_unix;
mod secret;
pub(crate) mod sources;
mod store;
mod verify;

use crate::provider::{Endpoint, Provider};
use anyhow::{Result, bail, ensure};
use clap::{Args, Subcommand};
use secret::Secret;
use std::{
    io::{BufRead, IsTerminal, Write},
    path::PathBuf,
};
use store::{Backend, NativeBackend, SavedCredentials, StorageMode};
use verify::Verifier;

#[derive(Subcommand)]
pub enum AuthCommand {
    /// Validate an API key from TypeSafe, OpenRouter or Vercel AI Gateway and save it for every repository
    ///
    /// Asks which kind of key it is, prompts without echo, checks the key
    /// with its provider (no source is sent), and saves it with its provider
    /// in the OS credential store, or in an owner-only file where no store is
    /// available. Create a key at https://console.typesafe.ai/settings/keys,
    /// https://openrouter.ai/settings/keys or in the Vercel dashboard.
    Login(LoginArgs),
    /// Show which credential a check would use; exit 0 when it works, 2 otherwise
    ///
    /// Verifies the key with its provider unless --offline. No source is sent
    /// and the key is never printed.
    Status(StatusArgs),
    /// Remove saved credentials; environment variables and repository .env files are left alone
    Logout,
}

#[derive(Args)]
pub struct LoginArgs {
    /// Read one key from stdin instead of prompting, for scripts
    #[arg(long)]
    with_key: bool,
    /// The key's provider [default: asked on a terminal; typesafe with --with-key]
    #[arg(long, value_enum)]
    provider: Option<Provider>,
    /// Where to save the key [default: JEVGATE_CREDENTIAL_STORE, else auto]
    ///
    /// `auto` uses the OS credential store and, on Unix, falls back to an
    /// owner-only file (and says so). `file` writes that file directly, under
    /// JEVGATE_CONFIG_DIR when set.
    #[arg(long, value_enum)]
    storage: Option<StorageMode>,
}

#[derive(Args)]
pub struct StatusArgs {
    /// Inspect this credential file instead of the repository .env; keys in the environment still win
    #[arg(long, value_name = "FILE")]
    env_file: Option<PathBuf>,
    /// Report the credential source without contacting the provider
    #[arg(long)]
    offline: bool,
    /// Print source, provider, endpoint, configured, connection_checked, authenticated, error and unused as JSON (never the key)
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
    let (provider, key) = if args.with_key {
        ensure!(
            !std::io::stdin().is_terminal(),
            "Pipe the key into jevgate auth login --with-key, or omit --with-key for hidden terminal entry"
        );
        let provider = args.provider.unwrap_or_default();
        (provider, secret::read_stdin(std::io::stdin().lock())?)
    } else {
        prompt(args.provider)?
    };
    let service = provider.service();
    sources::refuse_foreign(provider, &key, "jevgate auth login")?;
    let endpoint = Endpoint::new(provider)?;
    let mode = args
        .storage
        .map(Ok)
        .unwrap_or_else(StorageMode::configured)?;
    let store = SavedCredentials::native(mode)?;
    note!(
        "Validating with {}; no source code is uploaded.",
        service.label
    );
    let saved = validate_and_save(&endpoint, &store, provider, &key)?;
    if saved.fallback {
        note!("System credential store unavailable; using owner-only file storage (unencrypted).");
    }
    say!(
        "{} API key verified and saved in {}.",
        service.label,
        saved.description
    );
    say!("Ready: jevgate check . --dry-run");
    report_override();
    Ok(0)
}

/// Ask on the terminal which kind of key it is, unless `--provider` said,
/// then read the key without echo.
fn prompt(chosen: Option<Provider>) -> Result<(Provider, Secret)> {
    ensure!(
        std::io::stdin().is_terminal() && std::io::stderr().is_terminal(),
        "Interactive login requires a terminal. For automation use jevgate auth login --with-key [--provider NAME] < key-file, or set TYPESAFE_API_KEY, OPENROUTER_API_KEY or AI_GATEWAY_API_KEY"
    );
    let provider = match chosen {
        Some(provider) => provider,
        None => ask_provider(&mut std::io::stdin().lock(), &mut std::io::stderr())?,
    };
    let service = provider.service();
    note!("Create an API key at {}", service.keys_page);
    let value = rpassword::prompt_password(format!("{} API key (hidden): ", service.label))
        .map_err(|_| {
            anyhow::anyhow!("Could not read hidden input; use --with-key to read from stdin")
        })?;
    Ok((provider, Secret::parse(value)?))
}

/// Answers `ask_provider` takes before it gives up.
const MAX_ANSWERS: usize = 3;

/// Ask which kind of key it is until the answer names one: its number or its
/// name, or nothing for TypeSafe.
fn ask_provider(input: &mut impl BufRead, output: &mut impl Write) -> Result<Provider> {
    let menu: Vec<String> = Provider::ALL
        .iter()
        .enumerate()
        .map(|(i, provider)| format!("{} {}", i + 1, provider.service().label))
        .collect();
    for _ in 0..MAX_ANSWERS {
        write!(output, "Key kind: {} [1]: ", menu.join(", "))?;
        output.flush()?;
        let mut answer = String::new();
        if input.read_line(&mut answer)? == 0 {
            break;
        }
        if let Some(provider) = choice(answer.trim()) {
            return Ok(provider);
        }
        writeln!(
            output,
            "Answer 1, 2 or 3, or typesafe, openrouter or vercel."
        )?;
    }
    bail!("No key kind given; pass --provider typesafe, openrouter or vercel")
}

/// The provider an answer names by number or name; empty means TypeSafe.
fn choice(answer: &str) -> Option<Provider> {
    if answer.is_empty() {
        return Some(Provider::Typesafe);
    }
    let by_number = answer
        .parse::<usize>()
        .ok()
        .and_then(|n| Provider::ALL.get(n.checked_sub(1)?).copied());
    by_number.or_else(|| Provider::named(&answer.to_ascii_lowercase()))
}

fn validate_and_save(
    verifier: &impl Verifier,
    store: &SavedCredentials<impl Backend>,
    provider: Provider,
    key: &Secret,
) -> Result<store::SavedLocation> {
    verifier.verify(key)?;
    store.save(provider, key)
}

/// What `auth status` found: where the key is, its provider and endpoint,
/// the keys set besides it, and whether it works.
struct Status {
    source: Option<String>,
    provider: Option<Provider>,
    endpoint: Option<String>,
    unused: Vec<String>,
    result: Result<()>,
    checked: bool,
}

fn status(args: StatusArgs) -> Result<u8> {
    let path = environment_path(args.env_file.as_ref())?;
    let explicit = args.env_file.is_some();
    let unused = sources::unused(&path, explicit).unwrap_or_default();
    let found = sources::resolve(&path, explicit)
        .and_then(|credential| Ok((Endpoint::new(credential.provider)?, credential)));
    let status = match found {
        Ok((endpoint, credential)) => Status {
            source: Some(credential.source),
            provider: Some(credential.provider),
            endpoint: Some(endpoint.describe()),
            unused,
            result: if args.offline {
                Ok(())
            } else {
                endpoint.verify(&credential.key)
            },
            checked: !args.offline,
        },
        Err(error) => Status {
            source: None,
            provider: None,
            endpoint: None,
            unused,
            result: Err(error),
            checked: false,
        },
    };
    let code = if status.result.is_ok() { 0 } else { 2 };
    print_status(&args, status)?;
    Ok(code)
}

fn print_status(args: &StatusArgs, status: Status) -> Result<()> {
    if args.json {
        say!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "source": status.source,
                "provider": status.provider.map(Provider::name),
                "endpoint": status.endpoint,
                "configured": status.source.is_some(),
                "connection_checked": status.checked,
                "authenticated": verify::authenticated(&status.result, status.checked),
                "error": status.result.as_ref().err().map(|e| format!("{e:#}")),
                "unused": status.unused,
            }))?
        );
        return Ok(());
    }
    if let (Some(source), Some(provider), Some(endpoint)) =
        (&status.source, status.provider, &status.endpoint)
    {
        let service = provider.service();
        say!("Credential source: {source}");
        say!(
            "Provider: {} at {endpoint}; default model {}",
            service.label,
            service.default_model
        );
    }
    if !status.unused.is_empty() {
        say!(
            "Also set, not used: {} (the first key found is used)",
            status.unused.join("; ")
        );
    }
    match status.result {
        Ok(()) if args.offline => say!("Connection: not checked (--offline)."),
        Ok(()) => say!(
            "Connection: authenticated with {}. No source code was uploaded.",
            status.provider.unwrap_or_default().service().label
        ),
        Err(error) => note!("Authentication: {error:#}"),
    }
    Ok(())
}

fn logout() -> Result<u8> {
    let store = SavedCredentials::<NativeBackend>::native(StorageMode::configured()?)?;
    let removed = store.remove()?;
    if removed.file {
        say!("Removed saved credential file: {}", store.path.display());
    }
    if removed.keyring {
        say!("Removed the saved system credential.");
    }
    if !removed.file && !removed.keyring && !removed.keyring_error {
        say!("No saved credential to remove.");
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
    match environment_path(None).and_then(|path| sources::override_source(&path)) {
        Ok(Some(source)) => say!(
            "Current override: {source}. Saved credentials are used when this override is absent."
        ),
        Err(_) => note!(
            "A local credential override could not be read. Run jevgate auth status --offline to inspect it."
        ),
        Ok(None) => (),
    }
}

#[cfg(test)]
mod tests;
