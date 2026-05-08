-- Performance indexes for high throughput

-- Index for fetching unsolved questions (sorted by created_at)
CREATE INDEX IF NOT EXISTS idx_questions_solved_created ON questions(solved, created_at DESC);

-- Index for fetching user's questions
CREATE INDEX IF NOT EXISTS idx_questions_user_id ON questions(user_id);

-- Index for fetching answers by question
CREATE INDEX IF NOT EXISTS idx_answers_question_id ON answers(question_id);

-- Index for fetching answers by user
CREATE INDEX IF NOT EXISTS idx_answers_user_id ON answers(user_id);

-- Index for fetching starred questions by user
CREATE INDEX IF NOT EXISTS idx_stars_user_id ON stars(user_id);

-- Index for counting stars on a question
CREATE INDEX IF NOT EXISTS idx_stars_question_id ON stars(question_id);

-- Index for API key lookups (faster authentication)
CREATE INDEX IF NOT EXISTS idx_users_api_key ON users(api_key);
