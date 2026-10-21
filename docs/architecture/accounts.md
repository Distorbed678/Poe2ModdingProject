# Accounts and identity

How a player logs in, how accounts and tokens are stored, and how admins are chosen. How a remote player reaches the server: [../guides/hosting.md](../guides/hosting.md). Wire format: [../protocol/login.md](../protocol/login.md). The client side: [../re/standalone-login.md](../re/standalone-login.md).

The code is the `poe-accounts` crate (`AccountStore` trait, `SqliteAccounts` implementation over the `poe-store` database, migration `0006_accounts_auth.sql`). The login server decides in `servers/login-server/src/auth.rs`.

## Model

| | |
|---|---|
| Account | `accounts` table: `id`, `name` (the username; unique case-insensitively), `password_hash`, `role`, `created_at`, `last_login`, `disabled`. Characters belong to the account id. |
| Role | `player` or `admin`. Admin commands check the role at the time of the command; there is no config list any more. |
| Disabled | A disabled account cannot log in (result `0x47`), its tokens are revoked and its running sessions are ended. |
| Token | `account_tokens` table: `kind` (`game_remember`, `launcher`, `web`), `token_hash`, `cluster_id`, `created_at`, `expires_at`, `revoked`, `last_used`. |
| Cluster | `[cluster] id` in `config/server.toml`. Every token records the id of the cluster that issued it. |

Usernames are 3-64 characters from `A-Za-z0-9_.@+-` (so an email address works as the account name on the client's login screen). Server commands require passwords of at least 8 characters.

## Passwords: what is hashed

The client never sends the plain password. Its login screen sends `SHA-256(UTF-16LE(password))` (32 bytes, no salt; [login.md](../protocol/login.md#cs-0x0003-credentials-standalone-platform)). The stored secret is **argon2id over that digest** (PHC string with a random salt per hash; default cost 19 MiB, 2 passes).

Everything that sets or checks a password must produce the same digest first:

```
digest = SHA-256(password encoded as UTF-16LE, no BOM, no terminator)
stored = argon2id(digest)            // poe_accounts::AccountStore takes the digest, never the text
```

`poe_accounts::password_digest(&str)` does the first step. The server commands (`account create`, `account passwd`) and the web API (registration, login, password change, launcher login: `crates/poe-web-api`) go through it, or an account made on the web would not accept the same password in game. The web API receives the plain password in the request body (use TLS outside the local machine) and hashes it with the same function; it never needs the stored hash.

Consequence of the client's design: the digest is a password equivalent for this server (whoever has it can log in), but it is stored only inside an argon2id hash.

## Tokens

- 32 random bytes (never all zero). The server keeps only `SHA-256(token)`; the raw bytes exist once, in the `0x0004` reply (`session_token`) or the launcher/web response.
- `game_remember`: issued when a password login has `save_login = 1`. The client saves it in its credentials file and sends it back as `password_hash` with `token_login = 1`. **Rolling:** each token login revokes the token it used and returns a fresh one, so the file always holds a valid token and a stolen old copy dies. Lifetime `[accounts] remember_days` (30).
- A token is valid when its hash is known, its `cluster_id` equals `[cluster] id`, it is not revoked, not expired, of an accepted kind, and the account is not disabled. On a token login the client's saved user name must also match the token's account.
- A new password (`account passwd`) and disabling an account revoke all the account's tokens.
- **Cluster id:** change `[cluster] id` to invalidate every saved login at once; a database copied between clusters keeps its accounts and passwords, but no token from the other cluster works.
- Launcher and web tokens use the same table and trait (`TokenKind::Launcher`, `TokenKind::Web`), issued by `POST /api/v1/login` ([web-api.md](web-api.md)): web 12 hours, launcher `remember_days`. The token login of the game accepts `game_remember` and `launcher`, so a launcher-issued token could be passed for auto-login; the launcher itself uses `POST /api/v1/game-token`, which issues a separate one-off `game_remember` token because the game's rolling login would revoke the launcher token. The admin API accepts `web` only.

Result codes the client gets for failures: [login.md](../protocol/login.md#authentication-results).

## Login paths

| Path | Packet | When |
|---|---|---|
| Username and password | `0x0003`, `token_login = 0` | the standalone login screen (`login_mode = "standalone"` in the DLL config, [client-dll.md](client-dll.md#login-mode-and-trusted-servers)) |
| Saved token | `0x0003`, `token_login = 1` | Remember Me, launcher auto-login |
| Machine hash (dev) | `0x0005` (Steam) | only with `[accounts] dev_machine_hash_login = true`; auto-creates `Exile<id>` accounts without a password. Default in `config/server.toml` for now, off in code |

With `dev_machine_hash_login = false` a `0x0005` gets result `9` ("Access to the game is not currently enabled") with a notice.

Accounts that existed before migration `0006` (machine-hash accounts) are kept with a null password: they can only log in on the dev path, and `account passwd <name> <password>` gives one a real password. The migration made `Exile1` an admin, as the old `[admin] accounts` list of the dev config did.

## Admins and account commands

Admin is the `admin` role. Commands (chat `!account ...`, the overlay's Commands page, or offline `cargo run -p poe-server -- account ...`): `create`, `role`, `disable`, `enable`, `passwd`, `list`; see [../guides/server-admin.md](../guides/server-admin.md). The first admin is made offline: `account create <name> <password>`, then `account role <name> admin`. Passwords typed in chat are replaced by `***` in the log and the audit file. An admin cannot demote or disable themselves.

`[accounts] open_registration` (default false) lets `POST /api/v1/register` of the web API create player accounts ([web-api.md](web-api.md)); server commands always work.

## One session per account

A new login of an account ends the old one: `LoginSessions::supersede(account)` (`poe-cluster`) closes the account's login connections and signals its instance sessions, which disconnect. It runs after a successful login, before the new connection subscribes, so the new session is not affected. The same call ends the sessions of a disabled account. A resumed session (`0x0008`, "Exit to character selection") does not supersede.

## Other state

- **Storage:** accounts, characters and per-account character lists are in SQLite (`poe-store`). Character names are unique across accounts (case-insensitive). The slot limit comes from config.
- **Tickets and sessions** (`poe-cluster`): tickets with random keys per hand-off; resume tickets (single use); login sessions per account (logout, supersede, transfers).
- **Account settings:** `[accounts] flags`, `full_access`, `stash_tabs` are still global for every account (per-account columns come with the stash).
- **Authentication limits of the Steam path:** the `0x0005` message carries the Steam encrypted app ticket. GGG's key encrypts it, so the server cannot verify it; it only identifies the machine.

## Not done yet

- Rate limiting of password attempts (the client has a "too many attempts" code, `0x49`).
- Rate limiting of the web login (`/api/v1/login` shares the missing limit).
- Per-account settings.
- Whether `0x0f`/`0x62` behave as inferred in the client (a rejected token has not been tried in game; the password login and remember-me token login work in game): [../re/standalone-login.md](../re/standalone-login.md) section 5.
