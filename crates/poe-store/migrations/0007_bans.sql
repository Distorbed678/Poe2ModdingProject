-- Bans (web admin, Players page). An account ban refuses the account's logins (game and web)
-- and ends its sessions; an IP ban refuses connections from one address. A ban is a row: it
-- is active until `expires_at` (null = permanent) or until it is lifted (`lifted_at`); lifted
-- and expired rows stay as the history. Times are unix seconds.
CREATE TABLE bans (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    kind        TEXT NOT NULL CHECK (kind IN ('account', 'ip')),
    account_id  INTEGER REFERENCES accounts (id) ON DELETE CASCADE,
    -- Canonical text of an IPv4 or IPv6 address (IPv4-mapped IPv6 is stored as IPv4).
    ip          TEXT,
    reason      TEXT NOT NULL DEFAULT '',
    created_by  TEXT NOT NULL,
    created_at  INTEGER NOT NULL,
    expires_at  INTEGER,
    lifted_at   INTEGER,
    lifted_by   TEXT,
    CHECK ((kind = 'account' AND account_id IS NOT NULL AND ip IS NULL)
        OR (kind = 'ip' AND ip IS NOT NULL AND account_id IS NULL))
);
CREATE INDEX bans_account ON bans (account_id) WHERE lifted_at IS NULL;
CREATE INDEX bans_ip ON bans (ip) WHERE lifted_at IS NULL;
