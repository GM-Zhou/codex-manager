use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use base64::Engine as _;
use base64::engine::general_purpose::{URL_SAFE, URL_SAFE_NO_PAD};
use chrono::Utc;
use clap::{Parser, Subcommand};
use colored::Colorize;
use dialoguer::{Select, theme::ColorfulTheme};
use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
#[cfg(target_os = "macos")]
use sha2::{Digest, Sha256};

const TOKEN_ENDPOINT: &str = "https://auth.openai.com/oauth/token";
const USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";
const CODEX_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
#[cfg(target_os = "macos")]
const CODEX_KEYCHAIN_SERVICE: &str = "Codex Auth";
const SHARED_PROFILE_ENTRIES: &[&str] = &[
    "config.toml",
    "session_index.jsonl",
    "sessions",
    "archived_sessions",
    "history.jsonl",
    "vendor_imports",
    "models_cache.json",
    "cache",
    "log",
    "shell_snapshots",
    "version.json",
    ".codex-global-state.json",
];

type AppResult<T> = Result<T, String>;

#[derive(Parser)]
#[command(
    name = "codexm",
    version,
    about = "Codex multi-account CLI manager",
    long_about = "Manage multiple Codex accounts from command line, switch accounts, inspect quotas, and start Codex in current directory with a selected account.",
    after_help = "Examples:\n  codexm\n  codexm new\n  codexm new user@example.com\n  codexm add\n  codexm ls\n  codexm list\n  codexm switch user@example.com\n  codexm switch --force-refresh user@example.com\n  codexm delete user@example.com\n  codexm rm\n\nNotes:\n  - Running `codexm` without subcommand is equivalent to `codexm new`.\n  - Account data is stored under ~/.codex-manager.\n  - `switch` updates ~/.codex/auth.json and macOS Keychain for the default profile.\n  - `new` launches Codex with an isolated CODEX_HOME per account.\n  - In interactive account picker, press Esc or Ctrl+C to cancel silently."
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    #[command(
        about = "Add account via `codex login`",
        long_about = "Start official `codex login` flow, import tokens into local account store, and keep current active account unchanged."
    )]
    Add,
    #[command(
        about = "List all accounts and quotas",
        long_about = "Fetch and display quota status for all stored accounts. The current account is marked with `*`.",
        visible_alias = "list"
    )]
    Ls,
    #[command(
        about = "Switch account",
        long_about = "Switch active account for default Codex profile by writing ~/.codex/auth.json and syncing macOS Keychain. If no identifier is provided, choose from an interactive list."
    )]
    Switch {
        #[arg(
            help = "Account identifier (name / email / id)",
            long_help = "Account identifier used to match an account. Supports exact value or partial match across name, email, and id.\n\nExamples:\n  codexm switch user@example.com\n  codexm switch codex_abcd1234\n  codexm switch --force-refresh user@example.com"
        )]
        name: Option<String>,
        #[arg(
            long = "force-refresh",
            help = "Force refresh token before switching",
            long_help = "Refresh token before switching even when access_token is not expired. Useful when account subscription or permission changed recently."
        )]
        force_refresh: bool,
    },
    #[command(
        about = "Start Codex in current directory with selected account",
        long_about = "Launch Codex in current working directory using an isolated per-account profile directory under ~/.codex-manager/instances. If no email is provided, choose account interactively."
    )]
    New {
        #[arg(
            help = "Account email (optional; choose interactively when omitted)",
            long_help = "Account email to use for this launch. When omitted, an interactive account picker is shown.\n\nExamples:\n  codexm new\n  codexm new user@example.com"
        )]
        email: Option<String>,
    },
    #[command(
        about = "Delete account and local workspace",
        long_about = "Delete account from local store and remove its local workspace under ~/.codex-manager/instances. If no identifier is provided, choose account interactively.",
        visible_alias = "rm"
    )]
    Delete {
        #[arg(
            help = "Account identifier (name / email / id)",
            long_help = "Account identifier used to match an account. Supports exact value or partial match across name, email, and id.\n\nExamples:\n  codexm delete user@example.com\n  codexm rm user@example.com\n  codexm delete"
        )]
        email: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AccountIndex {
    version: String,
    current_account_id: Option<String>,
    accounts: Vec<AccountSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AccountSummary {
    id: String,
    name: String,
    email: String,
    created_at: i64,
    last_used: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredAccount {
    id: String,
    name: String,
    email: String,
    account_id: Option<String>,
    organization_id: Option<String>,
    tokens: Tokens,
    plan_type: Option<String>,
    quota: Option<Quota>,
    quota_error: Option<String>,
    created_at: i64,
    last_used: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Tokens {
    id_token: String,
    access_token: String,
    refresh_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Quota {
    primary_percentage: i32,
    primary_reset_time: Option<i64>,
    primary_window_minutes: Option<i64>,
    primary_present: Option<bool>,
    secondary_percentage: i32,
    secondary_reset_time: Option<i64>,
    secondary_window_minutes: Option<i64>,
    secondary_present: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct AuthFile {
    tokens: Option<AuthTokens>,
}

#[derive(Debug, Deserialize, Serialize)]
struct AuthTokens {
    id_token: String,
    access_token: String,
    refresh_token: Option<String>,
    account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UsageResponse {
    plan_type: Option<String>,
    rate_limit: Option<RateLimitInfo>,
}

#[derive(Debug, Deserialize)]
struct RateLimitInfo {
    primary_window: Option<WindowInfo>,
    secondary_window: Option<WindowInfo>,
}

#[derive(Debug, Deserialize)]
struct WindowInfo {
    used_percent: Option<i32>,
    limit_window_seconds: Option<i64>,
    reset_after_seconds: Option<i64>,
    reset_at: Option<i64>,
}

#[tokio::main]
async fn main() {
    let _ = ctrlc::set_handler(|| {
        ensure_terminal_cursor_visible();
        std::process::exit(0);
    });

    if let Err(error) = run().await {
        ensure_terminal_cursor_visible();
        if is_silent_cancel_error(&error) {
            std::process::exit(0);
        }
        eprintln!("Error: {}", error);
        std::process::exit(1);
    }
}

async fn run() -> AppResult<()> {
    let cli = Cli::parse();
    match cli.command {
        Some(Commands::Add) => add_account().await,
        Some(Commands::Ls) => list_accounts_with_quota().await,
        Some(Commands::Switch {
            name,
            force_refresh,
        }) => switch_account(name, force_refresh).await,
        Some(Commands::New { email }) => start_codex_with_account(email).await,
        Some(Commands::Delete { email }) => delete_account(email).await,
        None => start_codex_with_account(None).await,
    }
}

async fn add_account() -> AppResult<()> {
    ensure_codex_cli_available()?;
    let state = load_state()?;
    let temp_dir = state.base_dir.join(format!("login-tmp-{}", Utc::now().timestamp_millis()));
    fs::create_dir_all(&temp_dir).map_err(|e| format!("Failed to create temp directory: {}", e))?;

    println!("Starting `codex login`, please complete authorization in browser...");
    let status = Command::new("codex")
        .arg("login")
        .env("CODEX_HOME", &temp_dir)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| format!("Failed to start codex login: {}", e))?;

    if !status.success() {
        let _ = fs::remove_dir_all(&temp_dir);
        return Err("`codex login` did not complete successfully".to_string());
    }

    let auth_path = temp_dir.join("auth.json");
    let auth_content =
        fs::read_to_string(&auth_path).map_err(|e| format!("Failed to read temp auth.json: {}", e))?;
    let auth_file: AuthFile =
        serde_json::from_str(&auth_content).map_err(|e| format!("Failed to parse auth.json: {}", e))?;
    let tokens = auth_file
        .tokens
        .ok_or_else(|| "auth.json is missing tokens, cannot import account".to_string())?;
    let AuthTokens {
        id_token,
        access_token,
        refresh_token,
        account_id: token_account_id,
    } = tokens;

    let email = extract_email_from_id_token(&id_token)
        .ok_or_else(|| "Failed to parse email from id_token".to_string())?;
    let account_id = token_account_id.or_else(|| extract_chatgpt_account_id(&access_token));
    let organization_id = extract_chatgpt_organization_id(&access_token);
    let account_name = email.clone();
    let account_storage_id = build_account_storage_id(&email, account_id.as_deref(), organization_id.as_deref());
    let now = Utc::now().timestamp();

    let mut index = load_index(&state)?;
    let mut account = load_account(&state, &account_storage_id).unwrap_or(StoredAccount {
        id: account_storage_id.clone(),
        name: account_name.clone(),
        email: email.clone(),
        account_id: account_id.clone(),
        organization_id: organization_id.clone(),
        tokens: Tokens {
            id_token: id_token.clone(),
            access_token: access_token.clone(),
            refresh_token: refresh_token.clone(),
        },
        plan_type: None,
        quota: None,
        quota_error: None,
        created_at: now,
        last_used: now,
    });

    account.name = account_name;
    account.email = email.clone();
    account.account_id = account_id;
    account.organization_id = organization_id;
    account.tokens = Tokens {
        id_token,
        access_token,
        refresh_token,
    };
    account.last_used = now;

    save_account(&state, &account)?;
    upsert_summary(&mut index, &account);
    save_index(&state, &index)?;

    let _ = fs::remove_dir_all(&temp_dir);

    println!("Added successfully: {} ({})", account.name, account.email);
    println!("Account saved, current account remains unchanged");
    Ok(())
}

async fn list_accounts_with_quota() -> AppResult<()> {
    let state = load_state()?;
    let mut index = load_index(&state)?;
    if index.accounts.is_empty() {
        println!("No accounts found, run: codexm add");
        return Ok(());
    }

    println!("Refreshing account quotas...");
    refresh_all_accounts_cache_for_picker(&state, &mut index).await?;

    let mut accounts = Vec::new();
    for summary in &index.accounts {
        if let Some(account) = load_account(&state, &summary.id) {
            accounts.push(account);
        }
    }

    for account in accounts {
        let is_current = index.current_account_id.as_deref() == Some(account.id.as_str());
        let current_mark = if is_current {
            "*".bright_green().bold().to_string()
        } else {
            " ".to_string()
        };
        println!("{} {}", current_mark, account.email.bright_black());
        println!(
            "  {} {}",
            "id:".bright_black(),
            account.id.bright_black()
        );
        if let Some(plan) = &account.plan_type {
            println!("  {} {}", "plan:".bright_black(), color_plan(plan));
        }
        if let Some(quota) = &account.quota {
            println!(
                "  {} {} ({}), {} {} ({})",
                "5h quota:".bright_black(),
                color_percentage(quota.primary_percentage),
                format_reset(quota.primary_reset_time).bright_black(),
                "weekly quota:".bright_black(),
                color_percentage(quota.secondary_percentage),
                format_reset(quota.secondary_reset_time),
            );
        } else if let Some(err) = &account.quota_error {
            println!(
                "  {} {}",
                "quota:".bright_black(),
                format!("failed - {}", err).red()
            );
        } else {
            println!("  {} {}", "quota:".bright_black(), "no data".yellow());
        }
        println!();
    }

    Ok(())
}

async fn switch_account(name: Option<String>, force_refresh: bool) -> AppResult<()> {
    let state = load_state()?;
    let mut index = load_index(&state)?;
    if index.accounts.is_empty() {
        return Err("No accounts found, run `codexm add` first".to_string());
    }

    let selected_id = if let Some(name) = name {
        resolve_account_id_by_name(&state, &index, &name)?
    } else {
        println!("Refreshing account quotas...");
        refresh_all_accounts_cache_for_picker(&state, &mut index).await?;
        pick_account_interactively(&state, &index)?
    };

    let mut account = load_account(&state, &selected_id)
        .ok_or_else(|| format!("Account not found: {}", selected_id))?;

    if force_refresh {
        let refresh_token = account
            .tokens
            .refresh_token
            .clone()
            .ok_or_else(|| "force-refresh requested but refresh_token is missing".to_string())?;
        account.tokens = refresh_access_token(&refresh_token).await?;
    } else if is_token_expired(&account.tokens.access_token) {
        if let Some(refresh_token) = account.tokens.refresh_token.clone() {
            account.tokens = refresh_access_token(&refresh_token).await?;
        }
    }

    if let Err(err) = refresh_account_quota(&mut account).await {
        account.quota_error = Some(err);
    }

    write_auth_json_for_account(&account)?;
    account.last_used = Utc::now().timestamp();
    save_account(&state, &account)?;
    upsert_summary(&mut index, &account);
    index.current_account_id = Some(account.id.clone());
    save_index(&state, &index)?;

    println!("Switched to account: {} ({})", account.name, account.email);
    Ok(())
}

async fn refresh_all_accounts_cache_for_picker(
    state: &State,
    index: &mut AccountIndex,
) -> AppResult<()> {
    let account_ids: Vec<String> = index.accounts.iter().map(|item| item.id.clone()).collect();
    for account_id in account_ids {
        let Some(mut account) = load_account(state, &account_id) else {
            continue;
        };
        if let Err(err) = refresh_account_quota(&mut account).await {
            account.quota_error = Some(err);
        }
        save_account(state, &account)?;
        upsert_summary(index, &account);
    }
    save_index(state, index)?;
    Ok(())
}

async fn delete_account(email: Option<String>) -> AppResult<()> {
    let state = load_state()?;
    let mut index = load_index(&state)?;
    if index.accounts.is_empty() {
        return Err("No accounts found, run `codexm add` first".to_string());
    }

    let selected_id = if let Some(email) = email {
        resolve_account_id_by_name(&state, &index, &email)?
    } else {
        pick_account_interactively(&state, &index)?
    };

    let account = load_account(&state, &selected_id)
        .ok_or_else(|| format!("Account not found: {}", selected_id))?;

    let confirm_items = vec![
        format!("ok - delete {}", account.email),
        "cancel".to_string(),
    ];
    let confirm_choice = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Confirm account deletion")
        .items(&confirm_items)
        .default(1)
        .interact_opt()
        .map_err(|e| {
            let message = e.to_string();
            let lower = message.to_ascii_lowercase();
            if lower.contains("interrupted")
                || lower.contains("cancelled")
                || lower.contains("canceled")
            {
                "Selection cancelled".to_string()
            } else {
                format!("Failed to confirm account deletion: {}", e)
            }
        })?
        .ok_or_else(|| "Selection cancelled".to_string())?;

    if confirm_choice != 0 {
        return Err("Deletion cancelled".to_string());
    }

    let account_file_path = state.accounts_dir.join(format!("{}.json", account.id));
    if account_file_path.exists() {
        fs::remove_file(&account_file_path).map_err(|e| {
            format!(
                "Failed to delete account file ({}): {}",
                account_file_path.display(),
                e
            )
        })?;
    }

    let instance_dir = state.instances_dir.join(&account.id);
    if instance_dir.exists() {
        fs::remove_dir_all(&instance_dir).map_err(|e| {
            format!(
                "Failed to delete account workspace ({}): {}",
                instance_dir.display(),
                e
            )
        })?;
    }
    delete_codex_keychain_for_dir(&instance_dir)?;

    index.accounts.retain(|item| item.id != account.id);
    if index.current_account_id.as_deref() == Some(account.id.as_str()) {
        index.current_account_id = None;
    }
    save_index(&state, &index)?;

    println!("Deleted account: {}", account.email);
    Ok(())
}

async fn start_codex_with_account(email: Option<String>) -> AppResult<()> {
    ensure_codex_cli_available()?;

    let state = load_state()?;
    let mut index = load_index(&state)?;
    if index.accounts.is_empty() {
        return Err("No accounts found, run `codexm add` first".to_string());
    }

    let selected_id = if let Some(email) = email {
        resolve_account_id_by_name(&state, &index, &email)?
    } else {
        pick_account_interactively(&state, &index)?
    };

    let mut account = load_account(&state, &selected_id)
        .ok_or_else(|| format!("Account not found: {}", selected_id))?;
    if is_token_expired(&account.tokens.access_token) {
        if let Some(refresh_token) = account.tokens.refresh_token.clone() {
            account.tokens = refresh_access_token(&refresh_token).await?;
        } else {
            return Err("access_token expired and refresh_token is missing".to_string());
        }
    }

    account.last_used = Utc::now().timestamp();
    save_account(&state, &account)?;
    upsert_summary(&mut index, &account);
    save_index(&state, &index)?;

    let profile_dir = state.instances_dir.join(&account.id);
    fs::create_dir_all(&profile_dir)
        .map_err(|e| format!("Failed to create instance profile directory: {}", e))?;
    let default_home = resolve_codex_home();
    ensure_profile_shared_links(&profile_dir, &default_home)?;
    write_auth_json_for_account_to_dir(&account, &profile_dir)?;

    let cwd = std::env::current_dir().map_err(|e| format!("Failed to get current directory: {}", e))?;
    println!(
        "Starting codex in current directory with account {}",
        account.email
    );
    let status = Command::new("codex")
        .env("CODEX_HOME", &profile_dir)
        .current_dir(&cwd)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| format!("Failed to start codex: {}", e))?;

    if !status.success() {
        return Err(format!("codex exited with non-zero status: {}", status));
    }
    Ok(())
}

fn ensure_profile_shared_links(profile_dir: &Path, default_home: &Path) -> AppResult<()> {
    if profile_dir == default_home {
        return Ok(());
    }

    let backup_root = profile_dir
        .join(".codexm-local-backup")
        .join(Utc::now().format("%Y%m%d-%H%M%S").to_string());
    let mut copy_fallback_entries: Vec<String> = Vec::new();

    for relative in SHARED_PROFILE_ENTRIES {
        let source = default_home.join(relative);
        if !source.exists() {
            continue;
        }

        let target = profile_dir.join(relative);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                format!(
                    "Failed to create shared-entry parent directory ({}): {}",
                    parent.display(),
                    e
                )
            })?;
        }

        if let Ok(metadata) = fs::symlink_metadata(&target) {
            if metadata.file_type().is_symlink() {
                if let Ok(link_to) = fs::read_link(&target) {
                    if link_to == source {
                        continue;
                    }
                }
                remove_existing_path(&target, &metadata)?;
            } else {
                let backup_path = backup_root.join(relative);
                if let Some(parent) = backup_path.parent() {
                    fs::create_dir_all(parent).map_err(|e| {
                        format!(
                            "Failed to create backup directory ({}): {}",
                            parent.display(),
                            e
                        )
                    })?;
                }
                fs::rename(&target, &backup_path).map_err(|e| {
                    format!(
                        "Failed to backup existing path before linking ({} -> {}): {}",
                        target.display(),
                        backup_path.display(),
                        e
                    )
                })?;
            }
        }

        let used_copy_fallback = create_symlink(&source, &target, source.is_dir())?;
        if used_copy_fallback {
            copy_fallback_entries.push(relative.to_string());
        }
    }

    warn_copy_fallback_if_needed(&copy_fallback_entries);
    Ok(())
}

fn remove_existing_path(path: &Path, metadata: &fs::Metadata) -> AppResult<()> {
    if metadata.is_dir() {
        fs::remove_dir_all(path).map_err(|e| {
            format!(
                "Failed to remove existing directory before relinking ({}): {}",
                path.display(),
                e
            )
        })
    } else {
        fs::remove_file(path).map_err(|e| {
            format!(
                "Failed to remove existing file before relinking ({}): {}",
                path.display(),
                e
            )
        })
    }
}

#[cfg(unix)]
fn create_symlink(source: &Path, target: &Path, _is_dir: bool) -> AppResult<bool> {
    std::os::unix::fs::symlink(source, target).map_err(|e| {
        format!(
            "Failed to create symlink ({} -> {}): {}",
            target.display(),
            source.display(),
            e
        )
    })?;
    Ok(false)
}

#[cfg(windows)]
fn create_symlink(source: &Path, target: &Path, is_dir: bool) -> AppResult<bool> {
    let symlink_result = if is_dir {
        std::os::windows::fs::symlink_dir(source, target)
    } else {
        std::os::windows::fs::symlink_file(source, target)
    };

    if symlink_result.is_ok() {
        return Ok(false);
    }

    // On Windows, symlink may fail without Developer Mode/admin permission.
    // Fallback to copy to keep feature usable.
    if is_dir {
        copy_dir_recursive(source, target).map_err(|e| {
            format!(
                "Failed to create symlink and fallback copy for directory ({} -> {}): {}",
                source.display(),
                target.display(),
                e
            )
        })?;
    } else {
        fs::copy(source, target).map_err(|e| {
            format!(
                "Failed to create symlink and fallback copy for file ({} -> {}): {}",
                source.display(),
                target.display(),
                e
            )
        })?;
    }
    Ok(true)
}

#[cfg(windows)]
fn copy_dir_recursive(source: &Path, target: &Path) -> Result<(), std::io::Error> {
    fs::create_dir_all(target)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        let metadata = entry.metadata()?;
        if metadata.is_dir() {
            copy_dir_recursive(&source_path, &target_path)?;
        } else {
            fs::copy(&source_path, &target_path)?;
        }
    }
    Ok(())
}

#[cfg(windows)]
fn warn_copy_fallback_if_needed(entries: &[String]) {
    if entries.is_empty() {
        return;
    }
    eprintln!(
        "[codexm warning] Symlink permission is unavailable on this Windows environment, so some shared paths were copied instead: {}",
        entries.join(", ")
    );
    eprintln!(
        "[codexm warning] While using copy fallback, writes to those paths will NOT sync back to the global workspace (~/.codex)."
    );
    eprintln!(
        "[codexm warning] To enable true sync, enable Windows Developer Mode or run terminal as Administrator, then start codexm again."
    );
}

#[cfg(not(windows))]
fn warn_copy_fallback_if_needed(_entries: &[String]) {}

fn resolve_account_id_by_name(state: &State, index: &AccountIndex, name: &str) -> AppResult<String> {
    let needle = name.trim();
    if needle.is_empty() {
        return Err("Account name cannot be empty".to_string());
    }

    let mut exact = Vec::new();
    let mut fuzzy = Vec::new();
    for summary in &index.accounts {
        if summary.id == needle || summary.name == needle || summary.email == needle {
            exact.push(summary.id.clone());
            continue;
        }
        if summary.id.starts_with(needle)
            || summary.name.contains(needle)
            || summary.email.contains(needle)
        {
            fuzzy.push(summary.id.clone());
        }
    }

    if exact.len() == 1 {
        return Ok(exact[0].clone());
    }
    if exact.len() > 1 {
        return Err("Multiple exact matches found, please use a more specific name/email/id".to_string());
    }
    if fuzzy.len() == 1 {
        return Ok(fuzzy[0].clone());
    }
    if fuzzy.is_empty() {
        return Err(format!("Account not found: {}", needle));
    }

    let mut names = Vec::new();
    for id in &fuzzy {
        if let Some(account) = load_account(state, id) {
            names.push(account.email);
        }
    }
    Err(format!("Multiple accounts matched: {}", names.join(", ")))
}

fn pick_account_interactively(state: &State, index: &AccountIndex) -> AppResult<String> {
    let mut items = Vec::new();
    let mut ids = Vec::new();
    let mut default_index = 0usize;

    for (i, summary) in index.accounts.iter().enumerate() {
        let is_current = index.current_account_id.as_deref() == Some(summary.id.as_str());
        let marker = if is_current {
            "[current]".to_string()
        } else {
            String::new()
        };
        let account = load_account(state, &summary.id);
        let plan_text = account
            .as_ref()
            .and_then(|a| a.plan_type.as_deref())
            .map(|v| v.to_string())
            .unwrap_or_else(|| "-".to_string());
        let quota_text = account
            .as_ref()
            .map(format_quota_hint_for_picker)
            .unwrap_or_else(|| "quota n/a".to_string());

        items.push(format!(
            "{}  plan:{}  {} {}",
            summary.email,
            plan_text,
            quota_text,
            marker
        ));
        ids.push(summary.id.clone());
        if is_current {
            default_index = i;
        }
    }

    let choice = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Select account")
        .items(&items)
        .default(default_index)
        .interact_opt()
        .map_err(|e| {
            let message = e.to_string();
            let lower = message.to_ascii_lowercase();
            if lower.contains("interrupted")
                || lower.contains("cancelled")
                || lower.contains("canceled")
            {
                "Selection cancelled".to_string()
            } else {
                format!("Failed to choose account: {}", e)
            }
        })?
        .ok_or_else(|| "Selection cancelled".to_string());
    ensure_terminal_cursor_visible();
    let choice = choice?;

    Ok(ids[choice].clone())
}

fn is_silent_cancel_error(error: &str) -> bool {
    matches!(
        error.trim(),
        "Selection cancelled" | "Operation cancelled" | "Canceled" | "Deletion cancelled"
    )
}

fn ensure_terminal_cursor_visible() {
    let _ = std::io::stdout().write_all(b"\x1b[?25h");
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().write_all(b"\x1b[?25h");
    let _ = std::io::stderr().flush();
}

async fn refresh_account_quota(account: &mut StoredAccount) -> AppResult<()> {
    if is_token_expired(&account.tokens.access_token) {
        if let Some(refresh_token) = account.tokens.refresh_token.clone() {
            account.tokens = refresh_access_token(&refresh_token).await?;
        } else {
            return Err("access_token expired and refresh_token is missing".to_string());
        }
    }

    match fetch_usage(account).await {
        Ok((quota, plan_type)) => {
            account.quota = Some(quota);
            account.quota_error = None;
            if plan_type.is_some() {
                account.plan_type = plan_type;
            }
            Ok(())
        }
        Err(err) => {
            if should_force_refresh(&err) {
                if let Some(refresh_token) = account.tokens.refresh_token.clone() {
                    account.tokens = refresh_access_token(&refresh_token).await?;
                    let (quota, plan_type) = fetch_usage(account).await?;
                    account.quota = Some(quota);
                    account.quota_error = None;
                    if plan_type.is_some() {
                        account.plan_type = plan_type;
                    }
                    Ok(())
                } else {
                    Err(err)
                }
            } else {
                Err(err)
            }
        }
    }
}

async fn fetch_usage(account: &StoredAccount) -> AppResult<(Quota, Option<String>)> {
    let client = reqwest::Client::new();
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {}", account.tokens.access_token))
            .map_err(|e| format!("Failed to build Authorization header: {}", e))?,
    );
    headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
    let account_id = account
        .account_id
        .clone()
        .or_else(|| extract_chatgpt_account_id(&account.tokens.access_token));
    if let Some(acc_id) = account_id {
        if !acc_id.is_empty() {
            headers.insert(
                "ChatGPT-Account-Id",
                HeaderValue::from_str(&acc_id)
                    .map_err(|e| format!("Failed to build ChatGPT-Account-Id header: {}", e))?,
            );
        }
    }

    let response = client
        .get(USAGE_URL)
        .headers(headers)
        .send()
        .await
        .map_err(|e| format!("Failed to request quota: {}", e))?;

    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|e| format!("Failed to read quota response: {}", e))?;
    if !status.is_success() {
        return Err(format!("Quota API error {}: {}", status, body));
    }

    let usage: UsageResponse =
        serde_json::from_str(&body).map_err(|e| format!("Failed to parse quota JSON response: {}", e))?;
    let rate_limit = usage.rate_limit;
    let primary = rate_limit.as_ref().and_then(|r| r.primary_window.as_ref());
    let secondary = rate_limit.as_ref().and_then(|r| r.secondary_window.as_ref());

    let quota = Quota {
        primary_percentage: primary.map(remaining_percentage).unwrap_or(100),
        primary_reset_time: primary.and_then(reset_time),
        primary_window_minutes: primary.and_then(window_minutes),
        primary_present: Some(primary.is_some()),
        secondary_percentage: secondary.map(remaining_percentage).unwrap_or(100),
        secondary_reset_time: secondary.and_then(reset_time),
        secondary_window_minutes: secondary.and_then(window_minutes),
        secondary_present: Some(secondary.is_some()),
    };

    Ok((quota, usage.plan_type))
}

async fn refresh_access_token(refresh_token: &str) -> AppResult<Tokens> {
    let client = reqwest::Client::new();
    let params = [
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("client_id", CODEX_CLIENT_ID),
    ];

    let response = client
        .post(TOKEN_ENDPOINT)
        .form(&params)
        .send()
        .await
        .map_err(|e| format!("Failed to request token refresh: {}", e))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|e| format!("Failed to read refresh response: {}", e))?;
    if !status.is_success() {
        return Err(format!("Token refresh failed {}: {}", status, body));
    }

    let value: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("Failed to parse refresh response: {}", e))?;
    let id_token = value
        .get("id_token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Refresh response missing id_token".to_string())?
        .to_string();
    let access_token = value
        .get("access_token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Refresh response missing access_token".to_string())?
        .to_string();
    let new_refresh = value
        .get("refresh_token")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| Some(refresh_token.to_string()));

    Ok(Tokens {
        id_token,
        access_token,
        refresh_token: new_refresh,
    })
}

fn write_auth_json_for_account(account: &StoredAccount) -> AppResult<()> {
    let codex_home = resolve_codex_home();
    fs::create_dir_all(&codex_home)
        .map_err(|e| format!("Failed to create ~/.codex directory: {}", e))?;
    write_auth_json_for_account_to_dir(account, &codex_home)
}

fn write_auth_json_for_account_to_dir(account: &StoredAccount, target_dir: &Path) -> AppResult<()> {
    fs::create_dir_all(target_dir).map_err(|e| {
        format!(
            "Failed to create auth target directory ({}): {}",
            target_dir.display(),
            e
        )
    })?;
    let auth_file = serde_json::json!({
        "tokens": {
            "id_token": account.tokens.id_token,
            "access_token": account.tokens.access_token,
            "refresh_token": account.tokens.refresh_token,
            "account_id": account.account_id
        },
        "last_refresh": Utc::now().to_rfc3339(),
    });
    let content =
        serde_json::to_string_pretty(&auth_file).map_err(|e| format!("Failed to serialize auth.json: {}", e))?;
    fs::write(target_dir.join("auth.json"), content)
        .map_err(|e| format!("Failed to write auth.json: {}", e))?;

    write_codex_keychain_to_dir(target_dir, &auth_file)?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn build_codex_keychain_account(base_dir: &Path) -> String {
    let resolved = fs::canonicalize(base_dir).unwrap_or_else(|_| base_dir.to_path_buf());
    let mut hasher = Sha256::new();
    hasher.update(resolved.to_string_lossy().as_bytes());
    let digest = hasher.finalize();
    let digest_hex = format!("{:x}", digest);
    format!("cli|{}", &digest_hex[..16])
}

#[cfg(target_os = "macos")]
fn write_codex_keychain_to_dir(base_dir: &Path, payload: &serde_json::Value) -> AppResult<()> {
    let secret = serde_json::to_string(payload)
        .map_err(|e| format!("Failed to serialize keychain payload: {}", e))?;
    let keychain_account = build_codex_keychain_account(base_dir);
    let output = Command::new("security")
        .arg("add-generic-password")
        .arg("-U")
        .arg("-s")
        .arg(CODEX_KEYCHAIN_SERVICE)
        .arg("-a")
        .arg(&keychain_account)
        .arg("-w")
        .arg(&secret)
        .output()
        .map_err(|e| format!("Failed to run security command: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(format!(
            "Failed to sync keychain (service={}, account={}): status={}, stderr={}, stdout={}",
            CODEX_KEYCHAIN_SERVICE,
            keychain_account,
            output.status,
            if stderr.trim().is_empty() {
                "<empty>"
            } else {
                stderr.trim()
            },
            if stdout.trim().is_empty() {
                "<empty>"
            } else {
                stdout.trim()
            }
        ));
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn write_codex_keychain_to_dir(_base_dir: &Path, _payload: &serde_json::Value) -> AppResult<()> {
    Ok(())
}

#[cfg(target_os = "macos")]
fn delete_codex_keychain_for_dir(base_dir: &Path) -> AppResult<()> {
    let keychain_account = build_codex_keychain_account(base_dir);
    let output = Command::new("security")
        .arg("delete-generic-password")
        .arg("-s")
        .arg(CODEX_KEYCHAIN_SERVICE)
        .arg("-a")
        .arg(&keychain_account)
        .output()
        .map_err(|e| format!("Failed to run security delete command: {}", e))?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).to_ascii_lowercase();
    // Ignore not-found cases.
    if stderr.contains("could not be found") || stderr.contains("the specified item could not be found") {
        return Ok(());
    }

    Err(format!(
        "Failed to delete keychain item (service={}, account={}): status={}, stderr={}",
        CODEX_KEYCHAIN_SERVICE,
        keychain_account,
        output.status,
        String::from_utf8_lossy(&output.stderr).trim()
    ))
}

#[cfg(not(target_os = "macos"))]
fn delete_codex_keychain_for_dir(_base_dir: &Path) -> AppResult<()> {
    Ok(())
}

fn ensure_codex_cli_available() -> AppResult<()> {
    let status = Command::new("codex")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|_| "codex command not found, please install it and ensure it is in PATH".to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err("codex command is not available, please check your installation".to_string())
    }
}

fn remaining_percentage(window: &WindowInfo) -> i32 {
    100 - window.used_percent.unwrap_or(0).clamp(0, 100)
}

fn window_minutes(window: &WindowInfo) -> Option<i64> {
    let seconds = window.limit_window_seconds?;
    if seconds <= 0 {
        return None;
    }
    Some((seconds + 59) / 60)
}

fn reset_time(window: &WindowInfo) -> Option<i64> {
    if let Some(reset_at) = window.reset_at {
        return Some(reset_at);
    }
    let after = window.reset_after_seconds?;
    if after < 0 {
        return None;
    }
    Some(Utc::now().timestamp() + after)
}

fn should_force_refresh(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("token_invalidated")
        || lower.contains("your authentication token has been invalidated")
        || lower.contains("401 unauthorized")
}

fn is_token_expired(access_token: &str) -> bool {
    let payload = decode_jwt_payload(access_token);
    let Some(exp) = payload
        .and_then(|v| v.get("exp").and_then(|exp| exp.as_i64()))
    else {
        return true;
    };
    exp < Utc::now().timestamp() + 60
}

fn extract_email_from_id_token(id_token: &str) -> Option<String> {
    decode_jwt_payload(id_token)?
        .get("email")?
        .as_str()
        .map(|s| s.to_string())
}

fn extract_chatgpt_account_id(access_token: &str) -> Option<String> {
    let payload = decode_jwt_payload(access_token)?;
    payload
        .get("https://api.openai.com/auth")?
        .get("chatgpt_account_id")?
        .as_str()
        .map(|s| s.to_string())
}

fn extract_chatgpt_organization_id(access_token: &str) -> Option<String> {
    let payload = decode_jwt_payload(access_token)?;
    let auth = payload.get("https://api.openai.com/auth")?;
    for key in [
        "organization_id",
        "chatgpt_organization_id",
        "chatgpt_org_id",
        "org_id",
    ] {
        if let Some(value) = auth.get(key).and_then(|v| v.as_str()) {
            if !value.trim().is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

fn decode_jwt_payload(token: &str) -> Option<serde_json::Value> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() < 2 {
        return None;
    }
    let payload = parts[1];
    let bytes = URL_SAFE_NO_PAD
        .decode(payload)
        .or_else(|_| URL_SAFE.decode(payload))
        .ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn build_account_storage_id(email: &str, account_id: Option<&str>, org_id: Option<&str>) -> String {
    let mut seed = email.trim().to_string();
    if let Some(id) = account_id {
        let trimmed = id.trim();
        if !trimmed.is_empty() {
            seed.push('|');
            seed.push_str(trimmed);
        }
    }
    if let Some(id) = org_id {
        let trimmed = id.trim();
        if !trimmed.is_empty() {
            seed.push('|');
            seed.push_str(trimmed);
        }
    }
    format!("codex_{:x}", md5::compute(seed.as_bytes()))
}

fn format_reset(timestamp: Option<i64>) -> String {
    match timestamp {
        Some(value) => chrono::DateTime::<chrono::Local>::from(
            std::time::UNIX_EPOCH + std::time::Duration::from_secs(value.max(0) as u64),
        )
        .format("%m-%d %H:%M")
        .to_string(),
        None => "-".to_string(),
    }
}

fn color_percentage(percentage: i32) -> String {
    let text = format!("{}%", percentage);
    if percentage >= 80 {
        text.bright_green().bold().to_string()
    } else if percentage >= 50 {
        text.cyan().to_string()
    } else if percentage >= 20 {
        text.yellow().to_string()
    } else {
        text.red().bold().to_string()
    }
}

fn color_plan(plan: &str) -> String {
    match plan.trim().to_ascii_lowercase().as_str() {
        "free" => plan.bright_black().to_string(),
        "plus" | "pro" => plan.bright_cyan().bold().to_string(),
        "team" | "business" | "enterprise" => plan.bright_magenta().bold().to_string(),
        _ => plan.normal().to_string(),
    }
}

fn format_quota_hint_for_picker(account: &StoredAccount) -> String {
    if let Some(quota) = &account.quota {
        return format!(
            "5h:{}% weekly:{}%",
            quota.primary_percentage,
            quota.secondary_percentage
        );
    }
    if account.quota_error.is_some() {
        return "quota error".to_string();
    }
    "quota n/a".to_string()
}

struct State {
    base_dir: PathBuf,
    accounts_dir: PathBuf,
    instances_dir: PathBuf,
    index_path: PathBuf,
}

fn load_state() -> AppResult<State> {
    let base_dir = dirs::home_dir()
        .ok_or_else(|| "Failed to get user home directory".to_string())?
        .join(".codex-manager");
    let accounts_dir = base_dir.join("accounts");
    let instances_dir = base_dir.join("instances");
    let index_path = base_dir.join("index.json");
    fs::create_dir_all(&accounts_dir).map_err(|e| format!("Failed to create data directory: {}", e))?;
    fs::create_dir_all(&instances_dir)
        .map_err(|e| format!("Failed to create instances directory: {}", e))?;
    Ok(State {
        base_dir,
        accounts_dir,
        instances_dir,
        index_path,
    })
}

fn load_index(state: &State) -> AppResult<AccountIndex> {
    if !state.index_path.exists() {
        return Ok(AccountIndex {
            version: "1".to_string(),
            current_account_id: None,
            accounts: vec![],
        });
    }
    let content = fs::read_to_string(&state.index_path).map_err(|e| format!("Failed to read index: {}", e))?;
    if content.trim().is_empty() {
        return Ok(AccountIndex {
            version: "1".to_string(),
            current_account_id: None,
            accounts: vec![],
        });
    }
    serde_json::from_str(&content).map_err(|e| format!("Failed to parse index: {}", e))
}

fn save_index(state: &State, index: &AccountIndex) -> AppResult<()> {
    let content =
        serde_json::to_string_pretty(index).map_err(|e| format!("Failed to serialize index: {}", e))?;
    fs::write(&state.index_path, content).map_err(|e| format!("Failed to write index: {}", e))
}

fn load_account(state: &State, account_id: &str) -> Option<StoredAccount> {
    let path = state.accounts_dir.join(format!("{}.json", account_id));
    let content = fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

fn save_account(state: &State, account: &StoredAccount) -> AppResult<()> {
    let path = state.accounts_dir.join(format!("{}.json", account.id));
    let content =
        serde_json::to_string_pretty(account).map_err(|e| format!("Failed to serialize account: {}", e))?;
    fs::write(path, content).map_err(|e| format!("Failed to write account: {}", e))
}

fn upsert_summary(index: &mut AccountIndex, account: &StoredAccount) {
    if let Some(summary) = index.accounts.iter_mut().find(|x| x.id == account.id) {
        summary.name = account.name.clone();
        summary.email = account.email.clone();
        summary.last_used = account.last_used;
        return;
    }
    index.accounts.push(AccountSummary {
        id: account.id.clone(),
        name: account.name.clone(),
        email: account.email.clone(),
        created_at: account.created_at,
        last_used: account.last_used,
    });
}

fn resolve_codex_home() -> PathBuf {
    if let Ok(value) = std::env::var("CODEX_HOME") {
        let trimmed = value.trim().trim_matches('"').trim_matches('\'').trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }
    dirs::home_dir()
        .unwrap_or_else(|| Path::new("/").to_path_buf())
        .join(".codex")
}
