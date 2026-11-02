//! Server-scoped OS credentials. No automatic plaintext fallback.
//! OS store operations are blocking, serialized, and never run on the UI thread.

use anyhow::{Result, anyhow, bail, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fmt, fs,
    io::Read,
    path::{Path, PathBuf},
    sync::Mutex,
};
use tokio_util::sync::CancellationToken;
use zeroize::{ZeroizeOnDrop, Zeroizing};

const SERVICE: &str = "jorik-cli";
const MAX_LEGACY_BYTES: u64 = 16 * 1024;
static STORE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq, ZeroizeOnDrop)]
pub struct Auth {
    pub token: String,
    pub avatar_url: Option<String>,
    pub username: Option<String>,
}

impl fmt::Debug for Auth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Auth")
            .field("token", &"[redacted]")
            .field("username", &self.username)
            .finish_non_exhaustive()
    }
}

pub fn validate_token(token: &str) -> Result<()> {
    ensure!(
        !token.is_empty()
            && token.len() <= 2048
            && token
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"-._~+/=".contains(&c)),
        "invalid bearer token"
    );
    Ok(())
}

pub fn validate_auth(auth: &Auth) -> Result<()> {
    validate_token(&auth.token)?;
    if let Some(username) = &auth.username {
        ensure!(
            username.len() <= 512 && !username.chars().any(unsafe_text),
            "invalid display name"
        );
    }
    if let Some(avatar) = &auth.avatar_url {
        ensure!(
            avatar.len() <= 2048 && !avatar.chars().any(unsafe_text),
            "invalid avatar URL"
        );
        let url = url::Url::parse(avatar).map_err(|_| anyhow!("invalid avatar URL"))?;
        ensure!(
            url.scheme() == "https"
                && url.host_str().is_some()
                && url.username().is_empty()
                && url.password().is_none(),
            "invalid avatar URL"
        );
    }
    Ok(())
}

fn unsafe_text(c: char) -> bool {
    c.is_control() || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

pub fn server_identity(base: &str) -> Result<String> {
    ensure!(
        !base.chars().any(unsafe_text),
        "server URL contains unsafe control characters"
    );
    crate::api::validate_base_url(base)?;
    let mut url = url::Url::parse(base)?;
    let local = url.host_str() == Some("localhost")
        || match url.host() {
            Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
            Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
            _ => false,
        };
    ensure!(
        url.scheme() == "https" || local,
        "HTTPS is required except for loopback development servers"
    );
    let path = url.path().trim_end_matches('/').to_string();
    url.set_path(&path);
    Ok(url.as_str().trim_end_matches('/').to_string())
}

fn account(server: &str) -> String {
    format!("server:{:x}", Sha256::digest(server.as_bytes()))
}

trait SecureStore {
    fn get(&self, account: &str) -> Result<Option<Auth>>;
    fn put(&self, account: &str, auth: &Auth) -> Result<()>;
    fn delete(&self, account: &str) -> Result<()>;
}

struct OsStore;

fn storage_error() -> anyhow::Error {
    // Do not format native errors: some include credential bytes (BadEncoding).
    anyhow!(
        "OS credential store is unavailable or locked. Unlock your keychain/Secret Service and retry; no plaintext fallback is used. For headless use, supply JORIK_TOKEN explicitly"
    )
}

impl OsStore {
    fn entry(account: &str) -> Result<keyring::Entry> {
        ensure!(
            cfg!(any(
                target_os = "linux",
                target_os = "windows",
                target_os = "macos"
            )),
            "this platform has no supported OS credential store; supply JORIK_TOKEN explicitly"
        );
        keyring::Entry::new(SERVICE, account).map_err(|_| storage_error())
    }
}

impl SecureStore for OsStore {
    fn get(&self, account: &str) -> Result<Option<Auth>> {
        let value = match Self::entry(account)?.get_password() {
            Ok(value) => Zeroizing::new(value),
            Err(keyring::Error::NoEntry) => return Ok(None),
            Err(_) => return Err(storage_error()),
        };
        let auth: Auth = serde_json::from_str(&value)
            .map_err(|_| anyhow!("stored credential is invalid; sign out and log in again"))?;
        validate_auth(&auth)?;
        Ok(Some(auth))
    }

    fn put(&self, account: &str, auth: &Auth) -> Result<()> {
        validate_auth(auth)?;
        let json = Zeroizing::new(serde_json::to_string(auth)?);
        Self::entry(account)?
            .set_password(&json)
            .map_err(|_| storage_error())
    }

    fn delete(&self, account: &str) -> Result<()> {
        match Self::entry(account)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(storage_error()),
        }
    }
}

fn config_root() -> Result<PathBuf> {
    dirs::config_dir()
        .map(|path| path.join("jorik-cli"))
        .ok_or_else(|| anyhow!("cannot determine credential migration directory"))
}

fn read_legacy(path: &Path) -> Result<Option<Auth>> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => bail!("cannot inspect legacy credentials"),
    };
    ensure!(
        metadata.file_type().is_file(),
        "legacy credentials must be a regular file, not a symlink"
    );
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    let file = options
        .open(path)
        .map_err(|_| anyhow!("cannot open legacy credentials"))?;
    ensure!(
        file.metadata()?.file_type().is_file(),
        "legacy credentials must be a regular file"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        ensure!(
            file.metadata()?.nlink() == 1,
            "legacy credentials must not be hard-linked"
        );
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|_| anyhow!("cannot restrict legacy credential permissions"))?;
    }
    let mut json = Zeroizing::new(String::new());
    file.take(MAX_LEGACY_BYTES + 1)
        .read_to_string(&mut json)
        .map_err(|_| anyhow!("cannot read legacy credentials"))?;
    ensure!(
        json.len() as u64 <= MAX_LEGACY_BYTES,
        "legacy credential file is too large"
    );
    let auth = serde_json::from_str(&json)
        .map_err(|_| anyhow!("invalid legacy credentials; log in again"))?;
    validate_auth(&auth)?;
    Ok(Some(auth))
}

fn remove_legacy(directory: &Path) -> Result<()> {
    for name in ["auth.json", "token"] {
        match fs::remove_file(directory.join(name)) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => {
                bail!("the legacy plaintext file could not be removed; remove it before continuing")
            }
        }
    }
    Ok(())
}

fn load_with(store: &impl SecureStore, _directory: &Path, server: &str) -> Result<Option<Auth>> {
    // Legacy files did not record an origin. Never infer one or send their
    // token to any server without the user's explicit confirmation.
    store.get(&account(server))
}

fn migrate_with(
    store: &impl SecureStore,
    directory: &Path,
    server: &str,
    cancel: &CancellationToken,
) -> Result<Option<Auth>> {
    let Some(auth) = read_legacy(&directory.join("auth.json"))? else {
        return Ok(None);
    };
    if let Some(existing) = store.get(&account(server))? {
        ensure!(
            existing == auth,
            "different scoped credentials already exist; legacy credentials were retained. Clear legacy files explicitly if they are no longer needed"
        );
    }
    save_with(store, directory, server, &auth, cancel)?;
    ensure!(
        !cancel.is_cancelled(),
        "migration cancelled; legacy credentials were retained"
    );
    remove_legacy(directory)?;
    Ok(Some(auth))
}

fn save_with(
    store: &impl SecureStore,
    _directory: &Path,
    server: &str,
    auth: &Auth,
    cancel: &CancellationToken,
) -> Result<()> {
    validate_auth(auth)?;
    ensure!(!cancel.is_cancelled(), "login cancelled");
    let id = account(server);
    let previous = store.get(&id)?;
    store.put(&id, auth)?;
    let verified = store.get(&id).ok().flatten().as_ref() == Some(auth);
    if cancel.is_cancelled() || !verified {
        match previous {
            Some(ref previous) => store.put(&id, previous)?,
            None => store.delete(&id)?,
        }
        bail!("login cancelled or credential verification failed; previous credentials restored");
    }
    Ok(())
}

fn delete_with(store: &impl SecureStore, _directory: &Path, server: &str) -> Result<()> {
    store.delete(&account(server))
}

pub fn legacy_present() -> bool {
    config_root().ok().is_some_and(|directory| {
        ["auth.json", "token"]
            .iter()
            .any(|name| fs::symlink_metadata(directory.join(name)).is_ok())
    })
}

async fn blocking<T: Send + 'static>(
    operation: impl FnOnce(&OsStore, &Path) -> Result<T> + Send + 'static,
) -> Result<T> {
    tokio::task::spawn_blocking(move || {
        let _guard = STORE_LOCK
            .lock()
            .map_err(|_| anyhow!("credential store lock is unavailable"))?;
        operation(&OsStore, &config_root()?)
    })
    .await
    .map_err(|_| anyhow!("credential store operation failed"))?
}

pub async fn load(base: &str) -> Result<Option<Auth>> {
    let server = server_identity(base)?;
    blocking(move |store, directory| load_with(store, directory, &server)).await
}

pub async fn save(base: &str, auth: Auth, cancel: CancellationToken) -> Result<()> {
    let server = server_identity(base)?;
    blocking(move |store, directory| save_with(store, directory, &server, &auth, &cancel)).await
}

pub async fn delete(base: &str) -> Result<()> {
    let server = server_identity(base)?;
    blocking(move |store, directory| delete_with(store, directory, &server)).await
}

pub async fn migrate(base: &str, cancel: CancellationToken) -> Result<Option<Auth>> {
    let server = server_identity(base)?;
    blocking(move |store, directory| migrate_with(store, directory, &server, &cancel)).await
}

pub async fn clear_legacy() -> Result<()> {
    blocking(|_, directory| remove_legacy(directory)).await
}

pub async fn check_available(base: &str) -> Result<()> {
    let server = server_identity(base)?;
    blocking(move |store, _| {
        store.get(&account(&server))?;
        Ok(())
    })
    .await
}

#[cfg(test)]
mod tests;
