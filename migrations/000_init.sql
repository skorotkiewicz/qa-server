-- Users table with API key authentication
CREATE TABLE IF NOT EXISTS users (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    username    TEXT UNIQUE NOT NULL,
    api_key     TEXT UNIQUE NOT NULL,
    created_at  TEXT DEFAULT CURRENT_TIMESTAMP
);

-- Questions table
CREATE TABLE IF NOT EXISTS questions (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id     INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    title       TEXT NOT NULL,
    content     TEXT NOT NULL,
    created_at  TEXT DEFAULT CURRENT_TIMESTAMP,
    solved      INTEGER DEFAULT 0,
    solved_at   TEXT
);

-- Answers table
CREATE TABLE IF NOT EXISTS answers (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    question_id INTEGER NOT NULL REFERENCES questions(id) ON DELETE CASCADE,
    user_id     INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    content     TEXT NOT NULL,
    created_at  TEXT DEFAULT CURRENT_TIMESTAMP
);

-- Stars table (many-to-many: users <-> questions)
CREATE TABLE IF NOT EXISTS stars (
    user_id     INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    question_id INTEGER NOT NULL REFERENCES questions(id) ON DELETE CASCADE,
    created_at  TEXT DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (user_id, question_id)
);
