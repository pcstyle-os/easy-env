# IDEA.md — easyenv

> The env var manager that lives in your shell.
> Global, project, encrypted, synced through iCloud Keychain.

---

## 1. The problem

Every developer ends up with the same mess:

- A pile of `.env` files, half of them out of date.
- Tokens pasted into `~/.zshrc`, never rotated.
- Secrets in 1Password, screenshots in Slack, plaintext in Notion.
- A new laptop = a half-day ritual of re-pasting keys.
- Sharing a `STRIPE_SECRET` with a teammate = a DM you both forget to delete.

Existing tools each solve a slice and miss the rest:

| Tool          | Local-first | Sync       | Team share  | Encrypted | UX       |
| ------------- | ----------- | ---------- | ----------- | --------- | -------- |
| `.env` files  | ✓           | ✗          | ✗           | ✗         | painful  |
| direnv        | ✓           | ✗          | ✗           | ✗         | shell hooks |
| dotenv-vault  | ✗ (SaaS)    | ✓ (server) | ✓           | ✓         | medium   |
| Doppler/Vault | ✗ (SaaS)    | ✓ (server) | ✓           | ✓         | enterprise |
| 1Password CLI | ✓           | ✓          | manual copy | ✓         | not built for envs |

`easyenv` aims at the gap: **local-first, zero-ceremony, encrypted, synced via the OS, with optional team push when you want it.**

---

## 2. The pitch

A single statically-linked binary you run in your terminal. Three commands cover 90% of usage:

```bash
$ easyenv set OPENAI_API_KEY=sk-***          # store, encrypted
$ easyenv exec -- pnpm dev                   # run with vars injected
$ easyenv push --to=@alex                    # share with a teammate
```

That’s the whole product surface for most days.

---

## 3. Core ideas

### 3.1 Three scopes, no surprises

| Scope     | Lives where               | Synced?           | Use for                   |
| --------- | ------------------------- | ----------------- | ------------------------- |
| `global`  | OS keychain               | iCloud Keychain   | personal tokens           |
| `project` | OS keychain, dir-bound    | optional (opt-in) | per-app secrets           |
| `shell`   | process memory only       | never             | one-off overrides         |

Resolution: `shell` > `project` > `global`. Inspect with `easyenv get KEY --explain`.

### 3.2 iCloud Keychain as the sync substrate

The killer feature, and the boring one. Apple already shipped:

- End-to-end encrypted multi-device sync.
- A trusted, audited keystore in the Secure Enclave.
- A UX everyone with an Apple ID has already opted into.

`easyenv` stores global secrets as keychain items in the access group `dev.easyenv.shared`, which makes them eligible for iCloud Keychain sync. We never run a server in the path.

> **Implication:** there is no "easyenv cloud." The product can ship with zero backend infra and still feel magical.

### 3.3 Local-first, no daemon

- One Rust binary. ~3 MB.
- No background process, no shell-rc soup unless you opt-in to `easyenv hook`.
- `exec` spawns a child process with the resolved environment — works with anything that reads `process.env`.

### 3.4 Encrypted by default

- AES-256-GCM at rest, per-item nonces.
- Wrapping key bound to the Secure Enclave (macOS) or kernel keyring (Linux); non-extractable.
- Plaintext only ever lives in process memory and is zeroed on drop.

### 3.5 Teams via public-key envelopes (optional)

When you do need to share:

- Each member has an Ed25519 identity key derived on their device.
- `easyenv push` encrypts each value to each recipient's public key.
- The relay server is dumb — it sees only ciphertext.
- Audit log is signed and append-only.

No "trust the vendor" story. No "rotate the master key" panic.

---

## 4. Surface area (CLI)

```
easyenv init           Register the cwd as a project.
easyenv set            Store a key/value, optionally with --global / --expires.
easyenv get            Read a value. Masked by default. --reveal / --copy / --explain.
easyenv list (ls)      List vars. Filter by scope, format as table/json/dotenv/shell.
easyenv exec (run)     Spawn a command with the resolved env injected.
easyenv push           Encrypt + share to teammates by handle.
easyenv pull           Restore from iCloud / team / workspace.
easyenv sync           Reconcile local with iCloud.
easyenv rotate         Rotate a value, optionally pushing the new one to the team.
easyenv profile        Manage named profiles (development/staging/prod).
easyenv workspace      Create/invite/revoke for team workspaces.
easyenv audit          Show the signed audit log.
easyenv import .env    Migrate a legacy file, optionally --delete + gitignore.
easyenv doctor         Health-check (paths, keychain, sync, plaintext leftovers).
easyenv hook           Emit a shell hook for cd-on-load.
easyenv completion     Generate zsh/bash/fish completions.
```

---

## 5. Why now

- Apple Passwords (2024) made iCloud Keychain a first-class developer surface, with a public access-group story that doesn’t require an MDM profile.
- Rust + minimum-deps tooling makes "single binary, no runtime" trivial again.
- The SaaS-vault category has trained engineers that secrets need encryption and audit — but also that paying $20/seat to manage a `.env` is absurd for hobbyists and small teams.
- AI agents and dev assistants are increasingly running shell commands; they need a deterministic, scriptable env-resolver, not a `.env` race condition.

---

## 6. Audience

| Persona            | Job to be done                                         | Today's pain                          |
| ------------------ | ------------------------------------------------------ | ------------------------------------- |
| Indie dev / hobbyist | Re-use my OpenAI/GitHub keys across 30 side projects | Copy-paste into every new repo        |
| Small startup team | Share a `STRIPE_SECRET` rotation across 4 engineers    | Slack DM, half the team forgets       |
| Multi-device user   | Pick up a project on a different Mac                  | "wait, I think I have it on the iMac" |
| AI/agent runner    | Inject env vars deterministically into spawned tools  | `.env` parsing race conditions        |
| Security-aware dev | Stop pasting secrets into shell history                | Knows it's wrong, does it anyway      |

---

## 7. Non-goals (deliberate)

- **Not a SaaS vault.** No "easyenv cloud" tier. Team relay is end-to-end encrypted and optional.
- **Not a deployment tool.** We don't push secrets to Vercel/AWS/Fly. There are good integrations for that — we're the source of truth on your machine.
- **Not Windows-native.** WSL is supported; native Windows is not the target this year.
- **Not a password manager.** No browser autofill, no TOTP. 1Password owns that space.
- **Not configurable to death.** Three scopes. Not five. Not user-defined.

---

## 8. Success metrics

- **Activation**: > 60% of installers run `easyenv set` within 24h.
- **Retention**: > 40% weekly active 8 weeks post-install.
- **Multi-device**: > 25% of macOS users sign in on a 2nd device within 30 days.
- **Team adoption**: median team size 3-6 within first 90 days of inviting.
- **Security incidents**: 0. Forever.

---

## 9. Open questions

- Linux-without-DBus environments (CI containers): kernel keyring works, but the UX of "headless first run" needs design.
- iCloud Keychain item size cap (~10 KB) means very large secrets (mTLS PEMs) need a side-channel. Worth shipping a v1 that just refuses?
- Profiles vs. branches: should a profile auto-track the current git branch? Tempting, but doubles the mental model.
- Pricing: free for solo, paid for teams? Or free forever and monetize a hosted relay + audit retention?

---

## 10. The tagline test

Five candidates, ranked by how quickly they make a developer nod:

1. **"env vars done right."** — the working tagline.
2. "The env var manager that lives in your shell."
3. "Stop pasting secrets in your terminal."
4. "Your secrets, on every machine you own. Encrypted by Apple."
5. ".env, but for grown-ups."

---

*This doc lives at `/idea.md` on the marketing site so contributors and prospective users can read the full vision in one scroll. The product itself ships as a single Rust binary; the website at `/` and full docs at `/docs` walk you through using it.*
