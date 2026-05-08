# QA-RS

A simple Q&A system with CLI client and HTTP API server.

## Quick Start

### 1. Start the Server

```bash
cp server.yml.example server.yml
./target/debug/qa-server
```

The server will start on `0.0.0.0:7878` by default.

### 2. Use the CLI

```bash
# Create an account (API key will be saved to ~/.config/qa/config.yml)
./qa create-account

# Ask a question (reads from stdin, first line is title, rest is content)
cat question.md | ./qa ask

# List unsolved questions
./qa unsolved

# Get a question with all its answers
./qa get 12

# Answer a question (reads from stdin)
cat answer.md | ./qa answer 12

# Mark a question as solved (only the asker can do this)
./qa solved 12

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
| `qa unsolved` | List unsolved questions |
| `qa get <id>` | Get a question with all answers |
| `cat answer.md \| qa answer <id>` | Answer a question |
| `qa solved <id>` | Mark question as solved (asker only) |
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
| GET | `/questions/unsolved` | List unsolved questions |
| GET | `/questions/{id}` | Get question with answers |
| DELETE | `/questions/{id}` | Delete own question |
| POST | `/questions/{id}/answers` | Answer a question |
| DELETE | `/questions/{id}/answers/{answer_id}` | Delete own answer |
| POST | `/questions/{id}/solved` | Mark as solved |
| POST | `/questions/{id}/star` | Star a question |
| DELETE | `/questions/{id}/star` | Unstar a question |
| GET | `/health` | Health check |

</details>

## Configuration

### Server (`server.yml`)

```yaml
bind: "0.0.0.0:7878"
database_url: "sqlite://qa.db"
# Optional: Redis URL for response caching (GET endpoints)
# redis_url: "redis://127.0.0.1:6379"
```

**Features:**
- Rate limiting: 30 req/s per API key (configurable via governor)
- **Redis caching**: GET `/questions/unsolved` (60s TTL) and GET `/questions/{id}` (300s TTL)
  - Responses include `X-Cache: HIT` or `X-Cache: MISS` header

### Client (`~/.config/qa/config.yml`)

Created automatically after `create-account`:

```yaml
endpoint: "http://localhost:7878"
api_key: "your-api-key-here"
username: "your-username"
```

## Database Schema

- **users**: id, username, api_key, created_at
- **questions**: id, user_id, title, content, created_at, solved, solved_at
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
