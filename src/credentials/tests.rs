use super::*;
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
};

#[derive(Default)]
struct FakeStore {
    values: RefCell<HashMap<String, Auth>>,
    fail_get: Cell<bool>,
    fail_put: Cell<bool>,
    fail_delete: Cell<bool>,
    corrupt_after_put: Cell<bool>,
    corrupt_next: Cell<bool>,
    cancel_on_put: RefCell<Option<CancellationToken>>,
}

impl SecureStore for FakeStore {
    fn get(&self, id: &str) -> Result<Option<Auth>> {
        if self.fail_get.get() {
            bail!("store unavailable");
        }
        if self.corrupt_next.replace(false) {
            return Ok(Some(auth("unexpected")));
        }
        Ok(self.values.borrow().get(id).cloned())
    }
    fn put(&self, id: &str, value: &Auth) -> Result<()> {
        if self.fail_put.get() {
            bail!("store unavailable");
        }
        self.values
            .borrow_mut()
            .insert(id.to_string(), value.clone());
        if self.corrupt_after_put.get() {
            self.corrupt_next.set(true);
        }
        if let Some(cancel) = self.cancel_on_put.borrow_mut().take() {
            cancel.cancel();
        }
        Ok(())
    }
    fn delete(&self, id: &str) -> Result<()> {
        if self.fail_delete.get() {
            bail!("store unavailable");
        }
        self.values.borrow_mut().remove(id);
        Ok(())
    }
}

fn auth(token: &str) -> Auth {
    Auth {
        token: token.to_string(),
        username: Some("User".to_string()),
        avatar_url: None,
    }
}

fn legacy(directory: &Path, value: &Auth) {
    fs::write(
        directory.join("auth.json"),
        serde_json::to_vec(value).unwrap(),
    )
    .unwrap();
}

#[test]
fn origins_are_normalized_but_not_shared_across_hosts_ports_or_api_paths() {
    assert_eq!(
        server_identity("https://EXAMPLE.com:443/api/").unwrap(),
        "https://example.com/api"
    );
    let a = server_identity("https://example.com").unwrap();
    for base in [
        "https://other.example",
        "https://example.com:8443",
        "https://example.com/api",
    ] {
        assert_ne!(account(&a), account(&server_identity(base).unwrap()));
    }
    for base in [
        "http://127.0.0.1:8080",
        "http://[::1]:8080",
        "http://localhost:8080",
    ] {
        assert!(server_identity(base).is_ok());
    }
    for base in [
        "http://example.com",
        "https://user:secret@example.com",
        "https://example.com?q=x",
        "file:///tmp/server",
        "https://example.com/\u{1b}",
    ] {
        assert!(server_identity(base).is_err());
    }
}

#[test]
fn successful_migration_verifies_secure_storage_before_removing_plaintext() {
    let dir = tempfile::tempdir().unwrap();
    let store = FakeStore::default();
    let expected = auth("legacy-token");
    legacy(dir.path(), &expected);
    let loaded = migrate_with(
        &store,
        dir.path(),
        crate::api::DEFAULT_BASE_URL,
        &CancellationToken::new(),
    )
    .unwrap()
    .unwrap();
    assert_eq!(loaded, expected);
    assert!(!dir.path().join("auth.json").exists());
    assert_eq!(
        store
            .get(&account(crate::api::DEFAULT_BASE_URL))
            .unwrap()
            .as_ref(),
        Some(&expected)
    );
}

#[test]
fn migration_failures_preserve_the_legacy_credential() {
    for failure in ["get", "put", "verify"] {
        let dir = tempfile::tempdir().unwrap();
        let store = FakeStore::default();
        legacy(dir.path(), &auth("retain-me"));
        store.fail_get.set(failure == "get");
        store.fail_put.set(failure == "put");
        store.corrupt_after_put.set(failure == "verify");
        assert!(
            migrate_with(
                &store,
                dir.path(),
                crate::api::DEFAULT_BASE_URL,
                &CancellationToken::new()
            )
            .is_err()
        );
        assert!(
            fs::read_to_string(dir.path().join("auth.json"))
                .unwrap()
                .contains("retain-me")
        );
    }
}

#[test]
fn no_host_can_silently_import_unscoped_legacy_tokens() {
    let dir = tempfile::tempdir().unwrap();
    legacy(dir.path(), &auth("private-legacy-token"));
    let store = FakeStore::default();
    for server in ["https://custom.example", crate::api::DEFAULT_BASE_URL] {
        assert!(load_with(&store, dir.path(), server).unwrap().is_none());
    }
    assert!(store.values.borrow().is_empty());
    assert!(dir.path().join("auth.json").exists());
}

#[test]
fn explicit_confirmation_can_migrate_to_a_custom_host() {
    let dir = tempfile::tempdir().unwrap();
    legacy(dir.path(), &auth("custom-token"));
    let store = FakeStore::default();
    migrate_with(
        &store,
        dir.path(),
        "https://custom.example",
        &CancellationToken::new(),
    )
    .unwrap();
    assert!(
        load_with(&store, dir.path(), crate::api::DEFAULT_BASE_URL)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        load_with(&store, dir.path(), "https://custom.example")
            .unwrap()
            .unwrap()
            .token,
        "custom-token"
    );
}

#[test]
fn migration_does_not_overwrite_different_existing_scoped_credentials() {
    let dir = tempfile::tempdir().unwrap();
    legacy(dir.path(), &auth("obsolete"));
    let store = FakeStore::default();
    store
        .put(&account(crate::api::DEFAULT_BASE_URL), &auth("current"))
        .unwrap();
    assert!(
        migrate_with(
            &store,
            dir.path(),
            crate::api::DEFAULT_BASE_URL,
            &CancellationToken::new()
        )
        .is_err()
    );
    assert_eq!(
        store
            .get(&account(crate::api::DEFAULT_BASE_URL))
            .unwrap()
            .unwrap()
            .token,
        "current"
    );
    assert!(dir.path().join("auth.json").exists());
}

#[test]
fn saving_never_creates_a_plaintext_fallback() {
    let dir = tempfile::tempdir().unwrap();
    let store = FakeStore::default();
    store.fail_put.set(true);
    assert!(
        save_with(
            &store,
            dir.path(),
            crate::api::DEFAULT_BASE_URL,
            &auth("secret"),
            &CancellationToken::new()
        )
        .is_err()
    );
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn cancelling_a_save_restores_the_previous_secure_credential() {
    for previous in [None, Some(auth("previous"))] {
        let dir = tempfile::tempdir().unwrap();
        let store = FakeStore::default();
        let id = account(crate::api::DEFAULT_BASE_URL);
        if let Some(previous) = previous.as_ref() {
            store.put(&id, previous).unwrap();
        }
        let cancel = CancellationToken::new();
        *store.cancel_on_put.borrow_mut() = Some(cancel.clone());
        assert!(
            save_with(
                &store,
                dir.path(),
                crate::api::DEFAULT_BASE_URL,
                &auth("new"),
                &cancel
            )
            .is_err()
        );
        assert_eq!(store.values.borrow().get(&id), previous.as_ref());
    }
}

#[test]
fn signout_is_scoped_and_legacy_erasure_requires_explicit_intent() {
    let dir = tempfile::tempdir().unwrap();
    let store = FakeStore::default();
    store
        .put(&account(crate::api::DEFAULT_BASE_URL), &auth("public"))
        .unwrap();
    store
        .put(&account("https://custom.example"), &auth("custom"))
        .unwrap();
    legacy(dir.path(), &auth("legacy"));
    delete_with(&store, dir.path(), "https://custom.example").unwrap();
    assert!(
        store
            .get(&account(crate::api::DEFAULT_BASE_URL))
            .unwrap()
            .is_some()
    );
    assert!(dir.path().join("auth.json").exists());
    store.fail_delete.set(true);
    assert!(delete_with(&store, dir.path(), crate::api::DEFAULT_BASE_URL).is_err());
    assert!(dir.path().join("auth.json").exists());
    remove_legacy(dir.path()).unwrap(); // Explicit cleanup works even with the OS store unavailable.
    assert!(!dir.path().join("auth.json").exists());
}

#[test]
fn credential_debug_and_validation_do_not_expose_secrets() {
    let value = auth("super-secret-token");
    assert!(!format!("{value:?}").contains("super-secret-token"));
    for token in ["", "has spaces", "header\r\ninjection", "secret\x1b[31m"] {
        assert!(validate_auth(&auth(token)).is_err());
    }
    let mut value = auth("valid-token");
    value.username = Some("user\x1b[31m".to_string());
    assert!(validate_auth(&value).is_err());
    value.username = None;
    value.avatar_url = Some("http://external.example/image".to_string());
    assert!(validate_auth(&value).is_err());
}

#[tokio::test]
#[ignore = "requires an unlocked native OS credential store; creates and removes a test-only entry"]
async fn native_credential_store_round_trip() {
    let mut random = [0; 32];
    getrandom::fill(&mut random).unwrap();
    let server = format!(
        "https://credential-test.invalid/{:x}",
        Sha256::digest(random)
    );
    let expected = auth("test-only-not-a-real-token");
    let written = save(&server, expected.clone(), CancellationToken::new()).await;
    let loaded = load(&server).await;
    let isolated = load(&format!("{server}/other")).await;
    let replacement = auth("replacement-test-token");
    let overwritten = save(&server, replacement.clone(), CancellationToken::new()).await;
    let reloaded = load(&server).await;
    let removed = delete(&server).await; // Always try cleanup before asserting.
    let missing = load(&server).await;
    written.unwrap();
    assert_eq!(loaded.unwrap().as_ref(), Some(&expected));
    assert!(isolated.unwrap().is_none());
    overwritten.unwrap();
    assert_eq!(reloaded.unwrap().as_ref(), Some(&replacement));
    removed.unwrap();
    assert!(missing.unwrap().is_none());
}

#[cfg(target_os = "linux")]
#[tokio::test]
#[ignore = "requires JORIK_TEST_UNAVAILABLE_STORE=1 and an intentionally unavailable DBus session"]
async fn native_unavailable_store_fails_closed() {
    assert_eq!(
        std::env::var("JORIK_TEST_UNAVAILABLE_STORE").as_deref(),
        Ok("1")
    );
    assert!(!legacy_present());
    let server = "https://credential-unavailable-test.invalid";
    let secret = "test-only-no-plaintext-fallback";
    let written = save(server, auth(secret), CancellationToken::new()).await;
    let _ = delete(server).await;
    let error = written.unwrap_err().to_string();
    assert!(!error.contains(secret));
    assert!(error.contains("no plaintext fallback"));
    assert!(load(server).await.is_err());
    assert!(!legacy_present());
}

#[cfg(unix)]
#[test]
fn legacy_symlinks_are_rejected_and_regular_files_are_restricted() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let dir = tempfile::tempdir().unwrap();
    let original = dir.path().join("original");
    fs::write(&original, serde_json::to_vec(&auth("secret")).unwrap()).unwrap();
    let link = dir.path().join("auth.json");
    symlink(&original, &link).unwrap();
    assert!(read_legacy(&link).is_err());
    fs::remove_file(&link).unwrap();
    fs::set_permissions(&original, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(read_legacy(&original).unwrap().is_some());
    assert_eq!(
        fs::metadata(&original).unwrap().permissions().mode() & 0o777,
        0o600
    );
}
