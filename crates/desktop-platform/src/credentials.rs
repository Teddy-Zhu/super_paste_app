use std::sync::{Arc, Mutex};
use async_trait::async_trait;
use keyring_core::{CredentialStore, Entry};
use super_paste_core::{Error, Result, credentials::{CredentialBackend, CredentialReference, Secret}};
use super_paste_format::APPLICATION_ID;
use tokio::runtime::Handle;
use zeroize::Zeroize;

/// Uses an explicit native store, never a process-global fallback or file/mock implementation.
pub struct SystemCredentials { runtime: Handle, inner: Arc<NativeStore>, gate: Arc<tokio::sync::Mutex<()>> }
struct NativeStore { store: Mutex<Option<Arc<CredentialStore>>> }
impl SystemCredentials {
    pub fn new(runtime: Handle) -> Arc<Self> { Arc::new(Self { runtime, inner: Arc::new(NativeStore { store: Mutex::new(None) }), gate: Arc::new(tokio::sync::Mutex::new(())) }) }
}
impl NativeStore {
    fn with_entry<T>(&self, reference: CredentialReference, operation: impl FnOnce(Entry) -> Result<T>) -> Result<T> {
        let mut guard = self.store.lock().map_err(|_| Error::CredentialUnavailable)?;
        if guard.is_none() { *guard = Some(open_native_store()?); }
        let store = guard.as_ref().ok_or(Error::CredentialUnavailable)?;
        let entry = store.build(APPLICATION_ID, &reference.account(), None).map_err(classify_error)?;
        operation(entry)
    }
}
#[async_trait]
impl CredentialBackend for SystemCredentials {
    async fn read(&self, reference: CredentialReference) -> Result<Option<Arc<Secret>>> {
        let permit = self.gate.clone().lock_owned().await;
        let store = self.inner.clone();
        self.runtime.spawn_blocking(move || { let _permit = permit; store.with_entry(reference, |entry| {
            match entry.get_secret() {
                Ok(bytes) => Ok(Some(Arc::new(Secret::from_bytes(bytes)?))),
                Err(keyring_core::Error::NoEntry) => Ok(None),
                Err(error) => Err(classify_error(error)),
            }
        }) }).await?
    }
    async fn write(&self, reference: CredentialReference, secret: Arc<Secret>) -> Result<()> {
        let permit = self.gate.clone().lock_owned().await;
        let store = self.inner.clone();
        self.runtime.spawn_blocking(move || { let _permit = permit; store.with_entry(reference, |entry| {
            entry.set_secret(secret.expose().as_bytes()).map_err(classify_error)
        }) }).await?
    }
    async fn delete(&self, reference: CredentialReference) -> Result<()> {
        let permit = self.gate.clone().lock_owned().await;
        let store = self.inner.clone();
        self.runtime.spawn_blocking(move || { let _permit = permit; store.with_entry(reference, |entry| match entry.delete_credential() {
            Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(()),
            Err(error) => Err(classify_error(error)),
        }) }).await?
    }
}

fn open_native_store() -> Result<Arc<CredentialStore>> {
    #[cfg(target_os = "linux")]
    { zbus_secret_service_keyring_store::Store::new().map(|store| store as Arc<CredentialStore>).map_err(classify_error) }
    #[cfg(target_os = "windows")]
    { windows_native_keyring_store::Store::new().map(|store| store as Arc<CredentialStore>).map_err(classify_error) }
    #[cfg(target_os = "macos")]
    { apple_native_keyring_store::keychain::Store::new().map(|store| store as Arc<CredentialStore>).map_err(classify_error) }
    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    { Err(Error::CredentialUnavailable) }
}
fn classify_error(error: keyring_core::Error) -> Error {
    // Several upstream error variants contain raw secret bytes. Never format or log them.
    match error {
        keyring_core::Error::BadEncoding(mut bytes) | keyring_core::Error::BadDataFormat(mut bytes, _) => {
            bytes.zeroize(); Error::CredentialEncoding
        }
        keyring_core::Error::NoStorageAccess(_) | keyring_core::Error::NoDefaultStore => Error::CredentialUnavailable,
        _ => Error::CredentialOperation,
    }
}
