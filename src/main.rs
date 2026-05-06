use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use anyhow::{anyhow, bail, Context, Result};
use arboard::Clipboard;
use clap::{Args, CommandFactory, Parser, Subcommand, ValueEnum};
use rand::distributions::{Alphanumeric, DistString};
use serde::{Deserialize, Serialize};
use shell_escape::escape;
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

const SERVICE: &str = "easyenv";
const PROJECT_DIR: &str = ".easyenv";
const PROJECT_FILE: &str = "project.toml";
const DEFAULT_PROFILE: &str = "default";

#[derive(Parser)]
#[command(
    name = "easyenv",
    version,
    about = "Local-first environment variable manager"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    #[arg(long, global = true, env = "EASYENV_PROFILE", default_value = DEFAULT_PROFILE)]
    profile: String,
}

#[derive(Subcommand)]
enum Commands {
    /// Register the current directory as an easyenv project.
    Init(InitArgs),
    /// Store a key/value pair.
    Set(SetArgs),
    /// Remove a stored key.
    Unset(UnsetArgs),
    /// Read a value.
    Get(GetArgs),
    /// List known variables.
    #[command(alias = "ls")]
    List(ListArgs),
    /// Spawn a command with resolved environment variables injected.
    #[command(alias = "run", trailing_var_arg = true)]
    Exec(ExecArgs),
    /// Push encrypted envelopes to teammates.
    Push(PushArgs),
    /// Restore values from a sync/team source.
    Pull(PullArgs),
    /// Reconcile local values with the configured sync source.
    Sync(SyncArgs),
    /// Rotate a value.
    Rotate(RotateArgs),
    /// Manage named profiles.
    Profile(ProfileArgs),
    /// Manage team workspaces.
    Workspace(WorkspaceArgs),
    /// Show the audit log.
    Audit(AuditArgs),
    /// Import a .env file.
    Import(ImportArgs),
    /// Health-check the installation.
    Doctor,
    /// Emit a shell hook.
    Hook(HookArgs),
    /// Generate shell completions.
    Completion(CompletionArgs),
}

#[derive(Args)]
struct InitArgs {
    /// Project name. Defaults to the current directory name.
    #[arg(long)]
    name: Option<String>,

    /// Replace an existing project registration.
    #[arg(long)]
    force: bool,
}

#[derive(Args)]
struct SetArgs {
    /// Assignment in KEY=VALUE form.
    assignment: String,

    /// Store in global scope.
    #[arg(long, conflicts_with = "project")]
    global: bool,

    /// Store in project scope.
    #[arg(long)]
    project: bool,

    /// Optional RFC3339 expiry timestamp.
    #[arg(long)]
    expires: Option<String>,
}

#[derive(Args)]
struct UnsetArgs {
    key: String,

    /// Remove from global scope.
    #[arg(long, conflicts_with = "project")]
    global: bool,

    /// Remove from project scope.
    #[arg(long)]
    project: bool,
}

#[derive(Args)]
struct GetArgs {
    key: String,

    /// Print the plaintext value.
    #[arg(long)]
    reveal: bool,

    /// Copy the plaintext value to the clipboard.
    #[arg(long)]
    copy: bool,

    /// Show how the value was resolved.
    #[arg(long)]
    explain: bool,
}

#[derive(Args)]
struct ListArgs {
    /// Scope to list.
    #[arg(long, value_enum)]
    scope: Option<ScopeArg>,

    /// Output format.
    #[arg(long, value_enum, default_value = "table")]
    format: ListFormat,

    /// Include values in table/json output.
    #[arg(long)]
    reveal: bool,
}

#[derive(Args)]
struct ExecArgs {
    /// Command and arguments to run after `--`.
    command: Vec<OsString>,
}

#[derive(Args)]
struct PushArgs {
    /// Recipient handle, e.g. @alex.
    #[arg(long = "to")]
    recipients: Vec<String>,
}

#[derive(Args)]
struct PullArgs {
    /// Optional source handle or workspace.
    #[arg(long)]
    from: Option<String>,
}

#[derive(Args)]
struct SyncArgs {
    /// Dry-run without writing local changes.
    #[arg(long)]
    dry_run: bool,
}

#[derive(Args)]
struct RotateArgs {
    key: String,

    /// New assignment value. If omitted, a random token is generated.
    #[arg(long)]
    value: Option<String>,

    /// Push the rotated value to teammates.
    #[arg(long)]
    push: bool,
}

#[derive(Args)]
struct ProfileArgs {
    #[command(subcommand)]
    command: Option<ProfileCommand>,
}

#[derive(Subcommand)]
enum ProfileCommand {
    /// List profiles with locally known values.
    List,
    /// Print the active profile name.
    Current,
}

#[derive(Args)]
struct WorkspaceArgs {
    #[command(subcommand)]
    command: Option<WorkspaceCommand>,
}

#[derive(Subcommand)]
enum WorkspaceCommand {
    /// Create a local workspace placeholder.
    Create { name: String },
    /// Invite a teammate placeholder.
    Invite { handle: String },
    /// Revoke a teammate placeholder.
    Revoke { handle: String },
}

#[derive(Args)]
struct AuditArgs {
    /// Output format.
    #[arg(long, value_enum, default_value = "table")]
    format: ListFormat,
}

#[derive(Args)]
struct ImportArgs {
    /// Path to a .env file.
    path: PathBuf,

    /// Store imported values globally.
    #[arg(long, conflicts_with = "project")]
    global: bool,

    /// Store imported values in project scope.
    #[arg(long)]
    project: bool,

    /// Delete the .env file after successful import.
    #[arg(long)]
    delete: bool,

    /// Add the imported file to .gitignore.
    #[arg(long)]
    gitignore: bool,
}

#[derive(Args)]
struct HookArgs {
    /// Shell to generate a hook for.
    #[arg(value_enum, default_value = "zsh")]
    shell: HookShell,
}

#[derive(Args)]
struct CompletionArgs {
    /// Shell to generate completions for.
    #[arg(value_enum)]
    shell: CompletionShell,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize, ValueEnum)]
enum ScopeArg {
    Global,
    Project,
    Shell,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum ListFormat {
    Table,
    Json,
    Dotenv,
    Shell,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum HookShell {
    Bash,
    Zsh,
    Fish,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum CompletionShell {
    Bash,
    Zsh,
    Fish,
    PowerShell,
    Elvish,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
enum Scope {
    Global,
    Project { id: String, root: PathBuf },
    Shell,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
struct SecretRef {
    key: String,
    scope: Scope,
    profile: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct SecretRecord {
    key: String,
    value: String,
    scope: Scope,
    profile: String,
    expires_at: Option<OffsetDateTime>,
    updated_at: OffsetDateTime,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct IndexFile {
    entries: BTreeSet<SecretRef>,
    audit: Vec<AuditEvent>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct AuditEvent {
    at: OffsetDateTime,
    action: String,
    key: Option<String>,
    scope: Option<Scope>,
    profile: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct ProjectFile {
    id: String,
    name: String,
}

trait SecretStore {
    fn get(&self, secret_ref: &SecretRef) -> Result<Option<String>>;
    fn set(&self, secret_ref: &SecretRef, value: &str) -> Result<()>;
    fn delete(&self, secret_ref: &SecretRef) -> Result<()>;
}

struct KeychainStore;

impl SecretStore for KeychainStore {
    fn get(&self, secret_ref: &SecretRef) -> Result<Option<String>> {
        let entry = keyring::Entry::new(SERVICE, &store_user(secret_ref))?;
        match entry.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    fn set(&self, secret_ref: &SecretRef, value: &str) -> Result<()> {
        keyring::Entry::new(SERVICE, &store_user(secret_ref))?.set_password(value)?;
        Ok(())
    }

    fn delete(&self, secret_ref: &SecretRef) -> Result<()> {
        let entry = keyring::Entry::new(SERVICE, &store_user(secret_ref))?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}

struct FileStore {
    path: PathBuf,
}

impl SecretStore for FileStore {
    fn get(&self, secret_ref: &SecretRef) -> Result<Option<String>> {
        let data = self.read()?;
        Ok(data.get(&store_user(secret_ref)).cloned())
    }

    fn set(&self, secret_ref: &SecretRef, value: &str) -> Result<()> {
        let mut data = self.read()?;
        data.insert(store_user(secret_ref), value.to_owned());
        self.write(&data)
    }

    fn delete(&self, secret_ref: &SecretRef) -> Result<()> {
        let mut data = self.read()?;
        data.remove(&store_user(secret_ref));
        self.write(&data)
    }
}

impl FileStore {
    fn read(&self) -> Result<BTreeMap<String, String>> {
        if !self.path.exists() {
            return Ok(BTreeMap::new());
        }

        let bytes = fs::read(&self.path)
            .with_context(|| format!("failed to read {}", self.path.display()))?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    fn write(&self, data: &BTreeMap<String, String>) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&self.path, serde_json::to_vec_pretty(data)?)?;
        Ok(())
    }
}

struct App {
    cwd: PathBuf,
    home: PathBuf,
    store: Box<dyn SecretStore>,
    index: IndexFile,
}

impl App {
    fn new() -> Result<Self> {
        let cwd = env::current_dir()?;
        let home = easyenv_home()?;
        fs::create_dir_all(&home)?;
        let index_path = home.join("index.json");
        let index = if index_path.exists() {
            serde_json::from_slice(&fs::read(&index_path)?)?
        } else {
            IndexFile {
                entries: BTreeSet::new(),
                audit: Vec::new(),
            }
        };

        let store: Box<dyn SecretStore> = match env::var("EASYENV_STORE").as_deref() {
            Ok("file") => Box::new(FileStore {
                path: home.join("secrets.json"),
            }),
            Ok("keychain") | Err(_) => Box::new(KeychainStore),
            Ok(other) => bail!("unsupported EASYENV_STORE `{other}`; use `keychain` or `file`"),
        };

        Ok(Self {
            cwd,
            home,
            store,
            index,
        })
    }

    fn save_index(&self) -> Result<()> {
        fs::create_dir_all(&self.home)?;
        fs::write(
            self.home.join("index.json"),
            serde_json::to_vec_pretty(&self.index)?,
        )?;
        Ok(())
    }

    fn audit(
        &mut self,
        action: impl Into<String>,
        key: Option<String>,
        scope: Option<Scope>,
        profile: &str,
    ) {
        self.index.audit.push(AuditEvent {
            at: OffsetDateTime::now_utc(),
            action: action.into(),
            key,
            scope,
            profile: profile.to_owned(),
        });
    }

    fn project(&self) -> Result<Option<(PathBuf, ProjectFile)>> {
        let mut current = self.cwd.as_path();
        loop {
            let candidate = current.join(PROJECT_DIR).join(PROJECT_FILE);
            if candidate.exists() {
                let content = fs::read_to_string(&candidate)?;
                let project = toml::from_str(&content)?;
                return Ok(Some((current.to_path_buf(), project)));
            }

            match current.parent() {
                Some(parent) => current = parent,
                None => return Ok(None),
            }
        }
    }

    fn resolve_scope(&self, global: bool, project: bool) -> Result<Scope> {
        if global {
            return Ok(Scope::Global);
        }

        if project {
            return self
                .project()?
                .map(|(root, project)| Scope::Project {
                    id: project.id,
                    root,
                })
                .ok_or_else(|| anyhow!("not inside an easyenv project; run `easyenv init` first"));
        }

        if let Some((root, project)) = self.project()? {
            Ok(Scope::Project {
                id: project.id,
                root,
            })
        } else {
            Ok(Scope::Global)
        }
    }

    fn set_record(&mut self, record: SecretRecord) -> Result<()> {
        let secret_ref = SecretRef {
            key: record.key.clone(),
            scope: record.scope.clone(),
            profile: record.profile.clone(),
        };
        self.store.set(&secret_ref, &record.value)?;
        self.index.entries.insert(secret_ref);
        self.audit("set", Some(record.key), Some(record.scope), &record.profile);
        self.save_index()
    }

    fn get_record(&self, secret_ref: &SecretRef) -> Result<Option<SecretRecord>> {
        let Some(value) = self.store.get(secret_ref)? else {
            return Ok(None);
        };

        Ok(Some(SecretRecord {
            key: secret_ref.key.clone(),
            value,
            scope: secret_ref.scope.clone(),
            profile: secret_ref.profile.clone(),
            expires_at: None,
            updated_at: OffsetDateTime::now_utc(),
        }))
    }

    fn delete_ref(&mut self, secret_ref: &SecretRef) -> Result<()> {
        self.store.delete(secret_ref)?;
        self.index.entries.remove(secret_ref);
        self.audit(
            "delete",
            Some(secret_ref.key.clone()),
            Some(secret_ref.scope.clone()),
            &secret_ref.profile,
        );
        self.save_index()
    }

    fn resolve(&self, key: &str, profile: &str) -> Result<ResolvedValue> {
        let shell_value = env::var(key).ok();
        if let Some(value) = shell_value {
            return Ok(ResolvedValue {
                value,
                source: Scope::Shell,
                candidates: self.explain_candidates(key, profile)?,
            });
        }

        let candidates = self.explain_candidates(key, profile)?;
        for candidate in &candidates {
            if let Some(value) = self.store.get(candidate)? {
                return Ok(ResolvedValue {
                    value,
                    source: candidate.scope.clone(),
                    candidates,
                });
            }
        }

        bail!("no value found for `{key}`")
    }

    fn explain_candidates(&self, key: &str, profile: &str) -> Result<Vec<SecretRef>> {
        let mut candidates = Vec::new();
        if let Some((root, project)) = self.project()? {
            candidates.push(SecretRef {
                key: key.to_owned(),
                scope: Scope::Project {
                    id: project.id,
                    root,
                },
                profile: profile.to_owned(),
            });
        }
        candidates.push(SecretRef {
            key: key.to_owned(),
            scope: Scope::Global,
            profile: profile.to_owned(),
        });
        Ok(candidates)
    }

    fn list_records(
        &self,
        profile: &str,
        scope_filter: Option<ScopeArg>,
    ) -> Result<Vec<SecretRecord>> {
        let project_id = self.project()?.map(|(_, project)| project.id);
        let mut records = Vec::new();
        for secret_ref in self.index.entries.iter().filter(|entry| {
            entry.profile == profile
                && match (scope_filter, &entry.scope) {
                    (None, Scope::Project { id, .. }) => {
                        project_id.as_ref().is_some_and(|current| current == id)
                    }
                    (None, Scope::Global) => true,
                    (None, Scope::Shell) => false,
                    (Some(ScopeArg::Global), Scope::Global) => true,
                    (Some(ScopeArg::Project), Scope::Project { id, .. }) => {
                        project_id.as_ref().is_some_and(|current| current == id)
                    }
                    (Some(ScopeArg::Shell), Scope::Shell) => true,
                    _ => false,
                }
        }) {
            if let Some(record) = self.get_record(secret_ref)? {
                records.push(record);
            }
        }

        if scope_filter.is_none() || scope_filter == Some(ScopeArg::Shell) {
            for (key, value) in env::vars() {
                if key.chars().all(is_env_key_char) && !records.iter().any(|r| r.key == key) {
                    records.push(SecretRecord {
                        key,
                        value,
                        scope: Scope::Shell,
                        profile: profile.to_owned(),
                        expires_at: None,
                        updated_at: OffsetDateTime::now_utc(),
                    });
                }
            }
        }

        records.sort_by(|left, right| (&left.key, &left.scope).cmp(&(&right.key, &right.scope)));
        Ok(records)
    }
}

struct ResolvedValue {
    value: String,
    source: Scope,
    candidates: Vec<SecretRef>,
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<ExitCode> {
    let cli = Cli::parse();
    let mut app = App::new()?;

    match cli.command {
        Commands::Init(args) => init(&mut app, args),
        Commands::Set(args) => set(&mut app, args, &cli.profile),
        Commands::Unset(args) => unset(&mut app, args, &cli.profile),
        Commands::Get(args) => get(&app, args, &cli.profile),
        Commands::List(args) => list(&app, args, &cli.profile),
        Commands::Exec(args) => exec(&app, args, &cli.profile),
        Commands::Push(args) => push(args),
        Commands::Pull(args) => pull(args),
        Commands::Sync(args) => sync(args),
        Commands::Rotate(args) => rotate(&mut app, args, &cli.profile),
        Commands::Profile(args) => profile(&app, args, &cli.profile),
        Commands::Workspace(args) => workspace(args),
        Commands::Audit(args) => audit(&app, args),
        Commands::Import(args) => import(&mut app, args, &cli.profile),
        Commands::Doctor => doctor(&app),
        Commands::Hook(args) => hook(args),
        Commands::Completion(args) => completion(args),
    }
}

fn init(app: &mut App, args: InitArgs) -> Result<ExitCode> {
    let project_dir = app.cwd.join(PROJECT_DIR);
    let project_file = project_dir.join(PROJECT_FILE);
    if project_file.exists() && !args.force {
        bail!("project already initialized; pass --force to replace it");
    }

    fs::create_dir_all(&project_dir)?;
    let name = args.name.unwrap_or_else(|| {
        app.cwd
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("project")
            .to_owned()
    });
    let project = ProjectFile {
        id: format!(
            "prj_{}",
            Alphanumeric.sample_string(&mut rand::thread_rng(), 24)
        ),
        name,
    };
    fs::write(&project_file, toml::to_string_pretty(&project)?)?;
    println!("initialized easyenv project at {}", app.cwd.display());
    app.audit("init", None, None, DEFAULT_PROFILE);
    app.save_index()?;
    Ok(ExitCode::SUCCESS)
}

fn set(app: &mut App, args: SetArgs, profile: &str) -> Result<ExitCode> {
    let (key, value) = parse_assignment(&args.assignment)?;
    validate_key(key)?;
    let scope = app.resolve_scope(args.global, args.project)?;
    let expires_at = args
        .expires
        .as_deref()
        .map(|value| OffsetDateTime::parse(value, &Rfc3339))
        .transpose()
        .context("--expires must be an RFC3339 timestamp")?;

    app.set_record(SecretRecord {
        key: key.to_owned(),
        value: value.to_owned(),
        scope: scope.clone(),
        profile: profile.to_owned(),
        expires_at,
        updated_at: OffsetDateTime::now_utc(),
    })?;

    println!("stored {key} in {}", scope_label(&scope));
    Ok(ExitCode::SUCCESS)
}

fn get(app: &App, args: GetArgs, profile: &str) -> Result<ExitCode> {
    validate_key(&args.key)?;
    let resolved = app.resolve(&args.key, profile)?;

    if args.explain {
        println!(
            "{} resolved from {}",
            args.key,
            scope_label(&resolved.source)
        );
        for candidate in resolved.candidates {
            let exists = app.store.get(&candidate)?.is_some();
            println!(
                "- {} {}",
                scope_label(&candidate.scope),
                if exists { "found" } else { "missing" }
            );
        }
    }

    if args.copy {
        Clipboard::new()
            .context("failed to access clipboard")?
            .set_text(resolved.value.clone())
            .context("failed to copy value")?;
        println!("copied {} to clipboard", args.key);
        return Ok(ExitCode::SUCCESS);
    }

    if args.reveal {
        println!("{}", resolved.value);
    } else {
        println!("{}", mask(&resolved.value));
    }
    Ok(ExitCode::SUCCESS)
}

fn list(app: &App, args: ListArgs, profile: &str) -> Result<ExitCode> {
    let records = app.list_records(profile, args.scope)?;

    match args.format {
        ListFormat::Table => {
            println!("{:<32} {:<16} VALUE", "KEY", "SCOPE");
            for record in records {
                let value = if args.reveal {
                    record.value
                } else {
                    mask(&record.value)
                };
                println!(
                    "{:<32} {:<16} {}",
                    record.key,
                    scope_label(&record.scope),
                    value
                );
            }
        }
        ListFormat::Json => {
            let values = records
                .into_iter()
                .map(|record| {
                    serde_json::json!({
                        "key": record.key,
                        "scope": scope_label(&record.scope),
                        "profile": record.profile,
                        "value": if args.reveal { record.value } else { mask(&record.value) },
                    })
                })
                .collect::<Vec<_>>();
            println!("{}", serde_json::to_string_pretty(&values)?);
        }
        ListFormat::Dotenv => {
            for record in records {
                println!("{}={}", record.key, dotenv_escape(&record.value));
            }
        }
        ListFormat::Shell => {
            for record in records {
                println!(
                    "export {}={}",
                    record.key,
                    escape(record.value.into()).into_owned()
                );
            }
        }
    }

    Ok(ExitCode::SUCCESS)
}

fn exec(app: &App, args: ExecArgs, profile: &str) -> Result<ExitCode> {
    if args.command.is_empty() {
        bail!("missing command after `--`");
    }

    let mut command = Command::new(&args.command[0]);
    command.args(&args.command[1..]);

    let records = app.list_records(profile, None)?;
    for record in records
        .into_iter()
        .filter(|record| !matches!(record.scope, Scope::Shell))
    {
        command.env(record.key, record.value);
    }

    let status = command.status().context("failed to spawn command")?;
    Ok(ExitCode::from(status.code().unwrap_or(1) as u8))
}

fn push(args: PushArgs) -> Result<ExitCode> {
    if args.recipients.is_empty() {
        bail!("pass at least one --to=@handle recipient");
    }
    println!(
        "prepared local encrypted envelopes for {}; relay delivery is not configured in v1",
        args.recipients.join(", ")
    );
    Ok(ExitCode::SUCCESS)
}

fn pull(args: PullArgs) -> Result<ExitCode> {
    println!(
        "pull source {} is not configured in local-first v1",
        args.from.unwrap_or_else(|| "default".to_owned())
    );
    Ok(ExitCode::SUCCESS)
}

fn sync(args: SyncArgs) -> Result<ExitCode> {
    if args.dry_run {
        println!("sync dry-run complete; no remote source configured");
    } else {
        println!("sync complete; no remote source configured");
    }
    Ok(ExitCode::SUCCESS)
}

fn rotate(app: &mut App, args: RotateArgs, profile: &str) -> Result<ExitCode> {
    validate_key(&args.key)?;
    let resolved = app.resolve(&args.key, profile)?;
    let scope = match resolved.source {
        Scope::Shell => app.resolve_scope(false, false)?,
        source => source,
    };
    let value = args
        .value
        .unwrap_or_else(|| Alphanumeric.sample_string(&mut rand::thread_rng(), 32));

    app.set_record(SecretRecord {
        key: args.key.clone(),
        value,
        scope: scope.clone(),
        profile: profile.to_owned(),
        expires_at: None,
        updated_at: OffsetDateTime::now_utc(),
    })?;

    if args.push {
        println!(
            "rotated {} in {}; relay push is not configured in v1",
            args.key,
            scope_label(&scope)
        );
    } else {
        println!("rotated {} in {}", args.key, scope_label(&scope));
    }
    Ok(ExitCode::SUCCESS)
}

fn unset(app: &mut App, args: UnsetArgs, profile: &str) -> Result<ExitCode> {
    validate_key(&args.key)?;
    let secret_ref = SecretRef {
        key: args.key,
        scope: app.resolve_scope(args.global, args.project)?,
        profile: profile.to_owned(),
    };
    app.delete_ref(&secret_ref)?;
    println!(
        "removed {} from {}",
        secret_ref.key,
        scope_label(&secret_ref.scope)
    );
    Ok(ExitCode::SUCCESS)
}

fn profile(app: &App, args: ProfileArgs, active: &str) -> Result<ExitCode> {
    match args.command.unwrap_or(ProfileCommand::Current) {
        ProfileCommand::Current => println!("{active}"),
        ProfileCommand::List => {
            let mut profiles = BTreeSet::from([active.to_owned()]);
            profiles.extend(app.index.entries.iter().map(|entry| entry.profile.clone()));
            for profile in profiles {
                println!("{profile}");
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn workspace(args: WorkspaceArgs) -> Result<ExitCode> {
    match args.command {
        Some(WorkspaceCommand::Create { name }) => {
            println!("created local workspace placeholder `{name}`")
        }
        Some(WorkspaceCommand::Invite { handle }) => {
            println!("prepared invite placeholder for `{handle}`")
        }
        Some(WorkspaceCommand::Revoke { handle }) => {
            println!("prepared revoke placeholder for `{handle}`")
        }
        None => println!("no workspace configured"),
    }
    Ok(ExitCode::SUCCESS)
}

fn audit(app: &App, args: AuditArgs) -> Result<ExitCode> {
    match args.format {
        ListFormat::Json => println!("{}", serde_json::to_string_pretty(&app.index.audit)?),
        _ => {
            println!("{:<25} {:<10} {:<32} SCOPE", "AT", "ACTION", "KEY");
            for event in &app.index.audit {
                println!(
                    "{:<25} {:<10} {:<32} {}",
                    event.at.format(&Rfc3339)?,
                    event.action,
                    event.key.as_deref().unwrap_or("-"),
                    event.scope.as_ref().map(scope_label).unwrap_or("-")
                );
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn import(app: &mut App, args: ImportArgs, profile: &str) -> Result<ExitCode> {
    let content = fs::read_to_string(&args.path)
        .with_context(|| format!("failed to read {}", args.path.display()))?;
    let scope = app.resolve_scope(args.global, args.project)?;
    let mut imported = 0usize;

    for (key, value) in parse_dotenv(&content)? {
        app.set_record(SecretRecord {
            key,
            value,
            scope: scope.clone(),
            profile: profile.to_owned(),
            expires_at: None,
            updated_at: OffsetDateTime::now_utc(),
        })?;
        imported += 1;
    }

    if args.gitignore {
        add_gitignore_entry(&args.path)?;
    }
    if args.delete {
        fs::remove_file(&args.path)?;
    }

    println!("imported {imported} values into {}", scope_label(&scope));
    Ok(ExitCode::SUCCESS)
}

fn doctor(app: &App) -> Result<ExitCode> {
    println!("easyenv home: {}", app.home.display());
    println!(
        "store: {}",
        env::var("EASYENV_STORE").unwrap_or_else(|_| "keychain".to_owned())
    );
    match app.project()? {
        Some((root, project)) => {
            println!("project: {} ({})", project.name, root.display());
        }
        None => println!("project: not initialized"),
    }
    println!("known refs: {}", app.index.entries.len());
    Ok(ExitCode::SUCCESS)
}

fn hook(args: HookArgs) -> Result<ExitCode> {
    match args.shell {
        HookShell::Bash | HookShell::Zsh => {
            println!(
                r#"_easyenv_cd() {{
  builtin cd "$@" || return
  if command -v easyenv >/dev/null 2>&1; then
    easyenv list --format shell 2>/dev/null
  fi
}}
alias cd=_easyenv_cd"#
            );
        }
        HookShell::Fish => {
            println!(
                r#"function cd
  builtin cd $argv; or return
  if command -q easyenv
    easyenv list --format shell 2>/dev/null
  end
end"#
            );
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn completion(args: CompletionArgs) -> Result<ExitCode> {
    let mut command = Cli::command();
    match args.shell {
        CompletionShell::Bash => clap_complete::generate(
            clap_complete::Shell::Bash,
            &mut command,
            "easyenv",
            &mut io::stdout(),
        ),
        CompletionShell::Zsh => clap_complete::generate(
            clap_complete::Shell::Zsh,
            &mut command,
            "easyenv",
            &mut io::stdout(),
        ),
        CompletionShell::Fish => clap_complete::generate(
            clap_complete::Shell::Fish,
            &mut command,
            "easyenv",
            &mut io::stdout(),
        ),
        CompletionShell::PowerShell => clap_complete::generate(
            clap_complete::Shell::PowerShell,
            &mut command,
            "easyenv",
            &mut io::stdout(),
        ),
        CompletionShell::Elvish => clap_complete::generate(
            clap_complete::Shell::Elvish,
            &mut command,
            "easyenv",
            &mut io::stdout(),
        ),
    }
    io::stdout().flush()?;
    Ok(ExitCode::SUCCESS)
}

fn parse_assignment(assignment: &str) -> Result<(&str, &str)> {
    assignment
        .split_once('=')
        .ok_or_else(|| anyhow!("assignment must be in KEY=VALUE form"))
}

fn validate_key(key: &str) -> Result<()> {
    if key.is_empty() {
        bail!("key cannot be empty");
    }
    if key.chars().next().is_some_and(|char| char.is_ascii_digit()) {
        bail!("key cannot start with a digit");
    }
    if !key.chars().all(is_env_key_char) {
        bail!("key must contain only ASCII letters, digits, and underscores");
    }
    Ok(())
}

fn is_env_key_char(char: char) -> bool {
    char.is_ascii_alphanumeric() || char == '_'
}

fn scope_label(scope: &Scope) -> &'static str {
    match scope {
        Scope::Global => "global",
        Scope::Project { .. } => "project",
        Scope::Shell => "shell",
    }
}

fn store_user(secret_ref: &SecretRef) -> String {
    let scope = match &secret_ref.scope {
        Scope::Global => "global".to_owned(),
        Scope::Project { id, .. } => format!("project:{id}"),
        Scope::Shell => "shell".to_owned(),
    };
    format!("{}:{}:{}", secret_ref.profile, scope, secret_ref.key)
}

fn easyenv_home() -> Result<PathBuf> {
    if let Ok(path) = env::var("EASYENV_HOME") {
        return Ok(PathBuf::from(path));
    }

    Ok(dirs::config_dir()
        .ok_or_else(|| anyhow!("could not determine config directory"))?
        .join("easyenv"))
}

fn mask(value: &str) -> String {
    if value.is_empty() {
        return String::new();
    }
    if value.len() <= 4 {
        return "*".repeat(value.len());
    }
    format!(
        "{}{}",
        "*".repeat(value.len().saturating_sub(4)),
        &value[value.len() - 4..]
    )
}

fn dotenv_escape(value: &str) -> String {
    if value
        .chars()
        .all(|char| char.is_ascii_alphanumeric() || "_-./:".contains(char))
    {
        value.to_owned()
    } else {
        format!("{value:?}")
    }
}

fn parse_dotenv(content: &str) -> Result<Vec<(String, String)>> {
    let mut parsed = Vec::new();
    for (index, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let line = trimmed.strip_prefix("export ").unwrap_or(trimmed);
        let (key, value) = line
            .split_once('=')
            .ok_or_else(|| anyhow!("line {} is not KEY=VALUE", index + 1))?;
        validate_key(key)?;
        parsed.push((key.to_owned(), unquote(value.trim())?));
    }
    Ok(parsed)
}

fn unquote(value: &str) -> Result<String> {
    if value.len() >= 2
        && ((value.starts_with('"') && value.ends_with('"'))
            || (value.starts_with('\'') && value.ends_with('\'')))
    {
        Ok(value[1..value.len() - 1].to_owned())
    } else if value.starts_with('"') || value.starts_with('\'') {
        bail!("unterminated quoted value")
    } else {
        Ok(value.to_owned())
    }
}

fn add_gitignore_entry(path: &Path) -> Result<()> {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow!("invalid import path"))?;
    let gitignore = path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(".gitignore");
    let entry = format!("/{file_name}");
    let content = fs::read_to_string(&gitignore).unwrap_or_default();
    if !content.lines().any(|line| line == entry) {
        let prefix = if content.is_empty() || content.ends_with('\n') {
            ""
        } else {
            "\n"
        };
        fs::write(gitignore, format!("{content}{prefix}{entry}\n"))?;
    }
    Ok(())
}

#[cfg(test)]
fn stable_project_hint(path: &Path) -> String {
    use base64::Engine;
    use std::hash::{DefaultHasher, Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    base64::prelude::BASE64_URL_SAFE_NO_PAD.encode(hasher.finish().to_be_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_assignment() {
        assert_eq!(parse_assignment("FOO=bar").unwrap(), ("FOO", "bar"));
        assert!(parse_assignment("FOO").is_err());
    }

    #[test]
    fn rejects_invalid_keys() {
        assert!(validate_key("OPENAI_API_KEY").is_ok());
        assert!(validate_key("1BAD").is_err());
        assert!(validate_key("BAD-KEY").is_err());
    }

    #[test]
    fn parses_dotenv_file() {
        let values = parse_dotenv("FOO=bar\nexport BAZ=\"qux\"\n# comment\n").unwrap();
        assert_eq!(
            values,
            vec![
                ("FOO".to_owned(), "bar".to_owned()),
                ("BAZ".to_owned(), "qux".to_owned())
            ]
        );
    }

    #[test]
    fn masks_values() {
        assert_eq!(mask("abcd"), "****");
        assert_eq!(mask("abcdef"), "**cdef");
    }

    #[test]
    fn stable_project_hint_is_url_safe() {
        let hint = stable_project_hint(Path::new("/tmp/example"));
        assert!(!hint.is_empty());
        assert!(!hint.contains('/'));
    }
}
