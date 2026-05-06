# easyenv

> env vars done right.

`easyenv` is a local-first env var manager for the shell. It stores secret values in the OS credential store, keeps non-secret metadata in SQLite, resolves `project > global` values, injects the final environment into child processes, and now supports encrypted share bundles for person-to-person secret handoff.

## Stack

The implementation is optimized for a **macOS-first MVP** and uses:

- **Rust** for the single-binary CLI
- **clap** for subcommands, help, aliases, and completions
- **rusqlite** for local metadata (`projects`, `vars`, migrations)
- **security-framework** for macOS Keychain storage
- **AES-256-GCM** (`aes-gcm`) for encrypted secret payloads before storage
- **zeroize** for best-effort secret memory cleanup
- **keyring-rs** as the non-macOS native credential fallback
- **age** for encrypted share bundles (`push` / `pull`)

## Current scope

Implemented now:

- `easyenv init`
- `easyenv set`
- `easyenv get`
- `easyenv list` / `easyenv ls`
- `easyenv exec` / `easyenv run`
- `easyenv walkthrough` / `easyenv walktrough`
- `easyenv import`
- `easyenv push`
- `easyenv pull`
- `easyenv workspace show`
- `easyenv workspace rotate-identity`
- `easyenv doctor`
- `easyenv completion`

Still deferred:

- hosted relay / audit retention
- full team workspace membership management
- branch-aware profiles
- native Windows support

## Storage model

- **global** secrets: stored in the native credential store under `dev.easyenv.global`
- **project** secrets: stored in the native credential store under `dev.easyenv.project`
- **master encryption key**: stored separately under `dev.easyenv.master`
- **workspace share identity**: stored under `dev.easyenv.identity`
- **metadata**: stored in SQLite at:
  - macOS: `~/Library/Application Support/easyenv/easyenv.sqlite`
  - Linux: XDG data dir equivalent
- **resolution order**: `shell override > project > global`

Project registration is local-first: `easyenv init` registers the canonical path in the metadata database, and subdirectories inherit that project automatically.

## macOS keychain behavior

On macOS, global secrets now use the lower-level `PasswordOptions` API so `easyenv` can prefer synchronizable keychain entries while still being usable as an unsigned CLI during development.

Environment flags:

- `EASYENV_KEYCHAIN_GLOBAL_SYNC=prefer|force|never`
  - `prefer` (default): try synchronizable storage first, then fall back to local-only storage
  - `force`: require synchronizable storage
  - `never`: keep global secrets local-only
- `EASYENV_KEYCHAIN_ACCESS_GROUP=...`
  - optional access group for signed/distributed builds that want stricter keychain placement

Important note: true iCloud-keychain-grade behavior for a signed distributed app still depends on Apple signing and entitlements. This repo now uses the right API surface, but it does not claim full entitlement wiring for an unsigned development binary.

## Share bundles

`easyenv` can now export and import encrypted **age x25519 armored bundles**.

Each machine gets a local workspace identity:

```bash
easyenv workspace show
```

That prints an `age1...` recipient. Another developer can encrypt selected secrets to that recipient:

```bash
easyenv push --to age1... --key OPENAI_API_KEY --out easyenv-share.age
```

The recipient can then import the bundle:

```bash
easyenv pull easyenv-share.age --project
```

This is intentionally local-first scaffolding:

- no relay server
- no hosted audit trail
- no team membership graph yet
- encrypted bundle exchange works today

## Examples

Initialize a project:

```bash
easyenv init
```

Store a global secret:

```bash
easyenv set OPENAI_API_KEY=sk-...
```

Store a project secret:

```bash
easyenv set --project STRIPE_SECRET=whsec_...
```

Read a value without fully revealing it:

```bash
easyenv get OPENAI_API_KEY
```

Explain where a resolved value came from:

```bash
easyenv get STRIPE_SECRET --explain
```

List resolved vars as JSON:

```bash
easyenv list --format json --reveal
```

Run a command with injected vars:

```bash
easyenv exec -- pnpm dev
```

Get a built-in guided tour of the product:

```bash
easyenv walkthrough
```

(`easyenv walktrough` also works as a forgiving alias.)

Import an existing `.env` file into the active project and delete the source file:

```bash
easyenv import .env --delete
```

Show your local share recipient:

```bash
easyenv workspace show --json
```

Export all visible vars to an encrypted share bundle:

```bash
easyenv push --all --to age1... --out easyenv-share.age
```

Import a share bundle into project scope:

```bash
easyenv pull easyenv-share.age --project
```

Generate completions:

```bash
easyenv completion zsh
```

## Installation

See [docs/install.md](docs/install.md).

Quick source install:

```bash
cargo install --path crates/easyenv-cli --locked
```

## Development

Build:

```bash
cargo build
```

Run tests:

```bash
cargo test
```

### Test backend

Integration tests use a file-backed secret store instead of the real keychain.

Relevant env vars:

- `EASYENV_HOME` — overrides the application data directory
- `EASYENV_SECRET_BACKEND=file` — uses a local file secret store for testing

## Repo layout

- `crates/easyenv-cli` — CLI binary
- `crates/easyenv-core` — domain model, paths, crypto, metadata, resolver
- `crates/easyenv-keychain` — native secret-store implementations
- `docs/install.md` — install instructions
- `.github/workflows` — CI and release packaging
