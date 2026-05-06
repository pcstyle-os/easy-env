use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fmt,
    path::{Path, PathBuf},
    str::FromStr,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Ord, PartialOrd)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    Global,
    Project,
    Shell,
}

impl Scope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Project => "project",
            Self::Shell => "shell",
        }
    }

    pub fn stored(self) -> bool {
        !matches!(self, Self::Shell)
    }
}

impl fmt::Display for Scope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Scope {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "global" => Ok(Self::Global),
            "project" => Ok(Self::Project),
            "shell" => Ok(Self::Shell),
            other => bail!("invalid scope: {other}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Ord, PartialOrd)]
pub struct EnvKey(String);

impl EnvKey {
    pub fn parse(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        validate_key(&value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn validate_key(value: &str) -> Result<()> {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        bail!("env key cannot be empty");
    };

    if !(first == '_' || first.is_ascii_alphabetic()) {
        bail!("env key must start with a letter or underscore");
    }

    if !chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric()) {
        bail!("env key may only contain ASCII letters, digits, or underscores");
    }

    Ok(())
}

impl fmt::Display for EnvKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for EnvKey {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectRecord {
    pub id: String,
    pub root_path: PathBuf,
    pub name: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VarMetadata {
    pub key: EnvKey,
    pub scope: Scope,
    pub project_id: Option<String>,
    pub profile: String,
    pub updated_at: i64,
    pub expires_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretLocator {
    pub service: String,
    pub account: String,
    pub label: String,
}

impl SecretLocator {
    pub fn master_key() -> Self {
        Self {
            service: "dev.easyenv.master".to_string(),
            account: "master/aes256gcm/default".to_string(),
            label: "easyenv master key".to_string(),
        }
    }

    pub fn doctor_probe(suffix: &str) -> Self {
        Self {
            service: "dev.easyenv.doctor".to_string(),
            account: format!("probe/{suffix}"),
            label: "easyenv doctor probe".to_string(),
        }
    }

    pub fn share_identity() -> Self {
        Self {
            service: "dev.easyenv.identity".to_string(),
            account: "workspace/age/x25519/default".to_string(),
            label: "easyenv workspace identity".to_string(),
        }
    }

    pub fn for_var(metadata: &VarMetadata) -> Result<Self> {
        let key = metadata.key.as_str();
        let profile = metadata.profile.trim();

        if profile.is_empty() {
            return Err(anyhow!("profile cannot be empty"));
        }

        let (service, account) = match metadata.scope {
            Scope::Global => (
                "dev.easyenv.global".to_string(),
                format!("global/{profile}/{key}"),
            ),
            Scope::Project => {
                let project_id = metadata
                    .project_id
                    .as_deref()
                    .ok_or_else(|| anyhow!("project scope requires a project id"))?;
                (
                    "dev.easyenv.project".to_string(),
                    format!("project/{project_id}/{profile}/{key}"),
                )
            }
            Scope::Shell => bail!("shell scope values are never stored"),
        };

        Ok(Self {
            service,
            account,
            label: format!("easyenv {key}"),
        })
    }
}

pub fn canonicalize_existing(path: &Path) -> Result<PathBuf> {
    Ok(path.canonicalize()?)
}

pub fn project_id_for_path(path: &Path) -> String {
    let mut hasher = Sha256::new();
    hasher.update(path.to_string_lossy().as_bytes());
    let digest = hasher.finalize();
    hex::encode(&digest[..16])
}

pub fn default_project_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("project")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_valid_env_keys() {
        assert!(EnvKey::parse("OPENAI_API_KEY").is_ok());
        assert!(EnvKey::parse("_PRIVATE").is_ok());
    }

    #[test]
    fn rejects_invalid_env_keys() {
        assert!(EnvKey::parse("1BAD").is_err());
        assert!(EnvKey::parse("BAD-KEY").is_err());
        assert!(EnvKey::parse("").is_err());
    }

    #[test]
    fn scope_roundtrip() {
        assert_eq!(Scope::from_str("global").unwrap(), Scope::Global);
        assert_eq!(Scope::Project.to_string(), "project");
    }
}
