-- WorldAreas.Id of the area the character is in (or returns to).
ALTER TABLE characters ADD COLUMN area TEXT NOT NULL DEFAULT '';
