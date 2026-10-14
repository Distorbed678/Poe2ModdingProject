-- Where the character left the game: terrain seed of its area's layout and grid position.
-- NULL until it has been in the game (then it starts at the area's start position).
ALTER TABLE characters ADD COLUMN seed INTEGER;
ALTER TABLE characters ADD COLUMN pos_x INTEGER;
ALTER TABLE characters ADD COLUMN pos_y INTEGER;
