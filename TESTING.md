# Testing report

## Chosen stack

`easyenv` is a Rust command-line application, so the test stack is intentionally
Rust-native and black-box friendly:

- `cargo test` runs unit and integration tests.
- `assert_cmd` launches the compiled `easyenv` binary in integration tests.
- `predicates` asserts command output without brittle full-string matching.
- `tempfile` creates isolated config homes and project directories.

The integration tests set `EASYENV_STORE=file` so CI and developer machines do
not need to write to the host OS keychain.

## Commands

```bash
cargo test
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
```

For manual CLI testing without touching the OS keychain:

```bash
EASYENV_STORE=file EASYENV_HOME=.easyenv-dev cargo run -- set FOO=bar --global
EASYENV_STORE=file EASYENV_HOME=.easyenv-dev cargo run -- get FOO --reveal
```

## Context7 note

The requested Context7 MCP lookup was attempted, but the server returned
`Monthly quota exceeded`. I continued with repo analysis plus public docs for
`clap`, `keyring`, and `assert_cmd`.
