# Install easyenv

## Option 1: Download a release archive

From GitHub Releases:

- `easyenv-linux-x86_64.tar.gz`
- `easyenv-macos-x86_64.tar.gz`
- `easyenv-macos-aarch64.tar.gz`

Example:

```bash
curl -fsSL https://github.com/pcstyle/easy-env/releases/latest/download/easyenv-macos-aarch64.tar.gz -o easyenv.tar.gz
tar -xzf easyenv.tar.gz
sudo install -m 0755 easyenv-macos-aarch64/easyenv /usr/local/bin/easyenv
```

## Option 2: Use the install script

```bash
curl -fsSL https://raw.githubusercontent.com/pcstyle/easy-env/main/scripts/install.sh | bash
```

Install a specific version:

```bash
curl -fsSL https://raw.githubusercontent.com/pcstyle/easy-env/main/scripts/install.sh | bash -s -- v0.1.0
```

## Option 3: Build from source

Requires Rust stable.

```bash
git clone https://github.com/pcstyle/easy-env.git
cd easy-env
cargo install --path crates/easyenv-cli --locked
```

## Verify

```bash
easyenv --version
easyenv doctor
```

## Shell completions

### zsh

```bash
easyenv completion zsh > ~/.zsh/completions/_easyenv
```

### bash

```bash
easyenv completion bash > ~/.local/share/bash-completion/completions/easyenv
```

### fish

```bash
easyenv completion fish > ~/.config/fish/completions/easyenv.fish
```

## macOS keychain notes

By default, macOS global secrets use:

```bash
EASYENV_KEYCHAIN_GLOBAL_SYNC=prefer
```

That means `easyenv` will try synchronizable keychain storage first and fall back to local-only storage if the current unsigned binary cannot use the synchronizable path.

Useful overrides:

```bash
export EASYENV_KEYCHAIN_GLOBAL_SYNC=force
export EASYENV_KEYCHAIN_ACCESS_GROUP=dev.easyenv.shared
```

Use `force` only when you know the binary is signed/configured correctly for the keychain environment you want.
