mod share;

use anyhow::{Context, Result, bail};
use clap::{Args, CommandFactory, Parser, Subcommand, ValueEnum, ValueHint};
use clap_complete::{Shell, generate};
use easyenv_core::{
    AppPaths, DesiredScope, DoctorReport, EasyEnv, EnvKey, ResolvedSecret, Scope, mask_value,
};
use easyenv_keychain::ConfiguredSecretStore;
use serde_json::json;
use share::{ensure_identity, export_bundle, import_bundle, rotate_identity};
use std::{io, path::PathBuf, process::Command};

#[derive(Parser, Debug)]
#[command(name = "easyenv", version, about = "env vars done right.")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    Init(InitArgs),
    Set(SetArgs),
    Get(GetArgs),
    #[command(alias = "ls")]
    List(ListArgs),
    #[command(alias = "run")]
    Exec(ExecArgs),
    Push(PushArgs),
    Pull(PullArgs),
    Workspace(WorkspaceArgs),
    Import(ImportArgs),
    Doctor(DoctorArgs),
    Completion(CompletionArgs),
}

#[derive(Args, Debug)]
struct InitArgs {
    #[arg(default_value = ".")]
    path: PathBuf,
}

#[derive(Args, Debug)]
struct SetArgs {
    input: String,
    value: Option<String>,
    #[arg(long)]
    global: bool,
    #[arg(long)]
    project: bool,
    #[arg(long, default_value = "default")]
    profile: String,
}

#[derive(Args, Debug)]
struct GetArgs {
    key: String,
    #[arg(long)]
    reveal: bool,
    #[arg(long)]
    explain: bool,
    #[arg(long)]
    json: bool,
    #[arg(long, default_value = "default")]
    profile: String,
}

#[derive(Args, Debug)]
struct ListArgs {
    #[arg(long, value_enum, default_value = "all")]
    scope: ListScope,
    #[arg(long, value_enum, default_value = "table")]
    format: OutputFormat,
    #[arg(long)]
    reveal: bool,
    #[arg(long, default_value = "default")]
    profile: String,
}

#[derive(Args, Debug)]
struct ExecArgs {
    #[arg(long = "env", value_parser = parse_shell_override)]
    env: Vec<(EnvKey, String)>,
    #[arg(long, default_value = "default")]
    profile: String,
    #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true, num_args = 1.., value_hint = ValueHint::CommandWithArguments)]
    command: Vec<String>,
}

#[derive(Args, Debug)]
struct PushArgs {
    #[arg(long = "to", required = true)]
    to: Vec<String>,
    #[arg(long = "key")]
    key: Vec<String>,
    #[arg(long)]
    all: bool,
    #[arg(long, default_value = "easyenv-share.age")]
    out: PathBuf,
    #[arg(long, default_value = "default")]
    profile: String,
}

#[derive(Args, Debug)]
struct PullArgs {
    path: PathBuf,
    #[arg(long)]
    global: bool,
    #[arg(long)]
    project: bool,
    #[arg(long, default_value = "default")]
    profile: String,
}

#[derive(Args, Debug)]
struct WorkspaceArgs {
    #[command(subcommand)]
    command: WorkspaceCommand,
}

#[derive(Subcommand, Debug)]
enum WorkspaceCommand {
    Show(WorkspaceShowArgs),
    RotateIdentity(WorkspaceShowArgs),
}

#[derive(Args, Debug)]
struct WorkspaceShowArgs {
    #[arg(long)]
    json: bool,
}

#[derive(Args, Debug)]
struct ImportArgs {
    path: PathBuf,
    #[arg(long)]
    global: bool,
    #[arg(long)]
    project: bool,
    #[arg(long)]
    delete: bool,
    #[arg(long, default_value = "default")]
    profile: String,
}

#[derive(Args, Debug)]
struct DoctorArgs {
    #[arg(long)]
    json: bool,
}

#[derive(Args, Debug)]
struct CompletionArgs {
    shell: Shell,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum ListScope {
    All,
    Global,
    Project,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum OutputFormat {
    Table,
    Json,
    Dotenv,
    Shell,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let paths = AppPaths::detect()?;
    let store = ConfiguredSecretStore::from_env(&paths);
    let app = EasyEnv::new(paths, store.clone())?;
    let cwd = std::env::current_dir().context("failed to determine current directory")?;

    match cli.command {
        Commands::Init(args) => {
            let project = app.init_project(&args.path)?;
            println!(
                "initialized project {} ({})",
                project.root_path.display(),
                project.id
            );
        }
        Commands::Set(args) => {
            let desired_scope = desired_scope(args.global, args.project)?;
            let (key, value) = parse_assignment(args.input, args.value)?;
            let stored = app.set_secret(&cwd, desired_scope, &args.profile, key, &value, None)?;
            println!(
                "stored {} in {} scope{}",
                stored.key,
                stored.scope,
                stored
                    .project_root
                    .as_ref()
                    .map(|path| format!(" ({})", path.display()))
                    .unwrap_or_default()
            );
        }
        Commands::Get(args) => {
            let key = EnvKey::parse(args.key)?;
            let Some(secret) = app.get_secret(&cwd, &key, &args.profile)? else {
                bail!("{} not found", key);
            };
            print_get(secret, args.reveal, args.explain, args.json)?;
        }
        Commands::List(args) => {
            let scope = match args.scope {
                ListScope::All => None,
                ListScope::Global => Some(Scope::Global),
                ListScope::Project => Some(Scope::Project),
            };
            let secrets = app.list_secrets(&cwd, scope, &args.profile)?;
            print_list(&secrets, args.format, args.reveal)?;
        }
        Commands::Exec(args) => {
            let secrets = app.resolve_environment(&cwd, &args.profile, &args.env)?;
            let status = run_command(&args.command, &secrets)?;
            drop(secrets);
            std::process::exit(status);
        }
        Commands::Push(args) => {
            let keys = parse_keys(&args.key)?;
            let export = export_bundle(
                &app,
                &store,
                &cwd,
                &args.profile,
                &keys,
                args.all,
                &args.to,
                &args.out,
            )?;
            println!(
                "wrote encrypted share bundle {} for {} recipient(s) with {} secret(s)",
                export.output_path.display(),
                export.recipient_count,
                export.item_count,
            );
        }
        Commands::Pull(args) => {
            let desired_scope = desired_scope(args.global, args.project)?;
            let import =
                import_bundle(&app, &store, &cwd, desired_scope, &args.profile, &args.path)?;
            println!(
                "imported {} shared secret(s) from {} into {} scope",
                import.item_count, import.sender, import.scope,
            );
        }
        Commands::Workspace(args) => match args.command {
            WorkspaceCommand::Show(show) => {
                let identity = ensure_identity(&store)?;
                print_workspace(
                    show.json,
                    &identity.recipient,
                    app.active_project(&cwd)?.as_ref(),
                )?;
            }
            WorkspaceCommand::RotateIdentity(show) => {
                let identity = rotate_identity(&store)?;
                print_workspace(
                    show.json,
                    &identity.recipient,
                    app.active_project(&cwd)?.as_ref(),
                )?;
            }
        },
        Commands::Import(args) => {
            let desired_scope = desired_scope(args.global, args.project)?;
            let outcome =
                app.import_dotenv(&cwd, &args.path, desired_scope, &args.profile, args.delete)?;
            println!(
                "imported {} variables into {} scope{}{}",
                outcome.imported,
                outcome.scope,
                outcome
                    .project_root
                    .as_ref()
                    .map(|path| format!(" ({})", path.display()))
                    .unwrap_or_default(),
                if outcome.deleted_source {
                    "; source deleted"
                } else {
                    ""
                }
            );
        }
        Commands::Doctor(args) => {
            let report = app.doctor(&cwd)?;
            if args.json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                print_doctor(&report);
            }
        }
        Commands::Completion(args) => {
            let mut command = Cli::command();
            generate(args.shell, &mut command, "easyenv", &mut io::stdout());
        }
    }

    Ok(())
}

fn desired_scope(global: bool, project: bool) -> Result<DesiredScope> {
    match (global, project) {
        (true, true) => bail!("--global and --project are mutually exclusive"),
        (true, false) => Ok(DesiredScope::Global),
        (false, true) => Ok(DesiredScope::Project),
        (false, false) => Ok(DesiredScope::Auto),
    }
}

fn parse_assignment(input: String, value: Option<String>) -> Result<(EnvKey, String)> {
    match (input.split_once('='), value) {
        (Some((key, value)), None) => Ok((EnvKey::parse(key.trim())?, value.to_string())),
        (None, Some(value)) => Ok((EnvKey::parse(input)?, value)),
        (Some(_), Some(_)) => bail!("use either KEY=value or KEY VALUE, not both"),
        (None, None) => bail!("missing value; use KEY=value or KEY VALUE"),
    }
}

fn parse_keys(raw_keys: &[String]) -> Result<Vec<EnvKey>> {
    raw_keys.iter().map(|key| EnvKey::parse(key)).collect()
}

fn parse_shell_override(raw: &str) -> Result<(EnvKey, String), String> {
    let Some((key, value)) = raw.split_once('=') else {
        return Err("expected KEY=value".into());
    };
    let key = EnvKey::parse(key.trim()).map_err(|error| error.to_string())?;
    Ok((key, value.to_string()))
}

fn print_get(secret: ResolvedSecret, reveal: bool, explain: bool, json_output: bool) -> Result<()> {
    let rendered = if reveal {
        secret.value.to_string()
    } else {
        mask_value(secret.value.as_str())
    };

    if json_output {
        let payload = json!({
            "key": secret.key.as_str(),
            "scope": secret.scope.as_str(),
            "profile": secret.profile,
            "project_root": secret.project_root.as_ref().map(|path| path.display().to_string()),
            "value": rendered,
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(());
    }

    if explain {
        if let Some(project_root) = secret.project_root.as_ref() {
            println!("source: {} ({})", secret.scope, project_root.display());
        } else {
            println!("source: {}", secret.scope);
        }
    }

    println!("{}={}", secret.key, rendered);
    Ok(())
}

fn print_list(secrets: &[ResolvedSecret], format: OutputFormat, reveal: bool) -> Result<()> {
    match format {
        OutputFormat::Table => {
            println!("KEY\tSCOPE\tPROFILE\tVALUE");
            for secret in secrets {
                let value = if reveal {
                    secret.value.to_string()
                } else {
                    mask_value(secret.value.as_str())
                };
                println!(
                    "{}\t{}\t{}\t{}",
                    secret.key, secret.scope, secret.profile, value
                );
            }
        }
        OutputFormat::Json => {
            let payload: Vec<_> = secrets
                .iter()
                .map(|secret| {
                    json!({
                        "key": secret.key.as_str(),
                        "scope": secret.scope.as_str(),
                        "profile": secret.profile,
                        "project_root": secret.project_root.as_ref().map(|path| path.display().to_string()),
                        "value": if reveal { secret.value.to_string() } else { mask_value(secret.value.as_str()) },
                    })
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&payload)?);
        }
        OutputFormat::Dotenv => {
            for secret in secrets {
                println!("{}={}", secret.key, dotenv_quote(secret.value.as_str()));
            }
        }
        OutputFormat::Shell => {
            for secret in secrets {
                println!(
                    "export {}={}",
                    secret.key,
                    shell_quote(secret.value.as_str())
                );
            }
        }
    }

    Ok(())
}

fn print_workspace(
    json_output: bool,
    recipient: &str,
    active_project: Option<&easyenv_core::ProjectRecord>,
) -> Result<()> {
    if json_output {
        let payload = json!({
            "recipient": recipient,
            "active_project": active_project.map(|project| project.root_path.display().to_string()),
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
    } else {
        println!("recipient: {}", recipient);
        if let Some(project) = active_project {
            println!("active project: {}", project.root_path.display());
        }
        println!("share format: age x25519 armored bundle");
    }
    Ok(())
}

fn dotenv_quote(value: &str) -> String {
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | '/' | ':'))
    {
        return value.to_string();
    }

    let escaped = value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t");
    format!("\"{escaped}\"")
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn run_command(command: &[String], secrets: &[ResolvedSecret]) -> Result<i32> {
    let program = command.first().context("missing command to execute")?;
    let mut child = Command::new(program);
    if command.len() > 1 {
        child.args(&command[1..]);
    }
    for secret in secrets {
        child.env(secret.key.as_str(), secret.value.as_str());
    }
    let status = child
        .status()
        .with_context(|| format!("failed to execute {}", program))?;
    Ok(status.code().unwrap_or(1))
}

fn print_doctor(report: &DoctorReport) {
    println!("data dir: {}", report.data_dir.display());
    println!("db path:  {}", report.db_path.display());
    if let Some(project) = report.active_project.as_ref() {
        println!("project:  {}", project.display());
    }
    if let Ok(sync) = std::env::var("EASYENV_KEYCHAIN_GLOBAL_SYNC") {
        println!("keychain global sync: {}", sync);
    }
    if let Ok(group) = std::env::var("EASYENV_KEYCHAIN_ACCESS_GROUP") {
        println!("keychain access group: {}", group);
    }
    for check in &report.checks {
        println!("- {:?}: {} — {}", check.status, check.name, check.message);
    }
}
