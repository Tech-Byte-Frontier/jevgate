//! Saving, finding and checking keys: the credential store and its file
//! fallback, the order a check reads keys in, each provider's key check, and
//! the login question.
use super::*;
use crate::provider::{OPENROUTER, TYPESAFE, VERCEL};
use sources::{CredentialFile, Located};
use std::{
    cell::{Cell, RefCell},
    path::Path,
};
use store::SavedKey;
use zeroize::Zeroizing;

#[derive(Default)]
struct FakeStore {
    secret: RefCell<Option<String>>,
    unavailable: Cell<bool>,
    writes: Cell<usize>,
}
impl Backend for FakeStore {
    fn get(&self) -> Result<Option<Zeroizing<String>>> {
        ensure!(!self.unavailable.get(), "unavailable");
        Ok(self.secret.borrow().clone().map(Zeroizing::new))
    }
    fn set(&self, text: &str) -> Result<()> {
        ensure!(!self.unavailable.get(), "unavailable");
        self.writes.set(self.writes.get() + 1);
        *self.secret.borrow_mut() = Some(text.into());
        Ok(())
    }
    fn delete(&self) -> Result<bool> {
        ensure!(!self.unavailable.get(), "unavailable");
        Ok(self.secret.borrow_mut().take().is_some())
    }
}
struct Verification(bool);
impl Verifier for Verification {
    fn verify(&self, _key: &Secret) -> Result<()> {
        ensure!(self.0, "Rejected key");
        Ok(())
    }
}
fn store(root: &Path, mode: StorageMode) -> SavedCredentials<FakeStore> {
    SavedCredentials {
        backend: FakeStore::default(),
        path: root.join("user-config/credentials"),
        mode,
    }
}

fn key(value: &str) -> Secret {
    Secret::parse(value.into()).unwrap()
}

/// The saved key's value.
fn saved_value(saved: &SavedCredentials<FakeStore>) -> String {
    saved.get().unwrap().unwrap().key.expose().to_owned()
}

/// A store that already holds `old-key`, and the `new-key` meant to replace it.
fn replacing_old_key(root: &Path) -> (SavedCredentials<FakeStore>, Secret) {
    let saved = store(root, StorageMode::Auto);
    *saved.backend.secret.borrow_mut() = Some("old-key".into());
    (saved, key("new-key"))
}

#[test]
fn validation_failure_preserves_the_previous_credential() {
    let project = crate::tests::Project::new();
    let (saved, key) = replacing_old_key(&project.0);
    let provider = Provider::Typesafe;
    assert!(validate_and_save(&Verification(false), &saved, provider, &key).is_err());
    assert_eq!(saved.backend.writes.get(), 0);
    assert_eq!(saved_value(&saved), "old-key");
    assert!(!saved.path.exists());
}

#[test]
fn successful_login_and_logout_use_the_system_store() {
    let project = crate::tests::Project::new();
    let saved = store(&project.0, StorageMode::Auto);
    let provider = Provider::Typesafe;
    let location = validate_and_save(&Verification(true), &saved, provider, &key("new-key"));
    let location = location.unwrap();
    assert_eq!(location.description, "system credential store");
    assert!(!location.fallback && !saved.path.exists());
    assert_eq!(saved_value(&saved), "new-key");
    let removed = saved.remove().unwrap();
    assert!(removed.keyring && !removed.keyring_error);
    assert!(saved.get().unwrap().is_none());
}

#[test]
fn a_gateway_key_is_saved_with_its_provider_where_older_versions_refuse_it() {
    let project = crate::tests::Project::new();
    let saved = store(&project.0, StorageMode::Auto);
    let gateway_key = key("sk-or-v1-private");
    saved.save(Provider::Openrouter, &gateway_key).unwrap();
    let text = saved.backend.secret.borrow().clone().unwrap();
    assert_eq!(text, "openrouter sk-or-v1-private");
    assert!(
        Secret::parse(text).is_err(),
        "0.25 parses the stored value as a bare key"
    );
    let found = saved.get().unwrap().unwrap();
    assert_eq!(
        (found.provider, found.key.expose()),
        (Provider::Openrouter, "sk-or-v1-private")
    );
    assert_eq!(
        file::recorded_provider(&saved.path),
        Some(Provider::Openrouter)
    );
    saved
        .save(Provider::Typesafe, &key("typesafe-key"))
        .unwrap();
    assert_eq!(
        saved.backend.secret.borrow().as_deref(),
        Some("typesafe-key")
    );
    assert_eq!(
        file::recorded_provider(&saved.path),
        Some(Provider::Typesafe)
    );
    saved.remove().unwrap();
    assert_eq!(file::recorded_provider(&saved.path), None);
    for invalid in ["gateway sk-or-v1-x", "openrouter two words"] {
        assert!(store::saved_key(invalid).is_err(), "{invalid}");
    }
}

#[cfg(unix)]
#[test]
fn fallback_is_private_survives_store_recovery_and_can_migrate_back() {
    use std::os::unix::fs::PermissionsExt;
    let project = crate::tests::Project::new();
    let (saved, key) = replacing_old_key(&project.0);
    saved.backend.unavailable.set(true);
    let location = validate_and_save(&Verification(true), &saved, Provider::Typesafe, &key);
    assert!(location.unwrap().fallback);
    assert_eq!(
        std::fs::metadata(&saved.path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        std::fs::metadata(saved.path.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    saved.backend.unavailable.set(false);
    assert_eq!(saved_value(&saved), "new-key");
    saved.save(Provider::Typesafe, &key).unwrap();
    assert!(!saved.path.exists());
    assert_eq!(*saved.backend.get().unwrap().unwrap(), "new-key");
}

#[cfg(unix)]
#[test]
fn keyring_only_mode_never_falls_back_and_partial_logout_is_reported() {
    let project = crate::tests::Project::new();
    let mut saved = store(&project.0, StorageMode::Keyring);
    saved.backend.unavailable.set(true);
    let key = key("test-key");
    assert!(saved.save(Provider::Typesafe, &key).is_err());
    assert!(!saved.path.exists());
    saved.mode = StorageMode::File;
    saved.save(Provider::Typesafe, &key).unwrap();
    saved.mode = StorageMode::Auto;
    let removed = saved.remove().unwrap();
    assert!(removed.file && removed.keyring_error);
    assert!(!saved.path.exists());
}

/// What `jevgate auth login` saved, as a check finds it before reading the
/// key: the provider recorded beside it, and the provider a key saved before
/// 0.26, which recorded none, turns out to have when the store is read.
#[derive(Clone, Copy, Default)]
struct Saved {
    recorded: Option<Provider>,
    stored: Option<Provider>,
}

/// The first key a check finds with `environment` set, `file` as its
/// credential file and `saved` saved; `read` notes whether the credential
/// store was read.
fn located_with(
    environment: &[(Provider, &str)],
    file: CredentialFile<'_>,
    saved: Saved,
    read: &Cell<bool>,
) -> Result<Located> {
    let environment = environment
        .iter()
        .map(|(provider, value)| (*provider, value.to_string()))
        .collect();
    sources::locate(environment, file, saved.recorded, || {
        read.set(true);
        saved.stored
    })
}

/// The first key a check finds when no key is saved.
fn located(environment: &[(Provider, &str)], file: CredentialFile<'_>) -> Result<Located> {
    located_with(environment, file, Saved::default(), &Cell::new(false))
}

/// The found key's provider, value and source.
fn found(located: Result<Located>) -> (Provider, String, String) {
    match located.unwrap() {
        Located::Key(credential) => (
            credential.provider,
            credential.key.expose().to_owned(),
            credential.source,
        ),
        Located::Saved(provider) => (provider, String::new(), "saved".into()),
    }
}

#[test]
fn a_gateways_variable_is_read_after_every_key_given_to_jevgate() {
    let project = crate::tests::Project::new();
    let path = project.0.join(".env");
    let repository = CredentialFile {
        path: &path,
        explicit: false,
    };
    let selected = CredentialFile {
        path: &path,
        explicit: true,
    };
    let gateways = [
        (Provider::Openrouter, "sk-or-environment"),
        (Provider::Vercel, "vck_environment"),
    ];
    project.write(
        ".env",
        "OPENROUTER_API_KEY=sk-or-file\nTYPESAFE_API_KEY=file-key\n",
    );
    let everything = [(Provider::Typesafe, "environment-key"), gateways[0]];
    let (provider, value, source) = found(located(&everything, selected));
    assert_eq!(
        (provider, value.as_str(), source.as_str()),
        (
            Provider::Typesafe,
            "environment-key",
            "TYPESAFE_API_KEY environment variable"
        )
    );
    for file in [repository, selected] {
        let (provider, value, _) = found(located(&gateways, file));
        assert_eq!(
            (provider, value.as_str()),
            (Provider::Typesafe, "file-key"),
            "the credential file's TypeSafe key comes before a gateway's variable"
        );
    }
    project.write(
        ".env",
        "TYPESAFE_API_KEY=\nOPENROUTER_API_KEY=sk-or-file\nAI_GATEWAY_API_KEY=''\nUNRELATED=keep-me\n",
    );
    let (provider, value, source) = found(located(&gateways[1..], selected));
    assert_eq!(
        (provider, value.as_str()),
        (Provider::Openrouter, "sk-or-file")
    );
    assert!(source.starts_with("--env-file:") && source.ends_with("(OPENROUTER_API_KEY)"));
    let (provider, value, source) = found(located(&gateways, repository));
    assert_eq!(
        (provider, value.as_str(), source.as_str()),
        (
            Provider::Openrouter,
            "sk-or-environment",
            "OPENROUTER_API_KEY environment variable"
        ),
        "the repository .env is read only for TYPESAFE_API_KEY, and nothing is saved"
    );
    project.write(".env", "UNRELATED=keep-me\n");
    assert!(
        located(&gateways, selected).is_err(),
        "a selected file must hold a key"
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "UNRELATED=keep-me\n"
    );
}

#[test]
fn the_saved_key_comes_before_a_gateways_variable_and_the_store_is_read_only_to_decide_that() {
    let project = crate::tests::Project::new();
    let path = project.0.join("absent.env");
    let file = CredentialFile {
        path: &path,
        explicit: false,
    };
    let gateway = [(Provider::Openrouter, "sk-or-environment")];
    let recorded = Saved {
        recorded: Some(Provider::Vercel),
        stored: None,
    };
    let before_0_26 = Saved {
        recorded: None,
        stored: Some(Provider::Typesafe),
    };
    for (saved, reads_the_store) in [(recorded, false), (before_0_26, true)] {
        let read = Cell::new(false);
        let (provider, _, source) = found(located_with(&gateway, file, saved, &read));
        assert_eq!(source, "saved");
        assert_eq!(Some(provider), saved.recorded.or(saved.stored));
        assert_eq!(read.get(), reads_the_store);
    }
    let read = Cell::new(false);
    let (provider, _, source) = found(located_with(&[], file, before_0_26, &read));
    assert_eq!(
        (provider, source.as_str(), read.get()),
        (Provider::Typesafe, "saved", false),
        "without a gateway's variable, a key saved before 0.26 is TypeSafe's, as it was then"
    );
}

#[test]
fn a_key_issued_by_another_provider_is_refused_without_being_shown() {
    let project = crate::tests::Project::new();
    let path = project.0.join("absent.env");
    let file = CredentialFile {
        path: &path,
        explicit: false,
    };
    for (provider, value, variable) in [
        (Provider::Typesafe, "sk-or-v1-private", "OPENROUTER_API_KEY"),
        (Provider::Openrouter, "vck_private", "AI_GATEWAY_API_KEY"),
    ] {
        let error = located(&[(provider, value)], file)
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains(variable), "{error}");
        assert!(!error.contains("private"), "{error}");
    }
    let (provider, _, _) = found(located(&[(Provider::Vercel, "vck_ok")], file));
    assert_eq!(provider, Provider::Vercel);
}

#[test]
fn the_saved_key_must_be_of_the_provider_recorded_beside_it() {
    let project = crate::tests::Project::new();
    project.write(".env", "AI_GATEWAY_API_KEY=vck_app\n");
    let path = project.0.join(".env");
    let file = CredentialFile {
        path: &path,
        explicit: false,
    };
    let openrouter = || {
        Ok(Some(SavedKey {
            provider: Provider::Openrouter,
            key: key("sk-or-saved"),
            description: "system credential store".into(),
        }))
    };
    let credential = sources::saved(Provider::Openrouter, file, openrouter).unwrap();
    assert_eq!(credential.key.expose(), "sk-or-saved");
    let error = sources::saved(Provider::Typesafe, file, openrouter)
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains("run jevgate auth login again"), "{error}");
    let missing = sources::saved(Provider::Typesafe, file, || Ok(None))
        .err()
        .unwrap()
        .to_string();
    assert!(missing.starts_with("No API key configured"), "{missing}");
    assert!(
        missing.contains("AI_GATEWAY_API_KEY is read only with --env-file .env"),
        "{missing}"
    );
    let saved_by_0_25 = || {
        Ok(Some(SavedKey {
            provider: Provider::Typesafe,
            key: key("sk-or-v1-private"),
            description: "system credential store".into(),
        }))
    };
    let refused = sources::saved(Provider::Typesafe, file, saved_by_0_25)
        .err()
        .unwrap()
        .to_string();
    assert!(refused.contains("OPENROUTER_API_KEY") && !refused.contains("private"));
}

#[test]
fn invalid_or_duplicate_keys_never_fall_through_or_appear_in_errors() {
    let project = crate::tests::Project::new();
    project.write(
        ".env",
        "TYPESAFE_API_KEY=private-key\nTYPESAFE_API_KEY=another-private-key\n",
    );
    let path = project.0.join(".env");
    let file = CredentialFile {
        path: &path,
        explicit: false,
    };
    let error = located(&[], file).err().unwrap().to_string();
    assert!(!error.contains("private-key"));
    for value in ["", "private\nkey", "private key", "private\u{7f}key"] {
        assert!(Secret::parse(value.into()).is_err());
    }
    let error = located(&[(Provider::Typesafe, "bad key")], file)
        .err()
        .unwrap()
        .to_string();
    assert!(!error.contains("bad key"));
}

#[test]
fn credential_parser_does_not_execute_shell() {
    let project = crate::tests::Project::new();
    project.write(
        ".env",
        "export TYPESAFE_API_KEY='literal$(do-not-execute)'\n",
    );
    let path = project.0.join(".env");
    let file = CredentialFile {
        path: &path,
        explicit: false,
    };
    assert_eq!(found(located(&[], file)).1, "literal$(do-not-execute)");
}

#[test]
fn stdin_supports_one_key_with_a_trailing_newline_and_rejects_unbounded_input() {
    assert_eq!(
        secret::read_stdin(&b"stdin-key\n"[..]).unwrap().expose(),
        "stdin-key"
    );
    assert!(secret::read_stdin(&b"first\nsecond"[..]).is_err());
    assert!(secret::read_stdin(&vec![b'x'; secret::MAX_KEY_BYTES + 1][..]).is_err());
}

#[test]
fn login_asks_for_the_kind_of_key_by_number_or_name() {
    let ask = |answers: &str| {
        let mut output = Vec::new();
        let chosen = ask_provider(&mut answers.as_bytes(), &mut output);
        (chosen.ok(), String::from_utf8(output).unwrap())
    };
    let (chosen, prompt) = ask("\n");
    assert_eq!(chosen, Some(Provider::Typesafe));
    assert_eq!(
        prompt,
        "Key kind: 1 TypeSafe, 2 OpenRouter, 3 Vercel AI Gateway [1]: "
    );
    assert_eq!(ask("2\n").0, Some(Provider::Openrouter));
    assert_eq!(ask(" Vercel \n").0, Some(Provider::Vercel));
    let (chosen, prompt) = ask("4\nopenrouter\n");
    assert_eq!(chosen, Some(Provider::Openrouter));
    assert!(prompt.contains("Answer 1, 2 or 3"));
    assert_eq!(ask("0\nx\ny\n").0, None, "three wrong answers");
    assert_eq!(ask("").0, None, "no answer");
}

#[cfg(unix)]
#[test]
fn fallback_rejects_symlinks_hardlinks_and_broad_permissions() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let project = crate::tests::Project::new();
    let saved = store(&project.0, StorageMode::File);
    let key = key("test-key");
    saved.save(Provider::Typesafe, &key).unwrap();
    let another = project.0.join("linked-secret");
    std::fs::hard_link(&saved.path, &another).unwrap();
    assert!(saved.get().is_err());
    assert!(saved.save(Provider::Typesafe, &key).is_err());
    std::fs::remove_file(another).unwrap();
    std::fs::set_permissions(&saved.path, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(saved.get().is_err());
    std::fs::remove_file(&saved.path).unwrap();
    project.write("external", "do-not-touch");
    symlink(project.0.join("external"), &saved.path).unwrap();
    assert!(saved.save(Provider::Typesafe, &key).is_err());
    assert!(saved.remove().is_err());
    assert_eq!(
        std::fs::read_to_string(project.0.join("external")).unwrap(),
        "do-not-touch"
    );
}

#[test]
fn authentication_is_known_only_after_a_checked_connection() {
    assert_eq!(verify::authenticated(&Ok(()), false), None);
    assert_eq!(verify::authenticated(&Ok(()), true), Some(true));
    assert_eq!(
        verify::authenticated(&verify::http_error(&TYPESAFE, 403), true),
        Some(false)
    );
    assert_eq!(
        verify::authenticated(&verify::http_error(&TYPESAFE, 429), true),
        None
    );
}

#[test]
fn each_key_check_answer_is_validated_without_echoing_provider_text() {
    use crate::provider::KeyAnswer;
    for (service, valid) in [
        (
            &TYPESAFE,
            serde_json::json!({"models": [{"name": "jev-latest"}]}),
        ),
        (
            &OPENROUTER,
            serde_json::json!({"data": {"label": "k", "limit": null}}),
        ),
        (
            &VERCEL,
            serde_json::json!({"balance": "95.50", "total_used": "4.50"}),
        ),
    ] {
        let answer = service.key_check.answer;
        assert!(verify::valid_answer(service, answer, &valid).is_ok());
        let error = verify::valid_answer(
            service,
            answer,
            &serde_json::json!({"error": "secret-do-not-echo"}),
        )
        .unwrap_err()
        .to_string();
        assert!(!error.contains("secret-do-not-echo"));
        assert!(error.starts_with(service.label));
    }
    assert_eq!(OPENROUTER.key_check.answer, KeyAnswer::Key);
}

#[test]
fn http_errors_explain_rejection_and_retry() {
    let rejected = verify::http_error(&OPENROUTER, 401)
        .unwrap_err()
        .to_string();
    assert!(rejected.starts_with("OpenRouter rejected this API key"));
    assert!(rejected.contains(OPENROUTER.keys_page));
    assert!(
        verify::http_error(&TYPESAFE, 429)
            .unwrap_err()
            .to_string()
            .contains("not retried")
    );
}
