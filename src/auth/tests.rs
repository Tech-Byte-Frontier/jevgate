use super::*;
use std::{
    cell::{Cell, RefCell},
    path::Path,
};

#[derive(Default)]
struct FakeStore {
    secret: RefCell<Option<String>>,
    unavailable: Cell<bool>,
    writes: Cell<usize>,
}
impl Backend for FakeStore {
    fn get(&self) -> Result<Option<Secret>> {
        ensure!(!self.unavailable.get(), "unavailable");
        self.secret.borrow().clone().map(Secret::parse).transpose()
    }
    fn set(&self, key: &Secret) -> Result<()> {
        ensure!(!self.unavailable.get(), "unavailable");
        self.writes.set(self.writes.get() + 1);
        *self.secret.borrow_mut() = Some(key.expose().into());
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

/// A store that already holds `old-key`, and the `new-key` meant to replace it.
fn replacing_old_key(root: &Path) -> (SavedCredentials<FakeStore>, Secret) {
    let saved = store(root, StorageMode::Auto);
    *saved.backend.secret.borrow_mut() = Some("old-key".into());
    (saved, Secret::parse("new-key".into()).unwrap())
}

#[test]
fn validation_failure_preserves_the_previous_credential() {
    let project = crate::tests::Project::new();
    let (saved, key) = replacing_old_key(&project.0);
    assert!(validate_and_save(&Verification(false), &saved, &key).is_err());
    assert_eq!(saved.backend.writes.get(), 0);
    assert_eq!(saved.get().unwrap().unwrap().0.expose(), "old-key");
    assert!(!saved.path.exists());
}

#[test]
fn successful_login_and_logout_use_the_system_store() {
    let project = crate::tests::Project::new();
    let saved = store(&project.0, StorageMode::Auto);
    let key = Secret::parse("new-key".into()).unwrap();
    let location = validate_and_save(&Verification(true), &saved, &key).unwrap();
    assert_eq!(location.description, "system credential store");
    assert!(!location.fallback && !saved.path.exists());
    assert_eq!(saved.get().unwrap().unwrap().0.expose(), "new-key");
    let removed = saved.remove().unwrap();
    assert!(removed.keyring && !removed.keyring_error);
    assert!(saved.get().unwrap().is_none());
}

#[cfg(unix)]
#[test]
fn fallback_is_private_survives_store_recovery_and_can_migrate_back() {
    use std::os::unix::fs::PermissionsExt;
    let project = crate::tests::Project::new();
    let (saved, key) = replacing_old_key(&project.0);
    saved.backend.unavailable.set(true);
    let location = validate_and_save(&Verification(true), &saved, &key).unwrap();
    assert!(location.fallback);
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
    assert_eq!(saved.get().unwrap().unwrap().0.expose(), "new-key");
    saved.save(&key).unwrap();
    assert!(!saved.path.exists());
    assert_eq!(saved.backend.get().unwrap().unwrap().expose(), "new-key");
}

#[cfg(unix)]
#[test]
fn keyring_only_mode_never_falls_back_and_partial_logout_is_reported() {
    let project = crate::tests::Project::new();
    let mut saved = store(&project.0, StorageMode::Keyring);
    saved.backend.unavailable.set(true);
    let key = Secret::parse("test-key".into()).unwrap();
    assert!(saved.save(&key).is_err());
    assert!(!saved.path.exists());
    saved.mode = StorageMode::File;
    saved.save(&key).unwrap();
    saved.mode = StorageMode::Auto;
    let removed = saved.remove().unwrap();
    assert!(removed.file && removed.keyring_error);
    assert!(!saved.path.exists());
}

#[test]
fn precedence_is_environment_then_selected_file_then_saved_credentials() {
    let project = crate::tests::Project::new();
    let file = project.0.join(".env");
    project.write(".env", "TYPESAFE_API_KEY=repo-key\n");
    let env = sources::resolve_with(Some("environment-key".into()), &file, true, || {
        panic!("must not read saved credentials")
    })
    .unwrap();
    assert_eq!(env.key.expose(), "environment-key");
    let repo = sources::resolve_with(None, &file, false, || {
        panic!("must not read saved credentials")
    })
    .unwrap();
    assert_eq!(repo.key.expose(), "repo-key");
    project.write(".env", "UNRELATED=keep-me\n");
    let saved = sources::resolve_with(None, &file, false, || {
        Ok(Some((
            Secret::parse("saved-key".into())?,
            "system credential store".into(),
        )))
    })
    .unwrap();
    assert_eq!(saved.key.expose(), "saved-key");
    assert!(
        sources::resolve_with(None, &file, true, || panic!(
            "explicit missing key must fail"
        ))
        .is_err()
    );
    assert_eq!(
        std::fs::read_to_string(file).unwrap(),
        "UNRELATED=keep-me\n"
    );
}

#[test]
fn openrouter_uses_its_own_environment_and_env_file_key_before_any_saved_key() {
    let project = crate::tests::Project::new();
    let file = project.0.join(".env");
    project.write(
        ".env",
        "TYPESAFE_API_KEY=typesafe-file-key\nOPENROUTER_API_KEY=openrouter-file-key\n",
    );

    let environment = sources::resolve_with_named(
        Some("openrouter-environment-key".into()),
        &file,
        true,
        crate::options::Provider::OpenRouter,
        || panic!("environment credentials take precedence"),
    )
    .unwrap();
    assert_eq!(environment.key.expose(), "openrouter-environment-key");
    assert_eq!(
        environment.source,
        "OPENROUTER_API_KEY environment variable"
    );

    let file_credential = sources::resolve_with_named(
        None,
        &file,
        false,
        crate::options::Provider::OpenRouter,
        || panic!("the selected provider's env-file key takes precedence"),
    )
    .unwrap();
    assert_eq!(file_credential.key.expose(), "openrouter-file-key");

    project.write(".env", "TYPESAFE_API_KEY=typesafe-only-key\n");
    let missing = sources::resolve_with_named(
        None,
        &file,
        false,
        crate::options::Provider::OpenRouter,
        || Ok(None),
    )
    .err()
    .expect("an OpenRouter key is required")
    .to_string();
    assert!(missing.contains("OPENROUTER_API_KEY"));
}

#[test]
fn invalid_or_duplicate_keys_never_fall_through_or_appear_in_errors() {
    let project = crate::tests::Project::new();
    project.write(
        ".env",
        "TYPESAFE_API_KEY=private-key\nTYPESAFE_API_KEY=another-private-key\n",
    );
    let error = sources::key_from_file(&project.0.join(".env"))
        .err()
        .unwrap()
        .to_string();
    assert!(!error.contains("private-key"));
    for value in ["", "private\nkey", "private key", "private\u{7f}key"] {
        assert!(Secret::parse(value.into()).is_err());
    }
    assert!(
        sources::resolve_with(
            Some("bad key".into()),
            &project.0.join(".env"),
            false,
            || panic!("invalid override must not fall through")
        )
        .is_err()
    );
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

#[cfg(unix)]
#[test]
fn fallback_rejects_symlinks_hardlinks_and_broad_permissions() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let project = crate::tests::Project::new();
    let saved = store(&project.0, StorageMode::File);
    let key = Secret::parse("test-key".into()).unwrap();
    saved.save(&key).unwrap();
    let another = project.0.join("linked-secret");
    std::fs::hard_link(&saved.path, &another).unwrap();
    assert!(saved.get().is_err());
    assert!(saved.save(&key).is_err());
    std::fs::remove_file(another).unwrap();
    std::fs::set_permissions(&saved.path, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(saved.get().is_err());
    std::fs::remove_file(&saved.path).unwrap();
    project.write("external", "do-not-touch");
    symlink(project.0.join("external"), &saved.path).unwrap();
    assert!(saved.save(&key).is_err());
    assert!(saved.remove().is_err());
    assert_eq!(
        std::fs::read_to_string(project.0.join("external")).unwrap(),
        "do-not-touch"
    );
}

#[test]
fn authentication_is_known_only_after_a_checked_connection() {
    assert_eq!(provider::authenticated(&Ok(()), false), None);
    assert_eq!(provider::authenticated(&Ok(()), true), Some(true));
    assert_eq!(
        provider::authenticated(&provider::http_error(403), true),
        Some(false)
    );
    assert_eq!(
        provider::authenticated(&provider::http_error(429), true),
        None
    );
}

#[test]
fn model_listing_errors_never_echo_provider_text() {
    assert!(
        provider::validate_models(&serde_json::json!({"models":[{"name":"jev-latest"}]})).is_ok()
    );
    let error = provider::validate_models(&serde_json::json!({"error":"secret-do-not-echo"}))
        .unwrap_err()
        .to_string();
    assert!(!error.contains("secret-do-not-echo"));
}

#[test]
fn http_errors_explain_rejection_and_retry() {
    assert!(
        provider::http_error(403)
            .unwrap_err()
            .to_string()
            .contains("rejected")
    );
    assert!(
        provider::http_error(429)
            .unwrap_err()
            .to_string()
            .contains("not retried")
    );
}
