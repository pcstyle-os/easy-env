use age::{
    Encryptor,
    armor::{ArmoredWriter, Format},
    secrecy::ExposeSecret,
    x25519,
};
use anyhow::{Context, Result, anyhow, bail};
use easyenv_core::{DesiredScope, EasyEnv, EnvKey, Scope, SecretLocator, SecretStore};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
};
use zeroize::Zeroizing;

#[derive(Debug, Clone)]
pub struct IdentityInfo {
    pub recipient: String,
}

#[derive(Debug, Clone)]
pub struct ShareExport {
    pub output_path: PathBuf,
    pub item_count: usize,
    pub recipient_count: usize,
}

#[derive(Debug, Clone)]
pub struct ShareImport {
    pub sender: String,
    pub item_count: usize,
    pub scope: Scope,
}

#[derive(Debug, Serialize, Deserialize)]
struct ShareBundle {
    version: u8,
    sender: String,
    profile: String,
    created_at: i64,
    items: Vec<SharedItem>,
}

#[derive(Debug, Serialize, Deserialize)]
struct SharedItem {
    key: String,
    value: String,
    source_scope: String,
}

pub fn ensure_identity<S: SecretStore>(store: &S) -> Result<IdentityInfo> {
    let identity = load_or_generate_identity(store)?;
    Ok(IdentityInfo {
        recipient: identity.to_public().to_string(),
    })
}

pub fn rotate_identity<S: SecretStore>(store: &S) -> Result<IdentityInfo> {
    let identity = x25519::Identity::generate();
    persist_identity(store, &identity)?;
    Ok(IdentityInfo {
        recipient: identity.to_public().to_string(),
    })
}

pub fn export_bundle<S: SecretStore>(
    app: &EasyEnv<S>,
    store: &S,
    cwd: &Path,
    profile: &str,
    keys: &[EnvKey],
    include_all: bool,
    recipients: &[String],
    output_path: &Path,
) -> Result<ShareExport> {
    if recipients.is_empty() {
        bail!("provide at least one --to age recipient")
    }

    let sender = ensure_identity(store)?.recipient;
    let selected = select_secrets(app, cwd, profile, keys, include_all)?;
    let parsed_recipients = recipients
        .iter()
        .map(|recipient| {
            recipient
                .parse::<x25519::Recipient>()
                .map_err(|error| anyhow!("invalid age recipient {recipient}: {error}"))
        })
        .collect::<Result<Vec<_>>>()?;

    let bundle = ShareBundle {
        version: 1,
        sender,
        profile: profile.to_string(),
        created_at: current_timestamp(),
        items: selected
            .into_iter()
            .map(|secret| SharedItem {
                key: secret.key.to_string(),
                value: secret.value.to_string(),
                source_scope: secret.scope.to_string(),
            })
            .collect(),
    };

    let plaintext = serde_json::to_vec_pretty(&bundle)?;
    let ciphertext = encrypt_armored(&parsed_recipients, &plaintext)?;
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(output_path, ciphertext)?;

    Ok(ShareExport {
        output_path: output_path.to_path_buf(),
        item_count: bundle.items.len(),
        recipient_count: parsed_recipients.len(),
    })
}

pub fn import_bundle<S: SecretStore>(
    app: &EasyEnv<S>,
    store: &S,
    cwd: &Path,
    desired_scope: DesiredScope,
    profile: &str,
    input_path: &Path,
) -> Result<ShareImport> {
    let identity = load_or_generate_identity(store)?;
    let ciphertext =
        fs::read(input_path).with_context(|| format!("failed to read {}", input_path.display()))?;
    let plaintext =
        age::decrypt(&identity, &ciphertext).context("failed to decrypt share bundle")?;
    let bundle: ShareBundle = serde_json::from_slice(&plaintext).context("invalid share bundle")?;

    let mut item_count = 0;
    let mut final_scope = None;
    for item in bundle.items {
        let key = EnvKey::parse(item.key)?;
        let stored = app.set_secret(cwd, desired_scope, profile, key, item.value.as_str(), None)?;
        final_scope = Some(stored.scope);
        item_count += 1;
    }

    let scope = final_scope.ok_or_else(|| anyhow!("share bundle did not contain any items"))?;
    Ok(ShareImport {
        sender: bundle.sender,
        item_count,
        scope,
    })
}

fn load_or_generate_identity<S: SecretStore>(store: &S) -> Result<x25519::Identity> {
    let locator = SecretLocator::share_identity();
    if let Some(secret) = store.get(&locator)? {
        let secret = Zeroizing::new(
            String::from_utf8(secret).context("workspace identity was not valid UTF-8")?,
        );
        return secret
            .as_str()
            .parse::<x25519::Identity>()
            .map_err(|error| anyhow!("workspace identity could not be parsed: {error}"));
    }

    let identity = x25519::Identity::generate();
    persist_identity(store, &identity)?;
    Ok(identity)
}

fn persist_identity<S: SecretStore>(store: &S, identity: &x25519::Identity) -> Result<()> {
    let locator = SecretLocator::share_identity();
    let secret = identity.to_string();
    store.put(&locator, secret.expose_secret().as_bytes())
}

fn select_secrets<S: SecretStore>(
    app: &EasyEnv<S>,
    cwd: &Path,
    profile: &str,
    keys: &[EnvKey],
    include_all: bool,
) -> Result<Vec<easyenv_core::ResolvedSecret>> {
    if !include_all && keys.is_empty() {
        bail!("select at least one secret with --key or use --all")
    }

    let mut selected: BTreeMap<String, easyenv_core::ResolvedSecret> = BTreeMap::new();

    if include_all {
        for secret in app.list_secrets(cwd, None, profile)? {
            selected.insert(secret.key.to_string(), secret);
        }
    }

    for key in keys {
        let Some(secret) = app.get_secret(cwd, key, profile)? else {
            bail!("{} not found", key);
        };
        selected.insert(secret.key.to_string(), secret);
    }

    Ok(selected.into_values().collect())
}

fn encrypt_armored(recipients: &[x25519::Recipient], plaintext: &[u8]) -> Result<Vec<u8>> {
    let encryptor = Encryptor::with_recipients(recipients.iter().map(|recipient| recipient as _))?;

    let mut ciphertext = Vec::new();
    let armored = ArmoredWriter::wrap_output(&mut ciphertext, Format::AsciiArmor)?;
    let mut writer = encryptor.wrap_output(armored)?;
    writer.write_all(plaintext)?;
    writer.finish()?.finish()?;
    Ok(ciphertext)
}

fn current_timestamp() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock drifted before unix epoch")
        .as_secs() as i64
}
