use anyhow::Result;
use easyenv_core::{AppPaths, SecretLocator, SecretStore};
use sha2::{Digest, Sha256};
use std::{env, fs, path::PathBuf};

#[derive(Debug, Clone)]
pub enum ConfiguredSecretStore {
    Native(NativeSecretStore),
    File(FileSecretStore),
}

impl ConfiguredSecretStore {
    pub fn from_env(paths: &AppPaths) -> Self {
        match env::var("EASYENV_SECRET_BACKEND").ok().as_deref() {
            Some("file") => Self::File(FileSecretStore::new(paths.test_secrets_dir.clone())),
            _ => Self::Native(NativeSecretStore),
        }
    }
}

impl SecretStore for ConfiguredSecretStore {
    fn put(&self, locator: &SecretLocator, secret: &[u8]) -> Result<()> {
        match self {
            Self::Native(store) => store.put(locator, secret),
            Self::File(store) => store.put(locator, secret),
        }
    }

    fn get(&self, locator: &SecretLocator) -> Result<Option<Vec<u8>>> {
        match self {
            Self::Native(store) => store.get(locator),
            Self::File(store) => store.get(locator),
        }
    }

    fn delete(&self, locator: &SecretLocator) -> Result<()> {
        match self {
            Self::Native(store) => store.delete(locator),
            Self::File(store) => store.delete(locator),
        }
    }

    fn probe(&self) -> Result<()> {
        match self {
            Self::Native(store) => store.probe(),
            Self::File(store) => store.probe(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct NativeSecretStore;

impl SecretStore for NativeSecretStore {
    fn put(&self, locator: &SecretLocator, secret: &[u8]) -> Result<()> {
        platform::put(locator, secret)
    }

    fn get(&self, locator: &SecretLocator) -> Result<Option<Vec<u8>>> {
        platform::get(locator)
    }

    fn delete(&self, locator: &SecretLocator) -> Result<()> {
        platform::delete(locator)
    }

    fn probe(&self) -> Result<()> {
        let locator = SecretLocator::doctor_probe(&probe_suffix());
        let payload = b"easyenv-probe";
        self.put(&locator, payload)?;
        let loaded = self.get(&locator)?;
        self.delete(&locator)?;
        match loaded {
            Some(bytes) if bytes == payload => Ok(()),
            Some(_) => anyhow::bail!("secret store returned a different payload"),
            None => anyhow::bail!("secret store probe item was not readable"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct FileSecretStore {
    root: PathBuf,
}

impl FileSecretStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn file_path(&self, locator: &SecretLocator) -> PathBuf {
        let mut hasher = Sha256::new();
        hasher.update(locator.service.as_bytes());
        hasher.update([0]);
        hasher.update(locator.account.as_bytes());
        let digest = hasher.finalize();
        self.root.join(format!("{}.secret", hex::encode(digest)))
    }
}

impl SecretStore for FileSecretStore {
    fn put(&self, locator: &SecretLocator, secret: &[u8]) -> Result<()> {
        fs::create_dir_all(&self.root)?;
        fs::write(self.file_path(locator), secret)?;
        Ok(())
    }

    fn get(&self, locator: &SecretLocator) -> Result<Option<Vec<u8>>> {
        let path = self.file_path(locator);
        if !path.exists() {
            return Ok(None);
        }
        Ok(Some(fs::read(path)?))
    }

    fn delete(&self, locator: &SecretLocator) -> Result<()> {
        let path = self.file_path(locator);
        if path.exists() {
            fs::remove_file(path)?;
        }
        Ok(())
    }

    fn probe(&self) -> Result<()> {
        fs::create_dir_all(&self.root)?;
        let locator = SecretLocator::doctor_probe(&probe_suffix());
        let path = self.file_path(&locator);
        fs::write(&path, b"easyenv-probe")?;
        let loaded = fs::read(&path)?;
        fs::remove_file(path)?;
        if loaded == b"easyenv-probe" {
            Ok(())
        } else {
            anyhow::bail!("filesystem secret probe returned a different payload")
        }
    }
}

fn probe_suffix() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock drifted before unix epoch")
        .as_nanos();
    format!("{}-{}", std::process::id(), timestamp)
}

#[cfg(target_os = "macos")]
mod platform {
    use anyhow::{Context, Result};
    use easyenv_core::SecretLocator;
    use security_framework::passwords::{
        PasswordOptions, delete_generic_password_options, generic_password,
        set_generic_password_options,
    };
    use security_framework_sys::base::errSecItemNotFound;
    use std::env;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum GlobalSyncMode {
        Prefer,
        Force,
        Never,
    }

    #[derive(Debug, Clone)]
    struct KeychainConfig {
        access_group: Option<String>,
        global_sync: GlobalSyncMode,
    }

    impl KeychainConfig {
        fn from_env() -> Self {
            let access_group = env::var("EASYENV_KEYCHAIN_ACCESS_GROUP")
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty());
            let global_sync = match env::var("EASYENV_KEYCHAIN_GLOBAL_SYNC")
                .ok()
                .unwrap_or_else(|| "prefer".to_string())
                .trim()
                .to_ascii_lowercase()
                .as_str()
            {
                "force" | "always" => GlobalSyncMode::Force,
                "never" | "off" | "false" | "local" => GlobalSyncMode::Never,
                _ => GlobalSyncMode::Prefer,
            };
            Self {
                access_group,
                global_sync,
            }
        }
    }

    pub fn put(locator: &SecretLocator, secret: &[u8]) -> Result<()> {
        let config = KeychainConfig::from_env();
        let mut last_error = None;

        for sync in sync_candidates(locator, config.global_sync) {
            match put_with(locator, secret, sync, &config) {
                Ok(()) => {
                    cleanup_opposite_store(locator, sync, &config);
                    return Ok(());
                }
                Err(error) => last_error = Some(error),
            }
        }

        Err(last_error.expect("at least one sync candidate exists"))
    }

    pub fn get(locator: &SecretLocator) -> Result<Option<Vec<u8>>> {
        let config = KeychainConfig::from_env();
        let mut last_error = None;

        for sync in sync_candidates(locator, config.global_sync) {
            match get_with(locator, sync, &config) {
                Ok(Some(secret)) => return Ok(Some(secret)),
                Ok(None) => {}
                Err(error) => last_error = Some(error),
            }
        }

        match last_error {
            Some(error) if config.global_sync == GlobalSyncMode::Force || prefers_sync(locator) => {
                Err(error)
            }
            Some(_) | None => Ok(None),
        }
    }

    pub fn delete(locator: &SecretLocator) -> Result<()> {
        let config = KeychainConfig::from_env();
        let mut deleted_any = false;
        let mut last_error = None;

        for sync in sync_candidates(locator, config.global_sync) {
            match delete_with(locator, sync, &config) {
                Ok(()) => deleted_any = true,
                Err(error) => last_error = Some(error),
            }
        }

        if deleted_any || last_error.is_none() {
            Ok(())
        } else {
            Err(last_error.expect("error captured above"))
        }
    }

    fn sync_candidates(locator: &SecretLocator, mode: GlobalSyncMode) -> Vec<bool> {
        if !prefers_sync(locator) {
            return vec![false];
        }

        match mode {
            GlobalSyncMode::Force => vec![true],
            GlobalSyncMode::Never => vec![false],
            GlobalSyncMode::Prefer => vec![true, false],
        }
    }

    fn prefers_sync(locator: &SecretLocator) -> bool {
        locator.service == "dev.easyenv.global"
    }

    fn cleanup_opposite_store(locator: &SecretLocator, used_sync: bool, config: &KeychainConfig) {
        if prefers_sync(locator) {
            let _ = delete_with(locator, !used_sync, config);
        }
    }

    fn password_options(
        locator: &SecretLocator,
        sync: bool,
        config: &KeychainConfig,
    ) -> PasswordOptions {
        let mut options = PasswordOptions::new_generic_password(&locator.service, &locator.account);
        options.set_label(&locator.label);
        options.set_access_synchronized(Some(sync));
        if let Some(access_group) = config.access_group.as_deref() {
            options.set_access_group(access_group);
        }
        options
    }

    fn put_with(
        locator: &SecretLocator,
        secret: &[u8],
        sync: bool,
        config: &KeychainConfig,
    ) -> Result<()> {
        let options = password_options(locator, sync, config);
        set_generic_password_options(secret, options).with_context(|| {
            format!(
                "failed to write keychain item {}:{} (sync={sync})",
                locator.service, locator.account
            )
        })?;
        Ok(())
    }

    fn get_with(
        locator: &SecretLocator,
        sync: bool,
        config: &KeychainConfig,
    ) -> Result<Option<Vec<u8>>> {
        let options = password_options(locator, sync, config);
        match generic_password(options) {
            Ok(secret) => Ok(Some(secret)),
            Err(error) if error.code() == errSecItemNotFound => Ok(None),
            Err(error) => Err(error).with_context(|| {
                format!(
                    "failed to read keychain item {}:{} (sync={sync})",
                    locator.service, locator.account
                )
            }),
        }
    }

    fn delete_with(locator: &SecretLocator, sync: bool, config: &KeychainConfig) -> Result<()> {
        let options = password_options(locator, sync, config);
        match delete_generic_password_options(options) {
            Ok(()) => Ok(()),
            Err(error) if error.code() == errSecItemNotFound => Ok(()),
            Err(error) => Err(error).with_context(|| {
                format!(
                    "failed to delete keychain item {}:{} (sync={sync})",
                    locator.service, locator.account
                )
            }),
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    use anyhow::{Context, Result};
    use easyenv_core::SecretLocator;
    use keyring::{Entry, Error};

    pub fn put(locator: &SecretLocator, secret: &[u8]) -> Result<()> {
        let entry = Entry::new(&locator.service, &locator.account)?;
        entry.set_secret(secret).with_context(|| {
            format!(
                "failed to write native credential {}:{}",
                locator.service, locator.account
            )
        })?;
        Ok(())
    }

    pub fn get(locator: &SecretLocator) -> Result<Option<Vec<u8>>> {
        let entry = Entry::new(&locator.service, &locator.account)?;
        match entry.get_secret() {
            Ok(secret) => Ok(Some(secret)),
            Err(Error::NoEntry) => Ok(None),
            Err(error) => Err(error).with_context(|| {
                format!(
                    "failed to read native credential {}:{}",
                    locator.service, locator.account
                )
            }),
        }
    }

    pub fn delete(locator: &SecretLocator) -> Result<()> {
        let entry = Entry::new(&locator.service, &locator.account)?;
        match entry.delete_credential() {
            Ok(()) | Err(Error::NoEntry) => Ok(()),
            Err(error) => Err(error).with_context(|| {
                format!(
                    "failed to delete native credential {}:{}",
                    locator.service, locator.account
                )
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn file_store_roundtrips() {
        let temp = tempdir().unwrap();
        let store = FileSecretStore::new(temp.path());
        let locator = SecretLocator::doctor_probe("test");
        store.put(&locator, b"hello").unwrap();
        assert_eq!(store.get(&locator).unwrap().unwrap(), b"hello");
        store.delete(&locator).unwrap();
        assert!(store.get(&locator).unwrap().is_none());
    }
}
