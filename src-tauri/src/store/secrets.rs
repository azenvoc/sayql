//! Database passwords and AI API keys, kept only in the OS keychain.
//!
//! They all live in **one** keychain item, read once per launch and cached in memory.
//! macOS asks before an app reads a keychain item when the app's signature has changed
//! (every update of an unsigned build), and it asks per item: one item means one prompt,
//! not one per saved password.
//!
//! Secrets saved by earlier versions, one keychain item each, are copied into the shared
//! item the first time they're needed. The old items are left in place until the secret
//! is removed (say, when its connection is deleted).

use crate::error::{AppError, AppResult};
use crate::state::lock;
use std::collections::BTreeMap;
use std::sync::{LazyLock, Mutex, OnceLock};

/// The keychain service: the app's identifier, `com.sayql.app`, so a test build with an
/// identifier of its own never reads or writes the app's secrets.
static SERVICE: OnceLock<String> = OnceLock::new();
const DEFAULT_SERVICE: &str = "com.sayql.app";

/// Sets the keychain service to the app's identifier, once, at startup.
pub fn set_service(identifier: &str) {
    let _ = SERVICE.set(identifier.to_string());
}

fn service() -> &'static str {
    SERVICE.get().map_or(DEFAULT_SERVICE, String::as_str)
}

/// AskDB's items belong to the app itself, not to a test build.
fn legacy_service() -> Option<&'static str> {
    (service() == DEFAULT_SERVICE).then_some(LEGACY_SERVICE)
}
/// The account of the single item holding every secret, as a JSON object.
const VAULT_ACCOUNT: &str = "secrets";
/// The keychain service used before the app was renamed from AskDB.
const LEGACY_SERVICE: &str = "com.askdb.app";
const API_KEY_ACCOUNT: &str = "anthropic-api-key";

/// Where keychain items are read and written; a fake in tests.
trait Backend {
    fn read(&self, service: &str, account: &str) -> AppResult<Option<String>>;
    fn write(&self, service: &str, account: &str, value: &str) -> AppResult<()>;
    /// Removes an item; a missing item is not an error.
    fn delete(&self, service: &str, account: &str) -> AppResult<()>;
}

/// The OS keychain; unit tests use `TestKeychain` instead.
#[derive(Default)]
#[cfg_attr(test, allow(dead_code, reason = "unit tests use TestKeychain"))]
struct Keychain;

impl Backend for Keychain {
    fn read(&self, service: &str, account: &str) -> AppResult<Option<String>> {
        match keyring::Entry::new(service, account)?.get_password() {
            Ok(v) => Ok(Some(v).filter(|v| !v.is_empty())),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn write(&self, service: &str, account: &str, value: &str) -> AppResult<()> {
        Ok(keyring::Entry::new(service, account)?.set_password(value)?)
    }

    fn delete(&self, service: &str, account: &str) -> AppResult<()> {
        match keyring::Entry::new(service, account)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}

struct Vault<B: Backend> {
    backend: B,
    /// Loaded on first use; `None` until then.
    secrets: Option<BTreeMap<String, String>>,
    /// Why the shared item couldn't be read (say, the person clicked Deny). Kept for the
    /// rest of the launch so the keychain doesn't ask again for every secret.
    unavailable: Option<String>,
    /// Accounts already looked up in earlier versions' storage and not found there.
    not_found: std::collections::HashSet<String>,
}

impl<B: Backend> Vault<B> {
    fn new(backend: B) -> Self {
        Vault {
            backend,
            secrets: None,
            unavailable: None,
            not_found: Default::default(),
        }
    }

    fn loaded(&mut self) -> AppResult<&mut BTreeMap<String, String>> {
        if let Some(why) = &self.unavailable {
            return Err(AppError::User(why.clone()));
        }
        if self.secrets.is_none() {
            let map = match self.backend.read(service(), VAULT_ACCOUNT) {
                Ok(None) => Ok(BTreeMap::new()),
                // Never treat an unreadable item as empty: saving would erase every secret.
                Ok(Some(json)) => serde_json::from_str(&json).map_err(|_| "SayQL's saved passwords in the keychain are unreadable.".to_string()),
                Err(e) => Err(format!("SayQL couldn't read its saved passwords from the keychain ({e}). Restart SayQL to try again.")),
            }
            .map_err(|why| {
                self.unavailable = Some(why.clone());
                AppError::User(why)
            })?;
            self.secrets = Some(map);
        }
        Ok(self.secrets.as_mut().expect("loaded above"))
    }

    fn save(&mut self) -> AppResult<()> {
        let json = serde_json::to_string(self.loaded()?)?;
        self.backend.write(service(), VAULT_ACCOUNT, &json)
    }

    fn get(&mut self, account: &str) -> AppResult<Option<String>> {
        if let Some(v) = self.loaded()?.get(account) {
            return Ok(Some(v.clone()));
        }
        if self.not_found.contains(account) {
            return Ok(None);
        }
        // Saved by an earlier version as its own item: copy it into the vault.
        let old = match self.backend.read(service(), account)? {
            Some(v) => Some(v),
            None => match legacy_service() {
                Some(legacy) => self.backend.read(legacy, account)?,
                None => None,
            },
        };
        match &old {
            Some(v) => {
                self.loaded()?.insert(account.to_string(), v.clone());
                self.save()?;
            }
            None => {
                self.not_found.insert(account.to_string());
            }
        }
        Ok(old)
    }

    /// An empty value removes the secret.
    fn set(&mut self, account: &str, value: &str) -> AppResult<()> {
        self.not_found.remove(account);
        let secrets = self.loaded()?;
        let changed = if value.is_empty() {
            secrets.remove(account).is_some()
        } else {
            secrets
                .insert(account.to_string(), value.to_string())
                .as_deref()
                != Some(value)
        };
        if changed { self.save() } else { Ok(()) }
    }

    /// Removes the secret everywhere, including the item an earlier version saved it in.
    /// Each location is tried even when another fails; the first error is returned.
    fn remove(&mut self, account: &str) -> AppResult<()> {
        let shared = self.set(account, "");
        self.not_found.insert(account.to_string());
        let current = self.backend.delete(service(), account);
        let legacy = legacy_service().map_or(Ok(()), |l| self.backend.delete(l, account));
        shared.and(current).and(legacy)
    }
}

/// Unit tests get an empty keychain of their own: they must never read or change the real
/// one (and on macOS a read can wait on a permission prompt nobody answers).
#[cfg(test)]
#[derive(Default)]
struct TestKeychain(Mutex<BTreeMap<(String, String), String>>);

#[cfg(test)]
impl Backend for TestKeychain {
    fn read(&self, service: &str, account: &str) -> AppResult<Option<String>> {
        Ok(lock(&self.0)
            .get(&(service.into(), account.into()))
            .cloned())
    }
    fn write(&self, service: &str, account: &str, value: &str) -> AppResult<()> {
        lock(&self.0).insert((service.into(), account.into()), value.into());
        Ok(())
    }
    fn delete(&self, service: &str, account: &str) -> AppResult<()> {
        lock(&self.0).remove(&(service.into(), account.into()));
        Ok(())
    }
}

#[cfg(not(test))]
type Live = Keychain;
#[cfg(test)]
type Live = TestKeychain;

static VAULT: LazyLock<Mutex<Vault<Live>>> =
    LazyLock::new(|| Mutex::new(Vault::new(Live::default())));

fn get(account: &str) -> Option<String> {
    lock(&VAULT).get(account).ok().flatten()
}

fn set(account: &str, value: &str) -> AppResult<()> {
    lock(&VAULT).set(account, value)
}

fn connection_account(conn_id: &str) -> String {
    format!("conn:{conn_id}")
}

pub fn password(conn_id: &str) -> Option<String> {
    get(&connection_account(conn_id))
}

/// [`password`] off the async runtime: the first keychain read of a launch can wait on the
/// system's permission prompt, and must not hold up other requests meanwhile.
pub async fn password_async(conn_id: &str) -> Option<String> {
    let id = conn_id.to_string();
    tokio::task::spawn_blocking(move || password(&id))
        .await
        .ok()
        .flatten()
}

/// [`api_key`] off the async runtime, for the same reason as [`password_async`].
pub async fn api_key_async(provider: &str) -> Option<String> {
    let provider = provider.to_string();
    tokio::task::spawn_blocking(move || api_key(&provider))
        .await
        .ok()
        .flatten()
}

pub fn set_password(conn_id: &str, password: &str) -> AppResult<()> {
    set(&connection_account(conn_id), password)
}

/// Removes a connection's password, including any copy an earlier version left in its own
/// keychain item. A connection without a saved password is not an error.
pub fn delete_password(conn_id: &str) -> AppResult<()> {
    lock(&VAULT).remove(&connection_account(conn_id))
}

/// Claude's key keeps the account name it had before other providers were added.
fn api_key_account(provider: &str) -> String {
    if provider == crate::ai::ANTHROPIC {
        API_KEY_ACCOUNT.to_string()
    } else {
        format!("api-key:{provider}")
    }
}

/// The AI provider's API key. In debug builds, `ANTHROPIC_API_KEY` is a fallback for Claude;
/// release builds only use the key saved in Settings.
pub fn api_key(provider: &str) -> Option<String> {
    let saved = get(&api_key_account(provider));
    if provider != crate::ai::ANTHROPIC || !cfg!(debug_assertions) {
        return saved;
    }
    saved.or_else(|| {
        std::env::var("ANTHROPIC_API_KEY")
            .ok()
            .filter(|s| !s.is_empty())
    })
}

pub fn set_api_key(provider: &str, key: &str) -> AppResult<()> {
    set(&api_key_account(provider), key.trim())
}

const TEAM_TOKEN_ACCOUNT: &str = "team-token";

/// The SayQL Team sign-in token. Only read when settings say a team server is signed in.
pub fn team_token() -> Option<String> {
    get(TEAM_TOKEN_ACCOUNT)
}

pub fn set_team_token(token: &str) -> AppResult<()> {
    set(TEAM_TOKEN_ACCOUNT, token)
}

pub fn delete_team_token() -> AppResult<()> {
    lock(&VAULT).remove(TEAM_TOKEN_ACCOUNT)
}

fn repo_account(source_id: &str) -> String {
    format!("repo:{source_id}")
}

/// A git code source's read-only token (see crate::repos).
pub fn repo_token(source_id: &str) -> Option<String> {
    get(&repo_account(source_id))
}

/// Saves a code source's token; an empty one removes it.
pub fn set_repo_token(source_id: &str, token: &str) -> AppResult<()> {
    set(&repo_account(source_id), token.trim())
}

/// Removes a code source's token. Only the shared item holds them: no earlier version saved
/// one as its own item.
pub fn delete_repo_token(source_id: &str) -> AppResult<()> {
    set(&repo_account(source_id), "")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// Keychain items by (service, account), counting reads and writes.
    #[derive(Default)]
    struct Fake {
        items: RefCell<BTreeMap<(String, String), String>>,
        reads: RefCell<usize>,
        writes: RefCell<usize>,
        /// Every read fails, as when the person clicks Deny.
        deny: bool,
    }

    impl Backend for &Fake {
        fn read(&self, service: &str, account: &str) -> AppResult<Option<String>> {
            *self.reads.borrow_mut() += 1;
            if self.deny {
                return Err("User canceled the operation.".into());
            }
            Ok(self
                .items
                .borrow()
                .get(&(service.into(), account.into()))
                .cloned())
        }
        fn write(&self, service: &str, account: &str, value: &str) -> AppResult<()> {
            *self.writes.borrow_mut() += 1;
            self.items
                .borrow_mut()
                .insert((service.into(), account.into()), value.into());
            Ok(())
        }
        fn delete(&self, service: &str, account: &str) -> AppResult<()> {
            if self.deny {
                return Err("User canceled the operation.".into());
            }
            self.items
                .borrow_mut()
                .remove(&(service.to_string(), account.to_string()));
            Ok(())
        }
    }

    fn vault(fake: &Fake) -> Vault<&Fake> {
        Vault::new(fake)
    }

    #[test]
    fn every_secret_lives_in_one_item_read_once() {
        let fake = Fake::default();
        let mut v = vault(&fake);
        v.set("conn:a", "pw-a").unwrap();
        v.set("conn:b", "pw-b").unwrap();
        v.set("anthropic-api-key", "sk-ant").unwrap();
        assert_eq!(fake.items.borrow().len(), 1, "one keychain item");

        // A new launch reads that one item once, however many secrets are asked for.
        let mut next = vault(&fake);
        *fake.reads.borrow_mut() = 0;
        assert_eq!(next.get("conn:a").unwrap().as_deref(), Some("pw-a"));
        assert_eq!(next.get("conn:b").unwrap().as_deref(), Some("pw-b"));
        assert_eq!(
            next.get("anthropic-api-key").unwrap().as_deref(),
            Some("sk-ant")
        );
        assert_eq!(*fake.reads.borrow(), 1);
    }

    #[test]
    fn copies_secrets_saved_one_item_each_by_earlier_versions() {
        let fake = Fake::default();
        fake.items
            .borrow_mut()
            .insert((service().into(), "conn:a".into()), "old-a".into());
        fake.items.borrow_mut().insert(
            (LEGACY_SERVICE.into(), "anthropic-api-key".into()),
            "old-key".into(),
        );

        let mut v = vault(&fake);
        assert_eq!(v.get("conn:a").unwrap().as_deref(), Some("old-a"));
        assert_eq!(
            v.get("anthropic-api-key").unwrap().as_deref(),
            Some("old-key")
        );
        assert_eq!(v.get("conn:missing").unwrap(), None);

        // From now on they come from the shared item alone.
        let mut next = vault(&fake);
        *fake.reads.borrow_mut() = 0;
        assert_eq!(next.get("conn:a").unwrap().as_deref(), Some("old-a"));
        assert_eq!(*fake.reads.borrow(), 1);
    }

    #[test]
    fn removing_or_resaving_the_same_value() {
        let fake = Fake::default();
        let mut v = vault(&fake);
        v.set("conn:a", "pw").unwrap();
        let writes = *fake.writes.borrow();
        v.set("conn:a", "pw").unwrap();
        assert_eq!(
            *fake.writes.borrow(),
            writes,
            "unchanged values aren't rewritten"
        );
        v.set("conn:a", "").unwrap();
        assert_eq!(v.get("conn:a").unwrap(), None);
    }

    #[test]
    fn after_deny_the_keychain_is_not_asked_again_this_launch() {
        let fake = Fake {
            deny: true,
            ..Default::default()
        };
        let mut v = vault(&fake);
        assert!(v.get("conn:a").is_err());
        assert!(v.get("api-key:openai").is_err());
        assert!(v.set("conn:b", "pw").is_err());
        assert_eq!(*fake.reads.borrow(), 1, "one prompt, not one per secret");
        assert_eq!(*fake.writes.borrow(), 0);
    }

    #[test]
    fn missing_secrets_are_looked_up_in_old_storage_once() {
        let fake = Fake::default();
        let mut v = vault(&fake);
        for _ in 0..3 {
            assert_eq!(v.get("api-key:groq").unwrap(), None);
        }
        // The shared item, then the two old locations, once.
        assert_eq!(*fake.reads.borrow(), 3);
    }

    #[test]
    fn an_unreadable_item_is_never_overwritten() {
        let fake = Fake::default();
        fake.items
            .borrow_mut()
            .insert((service().into(), VAULT_ACCOUNT.into()), "not json".into());
        let mut v = vault(&fake);
        assert!(v.set("conn:a", "pw").is_err());
        assert_eq!(
            fake.items.borrow()[&(service().to_string(), VAULT_ACCOUNT.to_string())],
            "not json"
        );
    }

    #[test]
    fn removing_a_secret_also_removes_the_old_items() {
        let fake = Fake::default();
        fake.items
            .borrow_mut()
            .insert((service().into(), "conn:a".into()), "old-a".into());
        fake.items
            .borrow_mut()
            .insert((LEGACY_SERVICE.into(), "conn:a".into()), "older-a".into());

        let mut v = vault(&fake);
        assert_eq!(v.get("conn:a").unwrap().as_deref(), Some("old-a"));
        v.remove("conn:a").unwrap();
        assert_eq!(v.get("conn:a").unwrap(), None);
        // Only the shared item is left, and a new launch doesn't copy the secret back.
        assert_eq!(fake.items.borrow().len(), 1);
        assert_eq!(vault(&fake).get("conn:a").unwrap(), None);
    }

    #[test]
    fn removing_a_secret_that_was_never_saved_is_fine() {
        let fake = Fake::default();
        vault(&fake).remove("conn:none").unwrap();
    }

    #[test]
    fn removing_reports_keychain_errors() {
        let fake = Fake {
            deny: true,
            ..Default::default()
        };
        assert!(vault(&fake).remove("conn:a").is_err());
    }
}
