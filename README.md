# codexm

`codexm` is a CLI tool for managing multiple Codex accounts.

It keeps account auth isolated while preserving a near-default Codex experience via shared capabilities from `~/.codex`.

---

## ✨ Features

- 🔐 Multi-account management: add, list, switch
- 💰 Quota visibility: check account quota/remaining balance across accounts
- 🚀 Start Codex with a selected account in current directory
- 🧩 Shared capabilities with default workspace (history/config/skills/cache)
- 🛡️ Account isolation to avoid auth conflicts
- ⌨️ Interactive picker with silent cancel on `Esc` / `Ctrl+C`
- 📦 Tiny binary size with high performance

---

## 📋 Requirements

- `codex` command available in `PATH`

---

## 📥 Install

### macOS / Linux (curl)

```bash
curl -fsSL <INSTALL_SCRIPT_URL> | sh
```

### Windows (PowerShell)

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\install.ps1 -Repo <owner/repo> -Version latest
```

### Uninstall

```bash
rm -f ~/.local/bin/codexm
```

```powershell
Remove-Item "$HOME\AppData\Local\bin\codexm.exe" -Force
```

---

## ⚡ Quick Start

### 1) Add an account

```bash
codexm add
```

This runs official `codex login`, then imports tokens into local account storage.

### 2) List accounts and quota

```bash
codexm ls
# alias:
codexm list
```

### 3) Switch default account

```bash
codexm switch
codexm switch your@email.com
```

Force refresh token before switching:

```bash
codexm switch --force-refresh your@email.com
```

### 4) Start Codex with selected account (current directory)

```bash
codexm
# equivalent to:
codexm new

# with specific account:
codexm new your@email.com
```

### 5) Delete an account

```bash
codexm delete your@email.com
# alias:
codexm rm your@email.com
```

Without an argument, an account picker is shown:

```bash
codexm delete
```

After selecting an account, you must confirm with `ok` before deletion is executed.

---

## 🧭 Command Reference

- `codexm`  
  Open interactive account picker and start Codex in current directory (same as `codexm new`)

- `codexm add`  
  Add/import account via OAuth login flow

- `codexm ls` / `codexm list`  
  Show all accounts and quota status

- `codexm switch [name|email|id]`  
  Switch default account for `~/.codex`

- `codexm switch --force-refresh [name|email|id]`  
  Force token refresh before switching

- `codexm new [email]`  
  Start Codex in current directory with selected account

- `codexm delete [email]` / `codexm rm [email]`  
  Delete account from local store and remove its local workspace (with confirmation)

---

## 📁 Data & Behavior

- Account store: `~/.codex-manager`
- Default workspace: `~/.codex`
- Shared capability layer: history/config/skills/cache from default workspace
- Isolated auth layer: account credentials stay separated to prevent conflicts
- On Windows, if symlink permission is unavailable, codexm falls back to copy mode and prints a warning

---

## 🛠️ Local Build

If you want to build from source locally:

```bash
cargo build --release
./target/release/codexm --help
```

Or install globally from local source:

```bash
cargo install --path . --force
codexm --help
```

---

## ❓ FAQ

### Why does `ls` show Plus but Codex App still looks outdated?

Run:

```bash
codexm switch --force-refresh your@email.com
```

Then fully restart Codex App.

### Why do I see a Windows warning about copy fallback?

Cause:

- Windows symlink creation can be blocked by permission policy (no Developer Mode, no admin privilege, or restricted environment).

Impact:

- codexm can still run, but shared capability paths use copied content for that run.
- Changes written in those copied paths will not sync back to global `~/.codex`.

How to fix:

1. Enable **Developer Mode** in Windows settings, or run terminal as **Administrator**.
2. Start `codexm` again (it will retry symlink and switch back automatically when allowed).

### How do I exit the interactive account picker?

- `Esc` → silent cancel
- `Ctrl+C` → silent cancel

### What does `delete` remove?

- Account record in local store
- Local workspace for that account under `~/.codex-manager/instances/<account_id>`
- Related keychain entry (on macOS)

