use std::{collections::{BTreeMap, BTreeSet}, fmt, sync::Arc};
use async_trait::async_trait;
use super_paste_format::CredentialId;
use uuid::Uuid;
use zeroize::{Zeroize, Zeroizing};
use crate::{Error, Result, storage::Storage};

/// Intentionally has no Serialize, Display, or public backing-buffer access.
pub struct Secret(Zeroizing<String>);
impl Secret {
    pub fn new(value: String) -> Self { Self(Zeroizing::new(value)) }
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self> {
        match String::from_utf8(bytes) {
            Ok(value) => Ok(Self::new(value)),
            Err(error) => { let mut bytes = error.into_bytes(); bytes.zeroize(); Err(Error::CredentialEncoding) }
        }
    }
    pub fn expose(&self) -> &str { self.0.as_str() }
}
impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str("Secret([redacted])") }
}

/// References are non-secret. A new reference allows save failure to retain the old credential.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CredentialReference(Uuid);
impl CredentialReference {
    pub fn new() -> Self { Self(Uuid::new_v4()) }
    pub fn account(self) -> String { format!("credential:v1:{}", self.0) }
    pub fn parse(value: &str) -> Result<Self> {
        let value = value.strip_prefix("credential:v1:").ok_or_else(|| Error::Invalid("系统凭据引用格式无效".into()))?;
        let id = Uuid::parse_str(value).map_err(|_| Error::Invalid("系统凭据引用格式无效".into()))?;
        Ok(Self(id))
    }
}
impl Default for CredentialReference { fn default() -> Self { Self::new() } }

#[derive(Clone, Debug)]
pub enum SecretChange { Keep, Set(Arc<Secret>), Unset }
#[derive(Clone, Debug)]
pub struct CredentialChange { pub id: CredentialId, pub change: SecretChange }

#[async_trait]
pub trait CredentialBackend: Send + Sync {
    async fn read(&self, reference: CredentialReference) -> Result<Option<Arc<Secret>>>;
    async fn write(&self, reference: CredentialReference, secret: Arc<Secret>) -> Result<()>;
    async fn delete(&self, reference: CredentialReference) -> Result<()>;
}

pub type CredentialReferences = BTreeMap<CredentialId, CredentialReference>;

/// Stores only non-secret references in SQLite. All secret values remain in the native backend.
pub struct CredentialVault {
    storage: Arc<Storage>, backend: Arc<dyn CredentialBackend>, gate: Arc<tokio::sync::Mutex<()>>,
    cleanup: tokio::sync::watch::Sender<Option<Error>>,
}
impl CredentialVault {
    pub fn new(storage: Arc<Storage>, backend: Arc<dyn CredentialBackend>) -> Arc<Self> {
        let (cleanup, _) = tokio::sync::watch::channel(None);
        Arc::new(Self { storage, backend, gate: Arc::new(tokio::sync::Mutex::new(())), cleanup })
    }
    pub fn subscribe_cleanup(&self) -> tokio::sync::watch::Receiver<Option<Error>> { self.cleanup.subscribe() }
    pub(crate) async fn read(&self, reference: CredentialReference) -> Result<Option<Arc<Secret>>> { self.backend.read(reference).await }

    pub(crate) async fn prepare(self: &Arc<Self>, before: &CredentialReferences, changes: &[CredentialChange]) -> Result<PreparedCredentials> {
        let mut after = before.clone();
        let mut affected = false;
        let mut unique = BTreeSet::new();
        for change in changes {
            if !unique.insert(change.id) { return Err(Error::Invalid("同一提交包含重复凭据字段".into())); }
        }
        let needs_store = changes.iter().any(|change| match change.change {
            SecretChange::Set(_) => true, SecretChange::Unset => before.contains_key(&change.id), SecretChange::Keep => false,
        });
        if !needs_store { return Ok(PreparedCredentials { vault: self.clone(), after, affected: false, _guard: None }); }
        let guard = self.gate.clone().lock_owned().await;
        let result: Result<()> = async {
            for change in changes {
                match &change.change {
                    SecretChange::Keep => {}
                    SecretChange::Unset => { affected |= after.remove(&change.id).is_some(); }
                    SecretChange::Set(secret) => {
                        let reference = CredentialReference::new();
                        // Register before touching the OS store, including a potentially partial write.
                        self.storage.track_credential_garbage(reference).await?;
                        affected = true;
                        self.backend.write(reference, secret.clone()).await?;
                        after.insert(change.id, reference);
                    }
                }
            }
            Ok(())
        }.await;
        if let Err(error) = result {
            return Err(combine_cleanup(error, self.collect_inner().await));
        }
        Ok(PreparedCredentials { vault: self.clone(), after, affected, _guard: Some(guard) })
    }
    /// Run during startup or an explicit retry. The staging guard prevents deleting an in-flight save.
    pub async fn collect_garbage(&self) -> Result<()> {
        let _guard = self.gate.lock().await;
        self.collect_inner().await
    }
    async fn collect_inner(&self) -> Result<()> {
        let result = async {
            let references = self.storage.retired_credentials().await?;
            let mut failure = None;
            for reference in references {
                match self.backend.delete(reference).await {
                    Ok(()) => {
                        if let Err(error) = self.storage.forget_retired_credential(reference).await { failure.get_or_insert(error); }
                    }
                    Err(error) => { failure.get_or_insert(error); }
                }
            }
            match failure { Some(error) => Err(error), None => Ok(()) }
        }.await;
        // All cleanup paths publish, including failed staging and transaction
        // rollback. A later successful scan clears the recoverable warning.
        self.cleanup.send_replace(result.as_ref().err().cloned());
        result
    }
}
pub(crate) struct PreparedCredentials {
    vault: Arc<CredentialVault>, pub after: CredentialReferences, affected: bool,
    _guard: Option<tokio::sync::OwnedMutexGuard<()>>,
}
impl PreparedCredentials {
    pub async fn finish(self) -> Result<()> {
        if self.affected { self.vault.collect_inner().await } else { Ok(()) }
    }
    pub async fn abort(self, error: Error) -> Error {
        if self.affected { combine_cleanup(error, self.vault.collect_inner().await) } else { error }
    }
}
fn combine_cleanup(error: Error, cleanup: Result<()>) -> Error {
    match cleanup { Ok(()) => error, Err(cleanup) => Error::Task(format!("{error}；凭据清理待重试：{cleanup}").into()) }
}

pub(crate) fn read_references(connection: &rusqlite::Connection) -> Result<CredentialReferences> {
    let mut statement = connection.prepare("SELECT key,reference FROM credential_refs")?;
    let mut rows = statement.query([])?;
    let mut references = BTreeMap::new();
    while let Some(row) = rows.next()? {
        let key: String = row.get(0)?;
        let id = CredentialId::ALL.iter().copied().find(|id| id.account() == key)
            .ok_or_else(|| Error::Database("数据库包含不支持的凭据字段".into()))?;
        let reference: String = row.get(1)?;
        references.insert(id, CredentialReference::parse(&reference)?);
    }
    Ok(references)
}
impl Storage {
    async fn track_credential_garbage(&self, reference: CredentialReference) -> Result<()> {
        self.call(move |connection| {
            connection.execute("INSERT INTO credential_garbage(reference) VALUES(?1)", [reference.account()])?;
            Ok(())
        }).await
    }
    async fn retired_credentials(&self) -> Result<Vec<CredentialReference>> {
        self.call(|connection| {
            let mut statement = connection.prepare("SELECT reference FROM credential_garbage WHERE NOT EXISTS(SELECT 1 FROM credential_refs WHERE credential_refs.reference=credential_garbage.reference)")?;
            let values = statement.query_map([], |row| row.get::<_, String>(0))?.collect::<std::result::Result<Vec<_>, _>>()?;
            values.into_iter().map(|value| CredentialReference::parse(&value)).collect()
        }).await
    }
    async fn forget_retired_credential(&self, reference: CredentialReference) -> Result<()> {
        self.call(move |connection| {
            connection.execute("DELETE FROM credential_garbage WHERE reference=?1 AND NOT EXISTS(SELECT 1 FROM credential_refs WHERE reference=?1)", [reference.account()])?;
            Ok(())
        }).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{path::PathBuf, sync::{Mutex, atomic::{AtomicBool, Ordering}}};

    #[derive(Default)]
    struct Backend {
        entries: Mutex<BTreeMap<Uuid, Arc<Secret>>>,
        fail_write: AtomicBool,
        fail_delete: AtomicBool,
    }
    #[async_trait]
    impl CredentialBackend for Backend {
        async fn read(&self, reference: CredentialReference) -> Result<Option<Arc<Secret>>> {
            Ok(self.entries.lock().expect("backend lock").get(&reference.0).cloned())
        }
        async fn write(&self, reference: CredentialReference, secret: Arc<Secret>) -> Result<()> {
            self.entries.lock().expect("backend lock").insert(reference.0, secret);
            if self.fail_write.load(Ordering::SeqCst) { Err(Error::CredentialOperation) } else { Ok(()) }
        }
        async fn delete(&self, reference: CredentialReference) -> Result<()> {
            if self.fail_delete.load(Ordering::SeqCst) { return Err(Error::CredentialUnavailable); }
            self.entries.lock().expect("backend lock").remove(&reference.0);
            Ok(())
        }
    }
    struct Directory(PathBuf);
    impl Drop for Directory {
        fn drop(&mut self) {
            if let Err(error) = std::fs::remove_dir_all(&self.0) { eprintln!("failed to remove credential test directory: {error}"); }
        }
    }

    // Use the real garbage registry, including partial native writes and a
    // failed transaction's abort. No system credential store is accessed.
    #[tokio::test]
    async fn failed_staging_and_abort_publish_cleanup_until_retry_recovers() {
        let directory = Directory(std::env::temp_dir().join(format!("super-paste-rs-credentials-{}", Uuid::new_v4())));
        std::fs::create_dir(&directory.0).expect("isolated database directory");
        let storage = Storage::open(&directory.0.join("history.sqlite3")).await.expect("database");
        let backend = Arc::new(Backend::default());
        let vault = CredentialVault::new(storage.clone(), backend.clone());
        let mut updates = vault.subscribe_cleanup();
        assert!(updates.borrow_and_update().is_none());
        let changes = [CredentialChange { id: CredentialId::WebDavPassword, change: SecretChange::Set(Arc::new(Secret::new("fixture".into()))) }];

        backend.fail_write.store(true, Ordering::SeqCst);
        backend.fail_delete.store(true, Ordering::SeqCst);
        assert!(vault.prepare(&CredentialReferences::new(), &changes).await.is_err());
        assert!(updates.has_changed().expect("open status channel"));
        assert!(matches!(updates.borrow_and_update().as_ref(), Some(Error::CredentialUnavailable)));
        assert!(matches!(vault.subscribe_cleanup().borrow().as_ref(), Some(Error::CredentialUnavailable)));
        assert_eq!(storage.retired_credentials().await.expect("partial write registry").len(), 1);
        assert_eq!(backend.entries.lock().expect("backend lock").len(), 1);

        backend.fail_write.store(false, Ordering::SeqCst);
        backend.fail_delete.store(false, Ordering::SeqCst);
        vault.collect_garbage().await.expect("retry partial write cleanup");
        assert!(updates.has_changed().expect("recovery notification"));
        assert!(updates.borrow_and_update().is_none());
        assert!(storage.retired_credentials().await.expect("empty registry").is_empty());
        assert!(backend.entries.lock().expect("backend lock").is_empty());

        let prepared = vault.prepare(&CredentialReferences::new(), &changes).await.expect("stage credential");
        backend.fail_delete.store(true, Ordering::SeqCst);
        let error = prepared.abort(Error::Invalid("save rejected".into())).await;
        assert!(error.to_string().contains("save rejected"));
        assert!(updates.has_changed().expect("abort notification"));
        assert!(matches!(updates.borrow_and_update().as_ref(), Some(Error::CredentialUnavailable)));
        assert_eq!(storage.retired_credentials().await.expect("abort registry").len(), 1);
        backend.fail_delete.store(false, Ordering::SeqCst);
        vault.collect_garbage().await.expect("retry abort cleanup");
        assert!(updates.has_changed().expect("abort recovery notification"));
        assert!(updates.borrow_and_update().is_none());
        assert!(storage.retired_credentials().await.expect("empty abort registry").is_empty());
        assert!(backend.entries.lock().expect("backend lock").is_empty());
        storage.close_for_test().await.expect("close database");
    }
}
