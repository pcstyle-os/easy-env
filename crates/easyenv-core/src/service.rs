use anyhow::{Context, Result, anyhow, bail};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};
use zeroize::Zeroizing;

use crate::{
    AppPaths, EnvKey, MetadataStore, ProjectRecord, Scope, SecretLocator, SecretStore, VarMetadata,
    crypto, dotenv,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DesiredScope {
    Auto,
    Global,
    Project,
}

#[derive(Debug)]
pub struct ResolvedSecret {
    pub key: EnvKey,
    pub value: Zeroizing<String>,
    pub scope: Scope,
    pub project_root: Option<PathBuf>,
    pub profile: String,
}

#[derive(Debug, Serialize)]
pub struct ImportOutcome {
    pub imported: usize,
    pub deleted_source: bool,
    pub scope: Scope,
    pub project_root: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Pass,
    Warn,
    Fail,
}

#[derive(Debug, Clone, Serialize)]
pub struct DoctorCheck {
    pub name: String,
    pub status: CheckStatus,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DoctorReport {
    pub data_dir: PathBuf,
    pub db_path: PathBuf,
    pub active_project: Option<PathBuf>,
    pub checks: Vec<DoctorCheck>,
}

#[derive(Debug, Clone)]
pub struct EasyEnv<S> {
    pub paths: AppPaths,
    metadata: MetadataStore,
    store: S,
}

impl<S: SecretStore> EasyEnv<S> {
    pub fn new(paths: AppPaths, store: S) -> Result<Self> {
        paths.ensure()?;
        let metadata = MetadataStore::new(paths.db_path.clone());
        metadata.init()?;
        Ok(Self {
            paths,
            metadata,
            store,
        })
    }

    pub fn init_project(&self, cwd: &Path) -> Result<ProjectRecord> {
        self.metadata.register_project(cwd)
    }

    pub fn active_project(&self, cwd: &Path) -> Result<Option<ProjectRecord>> {
        self.metadata.find_project_for_dir(cwd)
    }

    pub fn set_secret(
        &self,
        cwd: &Path,
        desired_scope: DesiredScope,
        profile: &str,
        key: EnvKey,
        value: &str,
        expires_at: Option<i64>,
    ) -> Result<ResolvedSecret> {
        let (scope, project) = self.resolve_scope(cwd, desired_scope)?;
        let profile = normalize_profile(profile)?;
        let metadata = VarMetadata {
            key: key.clone(),
            scope,
            project_id: project.as_ref().map(|project| project.id.clone()),
            profile: profile.clone(),
            updated_at: current_timestamp(),
            expires_at,
        };
        let locator = SecretLocator::for_var(&metadata)?;
        let master_key = self.ensure_master_key()?;
        let encrypted = crypto::encrypt(master_key.as_ref(), value.as_bytes())?;
        self.store.put(&locator, &encrypted)?;
        self.metadata.upsert_var(&metadata)?;

        Ok(ResolvedSecret {
            key,
            value: Zeroizing::new(value.to_string()),
            scope,
            project_root: project.map(|project| project.root_path),
            profile,
        })
    }

    pub fn get_secret(
        &self,
        cwd: &Path,
        key: &EnvKey,
        profile: &str,
    ) -> Result<Option<ResolvedSecret>> {
        let profile = normalize_profile(profile)?;
        let project = self.active_project(cwd)?;
        let metadata = self.metadata.find_visible_var(
            key,
            project.as_ref().map(|project| project.id.as_str()),
            &profile,
        )?;

        match metadata {
            Some(metadata) => self.load_resolved_secret(metadata, project),
            None => Ok(None),
        }
    }

    pub fn list_secrets(
        &self,
        cwd: &Path,
        scope_filter: Option<Scope>,
        profile: &str,
    ) -> Result<Vec<ResolvedSecret>> {
        let profile = normalize_profile(profile)?;
        let project = self.active_project(cwd)?;
        let metadata_items = match scope_filter {
            Some(Scope::Global) => self.metadata.list_scope(Scope::Global, None, &profile)?,
            Some(Scope::Project) => {
                let Some(project) = project.as_ref() else {
                    return Ok(Vec::new());
                };
                self.metadata
                    .list_scope(Scope::Project, Some(&project.id), &profile)?
            }
            Some(Scope::Shell) => Vec::new(),
            None => self.metadata.list_visible_vars(
                project.as_ref().map(|project| project.id.as_str()),
                &profile,
            )?,
        };

        let mut resolved = Vec::with_capacity(metadata_items.len());
        for metadata in metadata_items {
            let project_root = metadata
                .project_id
                .as_deref()
                .and_then(|project_id| project.as_ref().filter(|project| project.id == project_id))
                .map(|project| project.root_path.clone());
            if let Some(secret) = self.load_secret_from_metadata(metadata, project_root)? {
                resolved.push(secret);
            }
        }
        Ok(resolved)
    }

    pub fn resolve_environment(
        &self,
        cwd: &Path,
        profile: &str,
        shell_overrides: &[(EnvKey, String)],
    ) -> Result<Vec<ResolvedSecret>> {
        let mut merged: BTreeMap<String, ResolvedSecret> = self
            .list_secrets(cwd, None, profile)?
            .into_iter()
            .map(|secret| (secret.key.as_str().to_string(), secret))
            .collect();

        for (key, value) in shell_overrides {
            merged.insert(
                key.as_str().to_string(),
                ResolvedSecret {
                    key: key.clone(),
                    value: Zeroizing::new(value.clone()),
                    scope: Scope::Shell,
                    project_root: None,
                    profile: profile.to_string(),
                },
            );
        }

        Ok(merged.into_values().collect())
    }

    pub fn import_dotenv(
        &self,
        cwd: &Path,
        source: &Path,
        desired_scope: DesiredScope,
        profile: &str,
        delete_source: bool,
    ) -> Result<ImportOutcome> {
        let contents = fs::read_to_string(source)
            .with_context(|| format!("failed to read {}", source.display()))?;
        let entries = dotenv::parse_dotenv(&contents)?;
        let (scope, project) = self.resolve_scope(cwd, desired_scope)?;
        let project_root = project.as_ref().map(|project| project.root_path.clone());

        for (key, value) in &entries {
            self.set_secret(cwd, desired_scope, profile, key.clone(), value, None)?;
        }

        if delete_source {
            fs::remove_file(source)
                .with_context(|| format!("failed to delete {}", source.display()))?;
        }

        Ok(ImportOutcome {
            imported: entries.len(),
            deleted_source: delete_source,
            scope,
            project_root,
        })
    }

    pub fn doctor(&self, cwd: &Path) -> Result<DoctorReport> {
        let mut checks = Vec::new();

        self.paths.ensure()?;
        checks.push(DoctorCheck {
            name: "paths".into(),
            status: CheckStatus::Pass,
            message: format!("using {}", self.paths.data_dir.display()),
        });

        self.metadata.init()?;
        checks.push(DoctorCheck {
            name: "sqlite".into(),
            status: CheckStatus::Pass,
            message: format!(
                "metadata database ready at {}",
                self.metadata.db_path().display()
            ),
        });

        match self.store.probe() {
            Ok(()) => checks.push(DoctorCheck {
                name: "secret_store".into(),
                status: CheckStatus::Pass,
                message: "secret store roundtrip succeeded".into(),
            }),
            Err(error) => checks.push(DoctorCheck {
                name: "secret_store".into(),
                status: CheckStatus::Fail,
                message: error.to_string(),
            }),
        }

        let active_project = self.active_project(cwd)?;
        checks.push(DoctorCheck {
            name: "project".into(),
            status: if active_project.is_some() {
                CheckStatus::Pass
            } else {
                CheckStatus::Warn
            },
            message: active_project
                .as_ref()
                .map(|project| format!("active project: {}", project.root_path.display()))
                .unwrap_or_else(|| "no registered project for current directory".into()),
        });

        checks.push(DoctorCheck {
            name: "icloud".into(),
            status: CheckStatus::Warn,
            message: "MVP stores secrets in the OS keychain; synchronizable iCloud attributes are not yet verified by doctor".into(),
        });

        Ok(DoctorReport {
            data_dir: self.paths.data_dir.clone(),
            db_path: self.paths.db_path.clone(),
            active_project: active_project.map(|project| project.root_path),
            checks,
        })
    }

    fn resolve_scope(
        &self,
        cwd: &Path,
        desired_scope: DesiredScope,
    ) -> Result<(Scope, Option<ProjectRecord>)> {
        let project = self.active_project(cwd)?;
        match desired_scope {
            DesiredScope::Auto => {
                if project.is_some() {
                    Ok((Scope::Project, project))
                } else {
                    Ok((Scope::Global, None))
                }
            }
            DesiredScope::Global => Ok((Scope::Global, None)),
            DesiredScope::Project => {
                let project = project
                    .ok_or_else(|| anyhow!("no initialized project found for {}", cwd.display()))?;
                Ok((Scope::Project, Some(project)))
            }
        }
    }

    fn ensure_master_key(&self) -> Result<Zeroizing<Vec<u8>>> {
        let locator = SecretLocator::master_key();
        if let Some(existing) = self.store.get(&locator)? {
            if existing.len() != 32 {
                bail!("stored master key has invalid length");
            }
            return Ok(Zeroizing::new(existing));
        }

        let master_key = crypto::generate_key()?;
        self.store.put(&locator, master_key.as_ref())?;
        Ok(master_key)
    }

    fn load_resolved_secret(
        &self,
        metadata: VarMetadata,
        active_project: Option<ProjectRecord>,
    ) -> Result<Option<ResolvedSecret>> {
        let project_root = metadata
            .project_id
            .as_deref()
            .and_then(|project_id| {
                active_project
                    .as_ref()
                    .filter(|project| project.id == project_id)
            })
            .map(|project| project.root_path.clone());
        self.load_secret_from_metadata(metadata, project_root)
    }

    fn load_secret_from_metadata(
        &self,
        metadata: VarMetadata,
        project_root: Option<PathBuf>,
    ) -> Result<Option<ResolvedSecret>> {
        let locator = SecretLocator::for_var(&metadata)?;
        let Some(encrypted) = self.store.get(&locator)? else {
            return Ok(None);
        };
        let master_key = self.ensure_master_key()?;
        let decrypted = crypto::decrypt(master_key.as_ref(), &encrypted)?;
        let value =
            String::from_utf8(decrypted.to_vec()).context("stored secret is not valid UTF-8")?;
        Ok(Some(ResolvedSecret {
            key: metadata.key,
            value: Zeroizing::new(value),
            scope: metadata.scope,
            project_root,
            profile: metadata.profile,
        }))
    }
}

pub fn mask_value(value: &str) -> String {
    if value.len() <= 4 {
        return "****".into();
    }

    let prefix = &value[..2];
    let suffix = &value[value.len() - 2..];
    format!("{prefix}***{suffix}")
}

fn normalize_profile(profile: &str) -> Result<String> {
    let profile = profile.trim();
    if profile.is_empty() {
        bail!("profile cannot be empty");
    }
    Ok(profile.to_string())
}

fn current_timestamp() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock drifted before unix epoch")
        .as_secs() as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        collections::HashMap,
        sync::{Arc, Mutex},
    };
    use tempfile::tempdir;

    #[derive(Clone, Default)]
    struct MemoryStore {
        values: Arc<Mutex<HashMap<(String, String), Vec<u8>>>>,
    }

    impl SecretStore for MemoryStore {
        fn put(&self, locator: &SecretLocator, secret: &[u8]) -> Result<()> {
            self.values.lock().unwrap().insert(
                (locator.service.clone(), locator.account.clone()),
                secret.to_vec(),
            );
            Ok(())
        }

        fn get(&self, locator: &SecretLocator) -> Result<Option<Vec<u8>>> {
            Ok(self
                .values
                .lock()
                .unwrap()
                .get(&(locator.service.clone(), locator.account.clone()))
                .cloned())
        }

        fn delete(&self, locator: &SecretLocator) -> Result<()> {
            self.values
                .lock()
                .unwrap()
                .remove(&(locator.service.clone(), locator.account.clone()));
            Ok(())
        }

        fn probe(&self) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn resolves_project_scope_over_global_scope() {
        let temp = tempdir().unwrap();
        let project_root = temp.path().join("repo");
        fs::create_dir_all(&project_root).unwrap();
        let nested = project_root.join("nested");
        fs::create_dir_all(&nested).unwrap();

        let paths = crate::AppPaths::from_data_dir(temp.path().join("data"));
        let app = EasyEnv::new(paths, MemoryStore::default()).unwrap();
        app.init_project(&project_root).unwrap();

        app.set_secret(
            temp.path(),
            DesiredScope::Global,
            "default",
            EnvKey::parse("API_KEY").unwrap(),
            "global",
            None,
        )
        .unwrap();
        app.set_secret(
            &project_root,
            DesiredScope::Project,
            "default",
            EnvKey::parse("API_KEY").unwrap(),
            "project",
            None,
        )
        .unwrap();

        let resolved = app
            .get_secret(&nested, &EnvKey::parse("API_KEY").unwrap(), "default")
            .unwrap()
            .unwrap();
        assert_eq!(resolved.scope, Scope::Project);
        assert_eq!(resolved.value.as_str(), "project");
    }

    #[test]
    fn masks_values_for_human_output() {
        assert_eq!(mask_value("abcd"), "****");
        assert_eq!(mask_value("abcdefgh"), "ab***gh");
    }
}
