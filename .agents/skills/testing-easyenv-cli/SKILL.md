---
name: testing-easyenv-cli
description: Test the easyenv Rust CLI end-to-end using a local file-backed secret store. Use when verifying CLI commands, project/global resolution, env injection, or encrypted share bundle flows.
---

# easyenv CLI testing

## Devin Secrets Needed

None for local CLI testing. Use `EASYENV_SECRET_BACKEND=file` with an isolated `EASYENV_HOME` so tests do not touch the OS keychain.

## Setup

From the repo root, install Linux native credential build dependencies if they are not already present:

```bash
sudo apt-get update
sudo apt-get install -y pkg-config libdbus-1-dev
```

Build/check the Rust workspace:

```bash
cargo check --workspace --locked
```

## Standard verification commands

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
EASYENV_SECRET_BACKEND=file cargo test --workspace --locked
```

## End-to-end local CLI flow

Use two temporary homes/projects to simulate sender and receiver machines without touching native keychains:

```bash
ROOT="/home/ubuntu/easyenv-manual-test-$(date +%s)"
mkdir -p "$ROOT/sender-project" "$ROOT/receiver-project" "$ROOT/sender-home" "$ROOT/receiver-home"

sender() { EASYENV_SECRET_BACKEND=file EASYENV_HOME="$ROOT/sender-home" cargo run --quiet -p easyenv -- "$@"; }
receiver() { EASYENV_SECRET_BACKEND=file EASYENV_HOME="$ROOT/receiver-home" cargo run --quiet -p easyenv -- "$@"; }

sender init "$ROOT/sender-project"
receiver init "$ROOT/receiver-project"
(cd "$ROOT/sender-project" && sender set --project OPENAI_API_KEY=shared-secret)
(cd "$ROOT/sender-project" && sender get OPENAI_API_KEY --reveal)
(cd "$ROOT/sender-project" && sender list --format json --reveal)
(cd "$ROOT/sender-project" && sender exec -- printenv OPENAI_API_KEY)
recipient="$(receiver workspace show --json | python3 -c 'import json,sys; print(json.load(sys.stdin)["recipient"])')"
(cd "$ROOT/sender-project" && sender push --to "$recipient" --key OPENAI_API_KEY --out "$ROOT/share.age")
(cd "$ROOT/receiver-project" && receiver pull "$ROOT/share.age" --project)
(cd "$ROOT/receiver-project" && receiver get OPENAI_API_KEY --reveal)
```

Expected assertions:

- `init` prints `initialized project` for both projects.
- Sender `get` prints `OPENAI_API_KEY=shared-secret`.
- `list --format json --reveal` contains `"key":"OPENAI_API_KEY"`, `"scope":"project"`, and `"value":"shared-secret"`.
- `exec -- printenv OPENAI_API_KEY` prints `shared-secret`.
- `workspace show --json` returns a recipient starting with `age1`.
- `push` prints `wrote encrypted share bundle` and writes a nonempty bundle.
- Receiver `pull` prints `imported 1 shared secret`.
- Receiver `get` prints `OPENAI_API_KEY=shared-secret`.

## Notes

- This app is a CLI. If all testing is shell-only, do not record the desktop; attach the command transcript and test report instead.
- Native macOS Keychain behavior requires a macOS environment. On Linux, use the file backend for deterministic local testing.
