-- Set once the character has played its start area's intro (the Riverbank "washed up" opening).
ALTER TABLE characters ADD COLUMN intro_done INTEGER NOT NULL DEFAULT 0;
