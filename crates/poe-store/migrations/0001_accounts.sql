CREATE TABLE accounts (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    -- Stable client identity (hex machine hash from the Authenticate message).
    login_key   TEXT NOT NULL UNIQUE,
    name        TEXT NOT NULL UNIQUE,
    created_at  INTEGER NOT NULL DEFAULT (unixepoch())
);
