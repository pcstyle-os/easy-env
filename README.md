# easyenv

The env var manager that lives in your shell.

`easyenv` is a local-first CLI for storing, resolving, and injecting environment
variables. The primary production target is macOS, where secrets are stored in
Apple Keychain and can participate in iCloud Keychain sync. Linux and Windows
use their native credential stores through the same Rust keyring abstraction.

This repository currently implements a practical v1 foundation:

- `global`, `project`, and `shell` scopes with `shell > project > global`
  resolution.
- OS keychain-backed `set`, `get`, `list`, `exec`, `init`, `import`, `doctor`,
  `hook`, and `completion` commands.
- File-backed project metadata under `.easyenv/`.
- Local team-envelope placeholders for `push`, `pull`, `sync`, `rotate`,
  `profile`, `workspace`, and `audit` so the CLI surface from `idea.md` is
  discoverable while backend relay features remain explicit future work.

## Install and run

```bash
cargo run -- --help
cargo run -- set OPENAI_API_KEY=sk-example --global
cargo run -- get OPENAI_API_KEY --reveal
cargo run -- exec -- printenv OPENAI_API_KEY
```

For isolated local development or CI, set `EASYENV_STORE=file` to use an
encrypted-local-store-compatible development backend rooted at
`EASYENV_HOME` instead of the OS keychain:

```bash
EASYENV_STORE=file EASYENV_HOME=.easyenv-dev cargo run -- set FOO=bar --global
```

The file backend is for tests/headless development only. Production use should
prefer the native OS keychain backend.

## Testing stack

This is a Rust CLI, so the testing stack is:

- `cargo test` for unit tests.
- `assert_cmd` for black-box CLI integration tests.
- `predicates` for stdout/stderr assertions.
- `tempfile` for isolated `EASYENV_HOME` and project directories.

Run all tests:

```bash
cargo test
```

Run formatting and lint checks:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
```

## macOS notes

The default `EASYENV_STORE=keychain` backend uses the `keyring` crate with the
Apple native feature enabled. Keychain items are stored under the `easyenv`
service. The v1 implementation avoids shell startup hooks unless the user runs
`easyenv hook`.
