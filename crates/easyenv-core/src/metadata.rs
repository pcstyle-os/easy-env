use anyhow::{Result, anyhow};
use rusqlite::{Connection, OptionalExtension, params};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use crate::domain::{
    EnvKey, ProjectRecord, Scope, VarMetadata, canonicalize_existing, default_project_name,
    project_id_for_path,
};

#[derive(Debug, Clone)]
pub struct MetadataStore {
    db_path: PathBuf,
}

impl MetadataStore {
    pub fn new(db_path: impl Into<PathBuf>) -> Self {
        Self {
            db_path: db_path.into(),
        }
    }

    pub fn init(&self) -> Result<()> {
        if let Some(parent) = self.db_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let conn = self.connect()?;
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE IF NOT EXISTS schema_migrations (
                 version INTEGER PRIMARY KEY,
                 applied_at INTEGER NOT NULL
             );
             INSERT OR IGNORE INTO schema_migrations(version, applied_at) VALUES (1, strftime('%s','now'));
             CREATE TABLE IF NOT EXISTS projects (
                 id TEXT PRIMARY KEY,
                 root_path TEXT NOT NULL UNIQUE,
                 name TEXT NOT NULL,
                 created_at INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS vars (
                 scope TEXT NOT NULL CHECK(scope IN ('global', 'project')),
                 key TEXT NOT NULL,
                 project_id TEXT NOT NULL DEFAULT '',
                 profile TEXT NOT NULL,
                 updated_at INTEGER NOT NULL,
                 expires_at INTEGER,
                 PRIMARY KEY (scope, key, project_id, profile)
             );
             CREATE INDEX IF NOT EXISTS idx_projects_root_path ON projects(root_path);
             CREATE INDEX IF NOT EXISTS idx_vars_lookup ON vars(scope, project_id, profile);
             CREATE INDEX IF NOT EXISTS idx_vars_key ON vars(key);
            ",
        )?;
        Ok(())
    }

    fn connect(&self) -> Result<Connection> {
        let conn = Connection::open(&self.db_path)?;
        Ok(conn)
    }

    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    pub fn register_project(&self, root_path: &Path) -> Result<ProjectRecord> {
        self.init()?;
        let root_path = canonicalize_existing(root_path)?;
        let root_string = root_path.to_string_lossy().to_string();
        let project = ProjectRecord {
            id: project_id_for_path(&root_path),
            name: default_project_name(&root_path),
            root_path: root_path.clone(),
            created_at: current_timestamp(),
        };

        let conn = self.connect()?;
        conn.execute(
            "INSERT OR IGNORE INTO projects(id, root_path, name, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![project.id, root_string, project.name, project.created_at],
        )?;

        self.get_project_by_root(&root_path)?
            .ok_or_else(|| anyhow!("failed to register project"))
    }

    pub fn get_project_by_root(&self, root_path: &Path) -> Result<Option<ProjectRecord>> {
        self.init()?;
        let root_path = canonicalize_existing(root_path)?;
        let conn = self.connect()?;
        conn.query_row(
            "SELECT id, root_path, name, created_at FROM projects WHERE root_path = ?1",
            params![root_path.to_string_lossy().to_string()],
            map_project,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn find_project_for_dir(&self, cwd: &Path) -> Result<Option<ProjectRecord>> {
        self.init()?;
        let cwd = canonicalize_existing(cwd)?;
        let mut current = Some(cwd.as_path());

        while let Some(path) = current {
            if let Some(project) = self.get_project_by_root(path)? {
                return Ok(Some(project));
            }
            current = path.parent();
        }

        Ok(None)
    }

    pub fn upsert_var(&self, metadata: &VarMetadata) -> Result<()> {
        self.init()?;
        let conn = self.connect()?;
        conn.execute(
            "INSERT INTO vars(scope, key, project_id, profile, updated_at, expires_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(scope, key, project_id, profile)
             DO UPDATE SET updated_at = excluded.updated_at, expires_at = excluded.expires_at",
            params![
                metadata.scope.to_string(),
                metadata.key.as_str(),
                normalize_project_id(metadata.project_id.as_deref()),
                metadata.profile,
                metadata.updated_at,
                metadata.expires_at,
            ],
        )?;
        Ok(())
    }

    pub fn find_var(
        &self,
        key: &EnvKey,
        scope: Scope,
        project_id: Option<&str>,
        profile: &str,
    ) -> Result<Option<VarMetadata>> {
        self.init()?;
        let conn = self.connect()?;
        conn.query_row(
            "SELECT scope, key, project_id, profile, updated_at, expires_at
             FROM vars
             WHERE scope = ?1 AND key = ?2 AND project_id = ?3 AND profile = ?4",
            params![
                scope.to_string(),
                key.as_str(),
                normalize_project_id(project_id),
                profile
            ],
            map_var,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn find_visible_var(
        &self,
        key: &EnvKey,
        active_project_id: Option<&str>,
        profile: &str,
    ) -> Result<Option<VarMetadata>> {
        if let Some(project_id) = active_project_id {
            if let Some(metadata) = self.find_var(key, Scope::Project, Some(project_id), profile)? {
                return Ok(Some(metadata));
            }
        }

        self.find_var(key, Scope::Global, None, profile)
    }

    pub fn list_scope(
        &self,
        scope: Scope,
        project_id: Option<&str>,
        profile: &str,
    ) -> Result<Vec<VarMetadata>> {
        self.init()?;
        let conn = self.connect()?;
        let mut statement = conn.prepare(
            "SELECT scope, key, project_id, profile, updated_at, expires_at
             FROM vars
             WHERE scope = ?1 AND project_id = ?2 AND profile = ?3
             ORDER BY key ASC",
        )?;

        let rows = statement.query_map(
            params![scope.to_string(), normalize_project_id(project_id), profile],
            map_var,
        )?;

        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn list_visible_vars(
        &self,
        active_project_id: Option<&str>,
        profile: &str,
    ) -> Result<Vec<VarMetadata>> {
        let globals = self.list_scope(Scope::Global, None, profile)?;
        let mut merged: BTreeMap<String, VarMetadata> = globals
            .into_iter()
            .map(|metadata| (metadata.key.as_str().to_string(), metadata))
            .collect();

        if let Some(project_id) = active_project_id {
            for metadata in self.list_scope(Scope::Project, Some(project_id), profile)? {
                merged.insert(metadata.key.as_str().to_string(), metadata);
            }
        }

        Ok(merged.into_values().collect())
    }
}

fn normalize_project_id(project_id: Option<&str>) -> String {
    project_id.unwrap_or_default().to_string()
}

fn current_timestamp() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock drifted before unix epoch")
        .as_secs() as i64
}

fn map_project(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProjectRecord> {
    Ok(ProjectRecord {
        id: row.get(0)?,
        root_path: PathBuf::from(row.get::<_, String>(1)?),
        name: row.get(2)?,
        created_at: row.get(3)?,
    })
}

fn map_var(row: &rusqlite::Row<'_>) -> rusqlite::Result<VarMetadata> {
    let project_id: String = row.get(2)?;
    let key: String = row.get(1)?;
    Ok(VarMetadata {
        scope: row.get::<_, String>(0)?.parse::<Scope>().map_err(|err| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    err.to_string(),
                )),
            )
        })?,
        key: EnvKey::parse(key).map_err(|err| {
            rusqlite::Error::FromSqlConversionFailure(
                1,
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    err.to_string(),
                )),
            )
        })?,
        project_id: if project_id.is_empty() {
            None
        } else {
            Some(project_id)
        },
        profile: row.get(3)?,
        updated_at: row.get(4)?,
        expires_at: row.get(5)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn registers_and_finds_projects() {
        let temp = tempdir().unwrap();
        let project_root = temp.path().join("repo");
        fs::create_dir_all(project_root.join("nested")).unwrap();

        let store = MetadataStore::new(temp.path().join("easyenv.sqlite"));
        let project = store.register_project(&project_root).unwrap();
        let detected = store
            .find_project_for_dir(&project_root.join("nested"))
            .unwrap()
            .unwrap();
        assert_eq!(detected.id, project.id);
    }

    #[test]
    fn project_scope_overrides_global_scope() {
        let temp = tempdir().unwrap();
        let project_root = temp.path().join("repo");
        fs::create_dir_all(&project_root).unwrap();

        let store = MetadataStore::new(temp.path().join("easyenv.sqlite"));
        let project = store.register_project(&project_root).unwrap();
        let key = EnvKey::parse("OPENAI_API_KEY").unwrap();

        store
            .upsert_var(&VarMetadata {
                key: key.clone(),
                scope: Scope::Global,
                project_id: None,
                profile: "default".into(),
                updated_at: 1,
                expires_at: None,
            })
            .unwrap();

        store
            .upsert_var(&VarMetadata {
                key: key.clone(),
                scope: Scope::Project,
                project_id: Some(project.id.clone()),
                profile: "default".into(),
                updated_at: 2,
                expires_at: None,
            })
            .unwrap();

        let visible = store
            .find_visible_var(&key, Some(&project.id), "default")
            .unwrap()
            .unwrap();
        assert_eq!(visible.scope, Scope::Project);
    }
}
