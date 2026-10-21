-- Real accounts (WP3.2). The machine-hash accounts of 0001 stay as they are: they get a null
-- password (no password login possible) and keep logging in through the dev-only
-- `[accounts] dev_machine_hash_login`. New accounts are created with a username and an argon2id
-- hash; their `login_key` is a unique placeholder ("pw:<random hex>") that no machine hash can
-- equal, because the column is NOT NULL UNIQUE from 0001.
--
-- `password_hash`: argon2id (PHC string) over the client's SHA-256(UTF-16LE password) digest.
-- Times are unix seconds.
ALTER TABLE accounts ADD COLUMN password_hash TEXT;
ALTER TABLE accounts ADD COLUMN role TEXT NOT NULL DEFAULT 'player' CHECK (role IN ('player', 'admin'));
ALTER TABLE accounts ADD COLUMN last_login INTEGER;
ALTER TABLE accounts ADD COLUMN disabled INTEGER NOT NULL DEFAULT 0;

-- Usernames are unique case-insensitively (0001 made `name` unique case-sensitively).
CREATE UNIQUE INDEX accounts_name_nocase ON accounts (name COLLATE NOCASE);

-- Admins used to be listed in `[admin] accounts` (the dev config named the first auto-created
-- account). That list is gone: carry the dev admin over so existing databases keep working.
-- Only an upgrade of an old development database does anything here: on a fresh database no
-- account exists yet when this runs, so nobody is promoted. (A player who happens to be
-- named Exile1 on a database that was created before this migration would become admin: keep
-- `dev_machine_hash_login` off on any server others can reach. Only comments change here: an
-- applied migration's statements are never edited.)
UPDATE accounts SET role = 'admin' WHERE name = 'Exile1';

-- Revocable tokens. Only the SHA-256 of the 32 random token bytes is stored.
-- kind: game_remember (the client's Remember Me file), launcher, web.
CREATE TABLE account_tokens (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id  INTEGER NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    kind        TEXT NOT NULL CHECK (kind IN ('game_remember', 'launcher', 'web')),
    token_hash  BLOB NOT NULL UNIQUE,
    -- `[cluster] id` of the cluster that issued it; another cluster's tokens never validate.
    cluster_id  TEXT NOT NULL,
    created_at  INTEGER NOT NULL,
    expires_at  INTEGER NOT NULL,
    revoked     INTEGER NOT NULL DEFAULT 0,
    last_used   INTEGER
);
CREATE INDEX account_tokens_account ON account_tokens (account_id);
