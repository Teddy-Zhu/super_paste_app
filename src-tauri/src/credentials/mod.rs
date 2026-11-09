use std::sync::Mutex;

use keyring_core::{Entry, Error as KeyringError};
use uuid::Uuid;

pub const CREDENTIAL_SERVICE: &str = "com.v2dawn.superpaste";
const REFERENCE_PREFIX: &str = "credential:v1:";

static KEYRING_IO: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialError {
    Unavailable(String),
    InvalidReference,
    Missing,
    Operation(String),
}

impl std::fmt::Display for CredentialError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable(message) | Self::Operation(message) => f.write_str(message),
            Self::InvalidReference => f.write_str("系统凭据引用格式无效"),
            Self::Missing => f.write_str("系统凭据引用对应的凭据不存在"),
        }
    }
}

impl std::error::Error for CredentialError {}

#[derive(Clone, Copy, Debug, Default)]
pub struct CredentialStore;

impl CredentialStore {
    pub fn new() -> Self {
        Self
    }

    pub fn reference_for_new_secret(&self, secret: &str) -> Result<String, CredentialError> {
        let reference = Self::new_reference();
        self.store_secret(&reference, secret)?;
        Ok(reference)
    }

    pub fn new_reference() -> String {
        format!("{REFERENCE_PREFIX}{}", Uuid::new_v4())
    }

    pub fn store_secret(&self, reference: &str, secret: &str) -> Result<(), CredentialError> {
        validate_reference(reference)?;
        let _guard = keyring_guard()?;
        entry(reference)?
            .set_password(secret)
            .map_err(operation_error)?;
        Ok(())
    }

    pub fn get_secret(&self, reference: &str) -> Result<String, CredentialError> {
        validate_reference(reference)?;
        let _guard = keyring_guard()?;
        entry(reference)?
            .get_password()
            .map_err(|error| match error {
                KeyringError::NoEntry => CredentialError::Missing,
                other => operation_error(other),
            })
    }

    pub fn delete_secret(&self, reference: &str) -> Result<(), CredentialError> {
        validate_reference(reference)?;
        let _guard = keyring_guard()?;
        match entry(reference)?.delete_credential() {
            Ok(()) | Err(KeyringError::NoEntry) => Ok(()),
            Err(error) => Err(operation_error(error)),
        }
    }
}

pub fn is_credential_reference(value: &str) -> bool {
    validate_reference(value).is_ok()
}

fn validate_reference(reference: &str) -> Result<(), CredentialError> {
    let Some(id) = reference.strip_prefix(REFERENCE_PREFIX) else {
        return Err(CredentialError::InvalidReference);
    };
    Uuid::parse_str(id)
        .map(|_| ())
        .map_err(|_| CredentialError::InvalidReference)
}

fn keyring_guard() -> Result<std::sync::MutexGuard<'static, ()>, CredentialError> {
    let guard = KEYRING_IO
        .lock()
        .map_err(|_| CredentialError::Unavailable("系统凭据服务锁已损坏".to_owned()))?;
    ensure_default_store()?;
    Ok(guard)
}

fn ensure_default_store() -> Result<(), CredentialError> {
    if keyring_core::get_default_store().is_some() {
        return Ok(());
    }

    #[cfg(target_os = "windows")]
    let store = windows_native_keyring_store::Store::new().map_err(unavailable_error)?;

    #[cfg(target_os = "macos")]
    let store = apple_native_keyring_store::keychain::Store::new().map_err(unavailable_error)?;

    #[cfg(target_os = "linux")]
    let store = zbus_secret_service_keyring_store::Store::new().map_err(unavailable_error)?;

    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    return Err(CredentialError::Unavailable(
        "当前平台没有配置系统凭据后端".to_owned(),
    ));

    #[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
    {
        keyring_core::set_default_store(store);
        Ok(())
    }
}

fn entry(reference: &str) -> Result<Entry, CredentialError> {
    Entry::new(CREDENTIAL_SERVICE, reference).map_err(unavailable_error)
}

fn unavailable_error(error: KeyringError) -> CredentialError {
    CredentialError::Unavailable(format!("系统凭据库不可用: {error}"))
}

fn operation_error(error: KeyringError) -> CredentialError {
    CredentialError::Operation(format!("系统凭据操作失败: {error}"))
}
