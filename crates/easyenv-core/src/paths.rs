use anyhow::{Result, anyhow};
use directories::BaseDirs;
use std::{
    env, fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppPaths {
    pub data_dir: PathBuf,
    pub db_path: PathBuf,
    pub test_secrets_dir: PathBuf,
}

impl AppPaths {
    pub fn detect() -> Result<Self> {
        if let Some(override_dir) = env::var_os("EASYENV_HOME") {
            return Ok(Self::from_data_dir(PathBuf::from(override_dir)));
        }

        let base_dirs =
            BaseDirs::new().ok_or_else(|| anyhow!("failed to detect platform directories"))?;
        Ok(Self::from_data_dir(base_dirs.data_dir().join("easyenv")))
    }

    pub fn from_data_dir(data_dir: impl Into<PathBuf>) -> Self {
        let data_dir = data_dir.into();
        Self {
            db_path: data_dir.join("easyenv.sqlite"),
            test_secrets_dir: data_dir.join("test-secrets"),
            data_dir,
        }
    }

    pub fn ensure(&self) -> Result<()> {
        fs::create_dir_all(&self.data_dir)?;
        Ok(())
    }

    pub fn db_dir(&self) -> &Path {
        self.db_path.parent().unwrap_or(&self.data_dir)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_db_path_from_data_dir() {
        let paths = AppPaths::from_data_dir("/tmp/easyenv-test");
        assert_eq!(
            paths.db_path,
            PathBuf::from("/tmp/easyenv-test/easyenv.sqlite")
        );
    }
}
