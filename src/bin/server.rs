use std::{fs, net::SocketAddr};

use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use clap::Parser;
use rand::distributions::Alphanumeric;
use rand::Rng;
use serde::{Deserialize, Serialize};
use serde_yaml;
use sqlx::FromRow;

// -- Config

#[derive(Debug, Deserialize)]
struct ServerConfig {
    bind: Option<String>,
    database_url: String,
}

fn load_config(path: &str) -> anyhow::Result<ServerConfig> {
    let text = fs::read_to_string(path)?;
    Ok(serde_yaml::from_str(&text)?)
}

// -- Database

type DbPool = sqlx::SqlitePool;

// -- Models

#[derive(Serialize, FromRow)]
struct User {
    id: i64,
    username: String,
    #[serde(skip_serializing)]
    #[allow(dead_code)]
    api_key: String,
    created_at: DateTime<Utc>,
}

#[derive(Serialize, FromRow)]
struct Question {
    id: i64,
    user_id: i64,
    #[sqlx(rename = "username")]
    author: String,
    title: String,
    content: String,
    created_at: DateTime<Utc>,
    solved: bool,
    solved_at: Option<DateTime<Utc>>,
    #[sqlx(default)]
    starred: bool,
}

#[derive(Serialize, FromRow)]
struct Answer {
    id: i64,
    question_id: i64,
    user_id: i64,
    #[sqlx(rename = "username")]
    author: String,
    content: String,
    created_at: DateTime<Utc>,
}

// -- API Request/Response models

#[derive(Deserialize)]
struct CreateUserRequest {
    username: String,
}

#[derive(Deserialize)]
struct ChangeApiKeyRequest {
    current_api_key: String,
}

#[derive(Deserialize)]
struct CreateQuestionRequest {
    title: String,
    content: String,
}

#[derive(Deserialize)]
struct CreateAnswerRequest {
    content: String,
}

#[derive(Serialize)]
struct CreateUserResponse {
    id: i64,
    username: String,
    api_key: String,
}

#[derive(Serialize)]
struct ApiKeyResponse {
    api_key: String,
}

// -- Helpers

fn generate_api_key() -> String {
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(32)
        .map(char::from)
        .collect()
}

async fn get_user_by_api_key(pool: &DbPool, api_key: &str) -> anyhow::Result<Option<User>> {
    let user = sqlx::query_as::<_, User>(
        "SELECT id, username, api_key, created_at FROM users WHERE api_key = $1"
    )
    .bind(api_key)
    .fetch_optional(pool)
    .await?;
    Ok(user)
}

fn get_api_key_from_headers(headers: &HeaderMap) -> Option<&str> {
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .or_else(|| {
            headers
                .get("x-api-key")
                .and_then(|v| v.to_str().ok())
        })
}

macro_rules! require_auth {
    ($pool:expr, $headers:expr) => {
        match get_api_key_from_headers(&$headers) {
            Some(api_key) => {
                match get_user_by_api_key($pool, api_key).await {
                    Ok(Some(user)) => user,
                    Ok(None) => {
                        return (StatusCode::UNAUTHORIZED, "invalid api key").into_response();
                    }
                    Err(e) => {
                        return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
                    }
                }
            }
            None => {
                return (StatusCode::UNAUTHORIZED, "missing api key").into_response();
            }
        }
    };
}

// -- Handlers

/// POST /register - Create new account
async fn create_account(
    State(pool): State<DbPool>,
    Json(req): Json<CreateUserRequest>,
) -> Response {
    let api_key = generate_api_key();
    
    let result = sqlx::query("INSERT INTO users (username, api_key) VALUES ($1, $2)")
        .bind(&req.username)
        .bind(&api_key)
        .execute(&pool)
        .await;
    
    match result {
        Ok(res) => {
            let user_id = res.last_insert_rowid();
            let resp = CreateUserResponse {
                id: user_id,
                username: req.username,
                api_key: api_key.clone(),
            };
            (
                StatusCode::CREATED,
                [("x-api-key", api_key.as_str())],
                Json(resp),
            )
                .into_response()
        }
        Err(e) => {
            if e.to_string().contains("UNIQUE") {
                (StatusCode::CONFLICT, "username already exists").into_response()
            } else {
                (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
            }
        }
    }
}

/// POST /change-api-key - Change API key (requires current)
async fn change_api_key(
    State(pool): State<DbPool>,
    headers: HeaderMap,
    Json(req): Json<ChangeApiKeyRequest>,
) -> Response {
    let current_key = match get_api_key_from_headers(&headers) {
        Some(key) => key,
        None => {
            return (StatusCode::UNAUTHORIZED, "missing api key in authorization header").into_response();
        }
    };
    
    if current_key != req.current_api_key {
        return (StatusCode::UNAUTHORIZED, "invalid current api key").into_response();
    }
    
    let new_api_key = generate_api_key();
    
    let result = sqlx::query("UPDATE users SET api_key = $1 WHERE api_key = $2")
        .bind(&new_api_key)
        .bind(&req.current_api_key)
        .execute(&pool)
        .await;
    
    match result {
        Ok(_) => {
            let resp = ApiKeyResponse { api_key: new_api_key.clone() };
            (
                StatusCode::OK,
                [("x-api-key", new_api_key.as_str())],
                Json(resp),
            )
                .into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// POST /questions - Ask a question
async fn create_question(
    State(pool): State<DbPool>,
    headers: HeaderMap,
    Json(req): Json<CreateQuestionRequest>,
) -> Response {
    let user = require_auth!(&pool, headers);
    
    let result = sqlx::query(
        "INSERT INTO questions (user_id, title, content) VALUES ($1, $2, $3)"
    )
    .bind(user.id)
    .bind(&req.title)
    .bind(&req.content)
    .execute(&pool)
    .await;
    
    match result {
        Ok(res) => {
            let question_id = res.last_insert_rowid();
            (StatusCode::CREATED, format!("{}", question_id)).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// GET /questions/unsolved - List unsolved questions
async fn list_unsolved(State(pool): State<DbPool>, headers: HeaderMap) -> Response {
    let user = require_auth!(&pool, headers);
    
    let questions = sqlx::query_as::<_, Question>(
        r#"
        SELECT 
            q.id, q.user_id, u.username, q.title, q.content, 
            q.created_at, q.solved, q.solved_at,
            EXISTS(SELECT 1 FROM stars s WHERE s.user_id = $1 AND s.question_id = q.id) as starred
        FROM questions q
        JOIN users u ON q.user_id = u.id
        WHERE q.solved = FALSE
        ORDER BY q.created_at DESC
        "#
    )
    .bind(user.id)
    .fetch_all(&pool)
    .await;
    
    match questions {
        Ok(rows) => Json(rows).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// POST /questions/:id/answers - Answer a question
async fn create_answer(
    State(pool): State<DbPool>,
    headers: HeaderMap,
    Path(question_id): Path<i64>,
    Json(req): Json<CreateAnswerRequest>,
) -> Response {
    let user = require_auth!(&pool, headers);
    
    // Check if question exists
    let question_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM questions WHERE id = $1)"
    )
    .bind(question_id)
    .fetch_one(&pool)
    .await
    .unwrap_or(false);
    
    if !question_exists {
        return (StatusCode::NOT_FOUND, "question not found").into_response();
    }
    
    let result = sqlx::query(
        "INSERT INTO answers (question_id, user_id, content) VALUES ($1, $2, $3)"
    )
    .bind(question_id)
    .bind(user.id)
    .bind(&req.content)
    .execute(&pool)
    .await;
    
    match result {
        Ok(res) => {
            let answer_id = res.last_insert_rowid();
            (StatusCode::CREATED, format!("{}", answer_id)).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// POST /questions/:id/solved - Mark question as solved (only asker)
async fn mark_solved(
    State(pool): State<DbPool>,
    headers: HeaderMap,
    Path(question_id): Path<i64>,
) -> Response {
    let user = require_auth!(&pool, headers);
    
    // Get question and verify ownership
    let question: Option<(i64,)> = sqlx::query_as(
        "SELECT user_id FROM questions WHERE id = $1"
    )
    .bind(question_id)
    .fetch_optional(&pool)
    .await
    .ok()
    .flatten();
    
    match question {
        Some((owner_id,)) => {
            if owner_id != user.id {
                return (StatusCode::FORBIDDEN, "only the asker can mark as solved").into_response();
            }
        }
        None => {
            return (StatusCode::NOT_FOUND, "question not found").into_response();
        }
    }
    
    let result = sqlx::query(
        "UPDATE questions SET solved = TRUE, solved_at = CURRENT_TIMESTAMP WHERE id = $1"
    )
    .bind(question_id)
    .execute(&pool)
    .await;
    
    match result {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// POST /questions/:id/star - Star a question
async fn star_question(
    State(pool): State<DbPool>,
    headers: HeaderMap,
    Path(question_id): Path<i64>,
) -> Response {
    let user = require_auth!(&pool, headers);
    
    let result = sqlx::query(
        "INSERT INTO stars (user_id, question_id) VALUES ($1, $2) ON CONFLICT DO NOTHING"
    )
    .bind(user.id)
    .bind(question_id)
    .execute(&pool)
    .await;
    
    match result {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            if e.to_string().contains("FOREIGN KEY") {
                (StatusCode::NOT_FOUND, "question not found").into_response()
            } else {
                (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
            }
        }
    }
}

/// DELETE /questions/:id/star - Unstar a question
async fn unstar_question(
    State(pool): State<DbPool>,
    headers: HeaderMap,
    Path(question_id): Path<i64>,
) -> Response {
    let user = require_auth!(&pool, headers);
    
    let result = sqlx::query(
        "DELETE FROM stars WHERE user_id = $1 AND question_id = $2"
    )
    .bind(user.id)
    .bind(question_id)
    .execute(&pool)
    .await;
    
    match result {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// GET /questions/:id - Get question with answers
async fn get_question(
    State(pool): State<DbPool>,
    headers: HeaderMap,
    Path(question_id): Path<i64>,
) -> Response {
    let _user = require_auth!(&pool, headers);
    
    let question = sqlx::query_as::<_, Question>(
        r#"
        SELECT 
            q.id, q.user_id, u.username, q.title, q.content, 
            q.created_at, q.solved, q.solved_at, 0 as starred
        FROM questions q
        JOIN users u ON q.user_id = u.id
        WHERE q.id = $1
        "#
    )
    .bind(question_id)
    .fetch_optional(&pool)
    .await;
    
    match question {
        Ok(Some(q)) => {
            let answers = sqlx::query_as::<_, Answer>(
                r#"
                SELECT 
                    a.id, a.question_id, a.user_id, u.username, a.content, a.created_at
                FROM answers a
                JOIN users u ON a.user_id = u.id
                WHERE a.question_id = $1
                ORDER BY a.created_at ASC
                "#
            )
            .bind(question_id)
            .fetch_all(&pool)
            .await;
            
            match answers {
                Ok(ans) => {
                    #[derive(Serialize)]
                    struct QuestionWithAnswers {
                        #[serde(flatten)]
                        question: Question,
                        answers: Vec<Answer>,
                    }
                    
                    let resp = QuestionWithAnswers {
                        question: q,
                        answers: ans,
                    };
                    Json(resp).into_response()
                }
                Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
            }
        }
        Ok(None) => (StatusCode::NOT_FOUND, "question not found").into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// GET /health - Health check
async fn health() -> &'static str {
    "ok"
}

// -- Main

#[derive(Parser)]
#[command(name = "qa-server", about = "Q&A server", version)]
struct Args {
    /// Path to config file
    #[arg(default_value = "server.yml")]
    config: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let cfg = load_config(&args.config)?;

    let bind: SocketAddr = cfg.bind.as_deref().unwrap_or("0.0.0.0:7878").parse()?;

    // Ensure database file exists for SQLite
    let db_path = cfg.database_url
        .strip_prefix("sqlite://")
        .or_else(|| cfg.database_url.strip_prefix("sqlite:"))
        .unwrap_or(&cfg.database_url);
    
    if !std::path::Path::new(db_path).exists() {
        std::fs::File::create(db_path)?;
    }
    
    let pool = sqlx::SqlitePool::connect(&cfg.database_url).await?;
    sqlx::migrate!("./migrations/sqlite").run(&pool).await?;
    
    println!("✓ database connected ({db_path})");

    let app = Router::new()
        .route("/health", get(health))
        .route("/register", post(create_account))
        .route("/change-api-key", post(change_api_key))
        .route("/questions", post(create_question))
        .route("/questions/unsolved", get(list_unsolved))
        .route("/questions/{id}", get(get_question))
        .route("/questions/{id}/answers", post(create_answer))
        .route("/questions/{id}/solved", post(mark_solved))
        .route("/questions/{id}/star", post(star_question).delete(unstar_question))
        .with_state(pool);

    println!("✓ qa-server listening on {bind}");
    let listener = tokio::net::TcpListener::bind(bind).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
