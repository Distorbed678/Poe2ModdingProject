# Accounts and identity

Current state and the plan. How a remote player gets to the server: [../guides/hosting.md](../guides/hosting.md).

## Current state

- **Account = machine hash.** The login server keys accounts by `auth.machine_hash` of the client's Authenticate message (`login-server/src/session.rs:247`, layout in [../protocol/login.md](../protocol/login.md)). The hash is stable per machine and comes from the client, so two people on one PC share an account and anyone can claim anyone's account. Account names are `Exile<id>`, not chosen.
- **Storage:** accounts, characters and per-account character lists are in SQLite (`poe-store`, `Store` trait). Character names are unique across accounts (case-insensitive). The slot limit comes from config.
- **Tickets and sessions** (`poe-cluster`): tickets with random keys per hand-off; resume tickets (single use) for "exit to character selection"; login sessions per account (logout, transfers). The pieces work for many accounts at once.
- **Admins:** `[admin] accounts = ["Exile1"]` in `config/server.toml`, tied to the auto-generated names, which change if the database is reset ([../guides/server-admin.md](../guides/server-admin.md)).
- **Account settings:** `[accounts] flags`, `full_access`, `stash_tabs` are global for every account.
- **Same account twice:** `poe-cluster` `MemorySessions`: a second login replaces the first's transfer channel; nothing kicks the old session.
- **Authentication limits:** the Authenticate message carries the Steam encrypted app ticket. GGG's key encrypts it, so the server cannot verify it; at best it identifies the account and does not authenticate it.

## Planned

| What | Plan |
|---|---|
| Real login | The DLL (or a small launcher, see [../guides/launcher.md](../guides/launcher.md)) supplies a username and password or token. Options: (a) check whether this exe still has the standalone email/password login path (Ghidra, open question); (b) the DLL puts a token from `poe-hook.toml` into the auth packet's unused bytes; (c) the Steam ticket (identifies only). |
| Admins | Name accounts at registration. Admin list by name or by an `admin` column in the database. |
| Duplicate login | Official behaviour: the new login kicks the old one. Do the same. |
| Account settings | Per-account columns instead of the global flags (stash tabs come with the stash). |
| Web administration | [web-api.md](web-api.md). |
