-- Add env, tags, is_favorite to connections
ALTER TABLE connections ADD COLUMN env TEXT;
ALTER TABLE connections ADD COLUMN tags TEXT;
ALTER TABLE connections ADD COLUMN is_favorite INTEGER NOT NULL DEFAULT 0;
