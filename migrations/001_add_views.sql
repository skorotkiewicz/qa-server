-- Add views counter to questions
ALTER TABLE questions ADD COLUMN views INTEGER DEFAULT 0;
