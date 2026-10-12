CREATE TABLE characters (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id   INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    name         TEXT NOT NULL UNIQUE COLLATE NOCASE,
    league       TEXT NOT NULL,
    -- Characters.IntegerId of the class.
    class_id     INTEGER NOT NULL,
    -- CharacterStartStates.Id chosen at creation (e.g. "DexFourDefault").
    start_state  TEXT NOT NULL,
    level        INTEGER NOT NULL DEFAULT 1,
    experience   INTEGER NOT NULL DEFAULT 0,
    created_at   INTEGER NOT NULL DEFAULT (unixepoch())
);
CREATE INDEX characters_account ON characters(account_id);
