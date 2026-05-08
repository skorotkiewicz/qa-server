# QA-RS

A high-performance Q&A system with CLI client and HTTP API server.

## Quick Start

**Docker**:
```bash
# Run pre-built image
docker pull skorotkiewicz/qa-server:latest # pull latest image
docker run -d -p 3000:7879 -v qa_server_data:/data skorotkiewicz/qa-server:latest

# Or build locally
docker build -t qa_server .
docker run -d -p 3000:7879 -v qa_server_data:/data qa-server

# Or use docker-compose
docker-compose up -d
```

**From source**:
```bash
cargo install --path .
```

### 1. Start the Server

```bash
cp server.yml.example server.yml
./target/debug/qa-server
```

The server will start on `0.0.0.0:7879` by default.

### 2. Use the CLI

```bash
# Create an account (API key will be saved to ~/.config/qa/config.yml)
./qa create-account

# Ask a question (reads from stdin, first line is title, rest is content)
cat question.md | ./qa ask

# List unsolved questions (paginated)
./qa unsolved
./qa unsolved --page 1 --limit 50

# List questions you have starred (paginated)
./qa starred
./qa starred --page 0 --limit 20

# Get a question with all its answers
./qa get 12

# Answer a question (reads from stdin)
cat answer.md | ./qa answer 12

# Mark a question as solved or unsolved (only the asker can do this)
./qa solve 12
./qa solve 12 --false

# Star/unstar a question
./qa star 12
./qa unstar 12

# Remove your own question (with all its answers)
./qa rm-question 12

# Remove your own answer
./qa rm-answer 12 5  # question_id=12, answer_id=5

# Change your API key
./qa change-api-key
```

## CLI Commands

| Command | Description |
|---------|-------------|
| `qa create-account` | Create a new account and save API key |
| `qa change-api-key` | Change your API key (requires current key) |
| `cat question.md \| qa ask` | Ask a question (title on first line) |
| `qa unsolved [--page N] [--limit N]` | List unsolved questions (paginated) |
| `qa starred [--page N] [--limit N]` | List questions you have starred (paginated) |
| `qa get <id>` | Get a question with all answers |
| `cat answer.md \| qa answer <id>` | Answer a question |
| `qa solve <id> [--false]` | Mark question as solved/unsolved (asker only) |
| `qa star <id>` | Star a question |
| `qa unstar <id>` | Unstar a question |
| `qa rm-question <id>` | Delete your own question (with answers) |
| `qa rm-answer <qid> <aid>` | Delete your own answer |

<details>
  <summary>API Endpoints</summary>

| Method | Endpoint | Description |
|--------|----------|-------------|
| POST | `/register` | Create account (returns API key in `x-api-key` header) |
| POST | `/change-api-key` | Change API key |
| POST | `/questions` | Ask a question |
| GET | `/questions/unsolved?page=N&limit=N` | List unsolved questions (paginated) |
| GET | `/questions/starred?page=N&limit=N` | List starred questions (paginated) |
| GET | `/questions/{id}` | Get question with answers |
| DELETE | `/questions/{id}` | Delete own question |
| POST | `/questions/{id}/answers` | Answer a question |
| DELETE | `/questions/{id}/answers/{answer_id}` | Delete own answer |
| POST | `/questions/{id}/solved?unsolved=true/false` | Mark as solved/unsolved |
| POST | `/questions/{id}/star` | Star a question |
| DELETE | `/questions/{id}/star` | Unstar a question |
| GET | `/health` | Health check |

</details>

## Configuration

### Server (`server.yml`)

```yaml
bind: "0.0.0.0:7879"
database_url: "sqlite://qa.db"
db_pool_size: 10
# Optional: Redis URL for caching (e.g., "redis://127.0.0.1:6379")
# redis_url: "redis://127.0.0.1:6379"
```

**Features:**
- Rate limiting: 30 req/s per API key
- **Pagination**: `page` and `limit` parameters (default 20, max 100)
- **Redis caching**: GET endpoints cached with TTL (60s for lists, 300s for questions)
- **Performance optimizations**: SQLite WAL mode, connection pooling, database indexes
- **Response headers**: `X-Cache: HIT` or `X-Cache: MISS`

### Environment Variables

Override config file settings with environment variables:

```bash
export QA_SERVER_IP=127.0.0.1        # Server IP (default: 0.0.0.0)
export QA_SERVER_PORT=8080           # Server port (default: 7879)
export DATABASE_URL=sqlite://qa.db   # Database URL
export REDIS_URL=redis://127.0.0.1:6379  # Redis URL for caching
```

### Client (`~/.config/qa/config.yml`)

Created automatically after `create-account`:

```yaml
endpoint: "http://localhost:7879"
api_key: "your-api-key-here"
username: "your-username"
```

## Database Schema

- **users**: id, username, api_key, created_at
- **questions**: id, user_id, title, content, created_at, solved, solved_at, views
- **answers**: id, question_id, user_id, content, created_at
- **stars**: user_id, question_id, created_at

## Building

```bash
cargo build --release
```

Binaries will be at:
- `./target/release/qa-server`
- `./target/release/qa`

## License

MIT
